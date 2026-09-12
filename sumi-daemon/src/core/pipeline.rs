//! 索引流水线的应用层：文件系统事实（路径 + size + mtime）→ 内存索引 + 权威存储 + SSE 事件。
//! 单写者：全部入口由消费循环/扫描 runner 串行调用（apply_* 需要 &mut ItemIndex）。
//! id 漂移的三分支迁移规则（移动/复制/去重优先）见 API 文档「Item 对象」节。

use crate::core::events::names;
use crate::core::item::{ItemCore, PathRecord};
use crate::core::metadata_store::MetadataStore;
use crate::core::paths::LibraryPaths;
use crate::core::index::ItemIndex;
use std::sync::Arc;

pub struct PipelineCtx<'a> {
    pub paths: &'a LibraryPaths,
    pub index: &'a mut ItemIndex,
    pub store: &'a MetadataStore,
    pub bus: &'a crate::core::events::EventBus,
    pub fulltext: Option<&'a crate::core::fulltext::FulltextIndex>,
}

/// 计算文件 BLAKE3 哈希（hex）
pub fn hash_file(abs_path: &str) -> std::io::Result<String> {
    let mut file = std::fs::File::open(abs_path)?;
    let mut hasher = blake3::Hasher::new();
    std::io::copy(&mut file, &mut hasher)?;
    Ok(hasher.finalize().to_hex().to_string())
}

/// 文件事实应用：新文件 / 内容变更 / 既有位置刷新（mtime 回写）。
/// 返回 Some(item_id)（用于事件与任务派发）。路径不可读时返回 None（调用方记录）
pub fn apply_file_fact(
    ctx: &mut PipelineCtx,
    rel: &str,
    size: u64,
    mtime: i64,
) -> Option<String> {
    let abs = ctx.paths.to_absolute(rel)?;
    let new_hash = match hash_file(&abs) {
        Ok(h) => h,
        Err(e) => {
            tracing::warn!("哈希计算失败 {rel}: {e}");
            return None;
        }
    };

    let old_owner = ctx.index.owner_of(rel).map(|s| s.to_string());
    let now = crate::core::paths::unix_ms(std::time::SystemTime::now());

    match old_owner {
        Some(old_id) if old_id == new_hash => {
            // 内容未变：刷新校验依据（size/mtime），保持新鲜
            let item = ctx.index.get(&old_id)?.clone();
            if refresh_path_record(ctx, &item, rel, size, mtime) {
                emit_item_updated(ctx, &old_id);
            }
            Some(old_id)
        }
        Some(old_id) => {
            // 内容变更：id 漂移迁移（三分支）
            migrate_id_drift(ctx, &old_id, &new_hash, rel, size, mtime, now)
        }
        None => {
            // 新文件
            if let Some(existing) = ctx.index.get(&new_hash).cloned() {
                // 同内容去重：位置并入既有条目
                let mut item = existing.clone();
                item.paths.push(PathRecord::new(rel, size, mtime));
                persist_and_upsert(ctx, item);
                emit_item_updated(ctx, &new_hash);
                Some(new_hash)
            } else {
                let mut item = ItemCore::new(&new_hash, vec![PathRecord::new(rel, size, mtime)], now);
                // 解析回填：一次打开文件产出书目元数据 + 封面 + 全文（契约：扫描导入即时提取）
                derive_book_facts(ctx, &mut item, rel, &abs);
                persist_and_upsert(ctx, item);
                emit_item_added(ctx, &new_hash);
                Some(new_hash)
            }
        }
    }
}

/// id 漂移迁移：旧 hash H（old_id）→ 新 hash X（new_hash），变更位置 rel
fn migrate_id_drift(
    ctx: &mut PipelineCtx,
    old_id: &str,
    new_hash: &str,
    rel: &str,
    size: u64,
    mtime: i64,
    now: i64,
) -> Option<String> {
    let old_item = ctx.index.get(old_id)?.clone();
    let others: Vec<PathRecord> = old_item
        .paths
        .iter()
        .filter(|p| p.path != rel)
        .cloned()
        .collect();

    if ctx.index.get(new_hash).is_some() {
        // 分支 1：X 已有元数据 → 去重语义优先，位置直接并入 X；H 剩余位置决定存续
        let mut x = ctx.index.get(new_hash).unwrap().clone();
        if !x.paths.iter().any(|p| p.path == rel) {
            x.paths.push(PathRecord::new(rel, size, mtime));
        }
        persist_and_upsert(ctx, x);
        emit_item_updated(ctx, new_hash);

        let mut h = old_item.clone();
        h.paths = others.clone();
        if h.paths.is_empty() {
            // H 无剩余位置：彻底删除（含自定义封面清理）
            drop_item(ctx, old_id);
        } else {
            persist_and_upsert(ctx, h);
            emit_item_updated(ctx, old_id);
        }
        Some(new_hash.to_string())
    } else if others.is_empty() {
        // 分支 2：整体移动（TOML rename / db 改键；自定义封面文件一并改名）
        let mut moved = old_item.clone();
        moved.id = new_hash.to_string();
        moved.paths = vec![PathRecord::new(rel, size, mtime)];
        moved.added_time = if moved.added_time == 0 { now } else { moved.added_time };
        migrate_custom_cover(ctx, old_id, new_hash);
        migrate_derived(ctx, old_id, new_hash);
        ctx.store.delete(old_id).ok();
        persist_and_upsert(ctx, moved);
        ctx.bus.emit(names::ITEM_REMOVED, serde_json::json!({ "id": old_id }));
        emit_item_added(ctx, new_hash);
        Some(new_hash.to_string())
    } else {
        // 分支 3：复制（H 保留原元数据，X 继承用户数据）
        let mut copied = old_item.clone();
        copied.id = new_hash.to_string();
        copied.paths = vec![PathRecord::new(rel, size, mtime)];
        migrate_derived(ctx, old_id, new_hash);
        persist_and_upsert(ctx, copied);
        emit_item_added(ctx, new_hash);
        // H 的该位置记录移除（位置改属 X）
        let mut h = old_item;
        h.paths = others;
        persist_and_upsert(ctx, h);
        emit_item_updated(ctx, old_id);
        Some(new_hash.to_string())
    }
}

/// 文件消失（监听事件/对账）：位置从索引移除；无剩余位置时彻底删除条目
pub fn apply_path_removed(ctx: &mut PipelineCtx, rel: &str) {
    let Some(owner) = ctx.index.owner_of(rel).map(|s| s.to_string()) else {
        return;
    };
    let Some(mut item) = ctx.index.get(&owner).cloned() else {
        return;
    };
    let in_trash = LibraryPaths::is_in_trash(rel);
    item.paths.retain(|p| p.path != rel);
    if item.paths.is_empty() {
        drop_item(ctx, &owner);
        if !in_trash {
            ctx.bus.emit(names::ITEM_REMOVED, serde_json::json!({ "id": owner }));
        }
    } else {
        persist_and_upsert(ctx, item);
        emit_item_updated(ctx, &owner);
    }
}

/// 移入回收站（保留元数据；folder/item delete 用，路径迁移在文件系统层完成后调用）
pub fn apply_trash_move(ctx: &mut PipelineCtx, rel: &str, trash_rel: &str) -> Option<String> {
    let owner = ctx.index.owner_of(rel).map(|s| s.to_string())?;
    let mut item = ctx.index.get(&owner)?.clone();
    let record = item.paths.iter().find(|p| p.path == rel)?.clone();
    item.paths.retain(|p| p.path != rel);
    item.paths.push(PathRecord::new(trash_rel, record.size, record.modification_time));
    let had_no_library_path = !item.has_library_path();
    persist_and_upsert(ctx, item);
    if had_no_library_path {
        ctx.bus.emit(names::ITEM_TRASHED, serde_json::json!({ "id": owner }));
    }
    Some(owner)
}

/// 从回收站恢复
pub fn apply_restore(ctx: &mut PipelineCtx, id: &str, trash_rel: &str) -> Option<()> {
    let mut item = ctx.index.get(id)?.clone();
    let record = item.paths.iter().find(|p| p.path == trash_rel)?.clone();
    item.paths.retain(|p| p.path != trash_rel);
    let original = LibraryPaths::trash_to_library_path(trash_rel).to_string();
    item.paths.push(PathRecord::new(original, record.size, record.modification_time));
    let has_library = item.has_library_path();
    persist_and_upsert(ctx, item);
    if has_library {
        emit_item_restored(ctx, id);
    }
    Some(())
}

/// 彻底删除条目：索引 + 权威存储 + 自定义封面 + 派生缓存
pub fn drop_item(ctx: &mut PipelineCtx, id: &str) {
    ctx.index.remove(id);
    ctx.store.delete(id).ok();
    let _ = std::fs::remove_file(format!("{}/{id}.webp", ctx.paths.covers_dir));
    let _ = std::fs::remove_file(format!("{}/{id}.webp", ctx.paths.cache_covers_dir));
    let _ = std::fs::remove_file(format!("{}/{id}.html", ctx.paths.cache_content_dir));
    if let Some(fts) = ctx.fulltext {
        let _ = fts.remove(id);
    }
}

fn refresh_path_record(ctx: &mut PipelineCtx, item: &ItemCore, rel: &str, size: u64, mtime: i64) -> bool {
    let mut updated = item.clone();
    let mut changed = false;
    for p in &mut updated.paths {
        if p.path == rel && (p.size != size || p.modification_time != mtime) {
            p.size = size;
            p.modification_time = mtime;
            changed = true;
        }
    }
    if changed {
        persist_and_upsert(ctx, updated);
    }
    changed
}

/// 解析派生：书目元数据回填（尊重用户编辑）+ 封面提取/生成落缓存 + 尺寸回填 + FTS 写入。
/// v1 为同步内联（正确性优先）；worker 化（CPU/4 封顶 8）在性能打磨阶段引入
fn derive_book_facts(ctx: &mut PipelineCtx, item: &mut ItemCore, rel: &str, abs: &str) {
    let name = LibraryPaths::name_of(rel).to_string();
    let ext = LibraryPaths::ext_of(rel);
    let bytes = match std::fs::read(abs) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("读取失败 {rel}: {e}");
            item.title = name;
            return;
        }
    };
    let parsed = crate::core::parser::parse(&name, &ext, &bytes);
    crate::core::parser::apply_to_item(item, &parsed);
    if item.title.trim().is_empty() {
        item.title = name;
    }

    // 封面：提取链（解码→缩放→webp）或排版生成（txt/md/docx）
    let cover_path = crate::core::cover::cache_cover_path(&ctx.paths.cache_covers_dir, &item.id);
    if !std::path::Path::new(&cover_path).exists() {
        let generated: Option<(Vec<u8>, u32, u32)> = parsed
            .cover
            .as_deref()
            .and_then(crate::core::cover::process_cover)
            .or_else(|| {
                if crate::core::cover::can_generate_cover(&ext) {
                    let (bytes, w, h) =
                        crate::core::cover::generated_cover(&item.title, &item.authors);
                    Some((bytes, w, h))
                } else {
                    None
                }
            });
        if let Some((bytes, w, h)) = generated {
            if !bytes.is_empty() {
                if let Err(e) = crate::core::config::atomic_write(&cover_path, &bytes) {
                    tracing::warn!("封面缓存写入失败 {rel}: {e}");
                } else {
                    item.cover_width = w;
                    item.cover_height = h;
                }
            }
        }
    }

    // 全文索引（txt/md/epub/docx）
    if let (Some(text), Some(fts)) = (&parsed.fulltext, ctx.fulltext) {
        if let Err(e) = fts.upsert(&item.id, text) {
            tracing::warn!("全文索引写入失败 {rel}: {e}");
        }
    }
}

/// 派生缓存迁移：自动封面 + 归一化内容 + FTS（id 漂移随迁）
fn migrate_derived(ctx: &PipelineCtx, old_id: &str, new_id: &str) {
    let from = crate::core::cover::cache_cover_path(&ctx.paths.cache_covers_dir, old_id);
    let to = crate::core::cover::cache_cover_path(&ctx.paths.cache_covers_dir, new_id);
    if std::path::Path::new(&from).exists() {
        let _ = std::fs::rename(&from, &to);
    }
    let from_content = format!("{}/{old_id}.html", ctx.paths.cache_content_dir);
    let to_content = format!("{}/{new_id}.html", ctx.paths.cache_content_dir);
    if std::path::Path::new(&from_content).exists() {
        let _ = std::fs::rename(&from_content, &to_content);
    }
    if let Some(fts) = ctx.fulltext {
        let _ = fts.migrate(old_id, new_id);
    }
}

fn migrate_custom_cover(ctx: &PipelineCtx, old_id: &str, new_id: &str) {
    let from = format!("{}/{old_id}.webp", ctx.paths.covers_dir);
    let to = format!("{}/{new_id}.webp", ctx.paths.covers_dir);
    if std::path::Path::new(&from).exists() {
        let _ = std::fs::rename(&from, &to);
    }
}

fn persist_and_upsert(ctx: &mut PipelineCtx, item: ItemCore) {
    if let Err(e) = ctx.store.upsert(&item) {
        tracing::error!("元数据写入失败 {}: {e}", item.id);
    }
    ctx.index.upsert(item);
}

fn emit_item_added(ctx: &PipelineCtx, id: &str) {
    if let Some(item) = ctx.index.get(id) {
        ctx.bus.emit_json(names::ITEM_ADDED, &item_dto(item));
    }
}

fn emit_item_updated(ctx: &PipelineCtx, id: &str) {
    if let Some(item) = ctx.index.get(id) {
        ctx.bus.emit_json(names::ITEM_UPDATED, &item_dto(item));
    }
}

fn emit_item_restored(ctx: &PipelineCtx, id: &str) {
    if let Some(item) = ctx.index.get(id) {
        ctx.bus.emit_json(names::ITEM_RESTORED, &item_dto(item));
    }
}

/// 流水线内部的轻量 DTO（完整 DTO 投影在 api 层；SSE 负载契约与 item/list 同构）
fn item_dto(item: &ItemCore) -> serde_json::Value {
    let primary = item.primary_path().to_string();
    serde_json::json!({
        "id": item.id,
        "name": LibraryPaths::name_of(&primary),
        "ext": LibraryPaths::ext_of(&primary),
        "size": item.paths.iter().find(|p| p.path == primary).map(|p| p.size).unwrap_or(0),
        "title": item.title,
        "authors": item.authors,
        "publisher": item.publisher,
        "pubdate": item.pubdate,
        "isbn": item.isbn,
        "language": item.language,
        "series": item.series,
        "series_index": item.series_index,
        "description": item.description,
        "url": item.url,
        "tags": item.tags,
        "categories": item.categories,
        "paths": item.paths.iter().map(|p| p.path.clone()).collect::<Vec<_>>(),
        "star": item.star,
        "read_status": item.read_status,
        "progress": item.progress,
        "progress_loc": item.progress_loc,
        "last_read_time": item.last_read_time,
        "annotation": item.annotation,
        "added_time": item.added_time,
    })
}

/// Arc 便利包装：bootstrap/测试里持 Arc<Field> 装配 ctx
pub struct SharedCore {
    pub index: std::sync::Mutex<ItemIndex>,
}

impl SharedCore {
    pub fn new() -> Arc<SharedCore> {
        Arc::new(SharedCore {
            index: std::sync::Mutex::new(ItemIndex::new()),
        })
    }
}
