//! 文件监听（notify）：实时感知书库变更（新增/删除/重命名/修改），防抖 300ms 后经流水线应用。
//! `.sumi/` 自身不参与索引，但 config.toml 与注册表文件的变更被监听（reload 热生效并广播事件）。
//! 静默丢事件的最终一致由周期兜底扫描/启动扫描保证（见 storage.md「实时文件监听」）。

use crate::core::config::LibraryConfig;
use crate::core::index::ItemIndex;
use crate::core::metadata_store::MetadataStore;
use crate::core::paths::LibraryPaths;
use crate::core::pipeline::{apply_file_fact, apply_path_removed, PipelineCtx};
use crate::core::registry_file::{GlobalFilter, NameRegistry, ViewPreferences};
use crate::core::locks::Locks;
use notify::{Event as NotifyEvent, Watcher};
use notify::EventKind::{Create, Modify, Remove};
use std::sync::mpsc as std_mpsc;
use std::sync::Arc;
use std::time::Duration;

/// 组件包：watcher 线程需要触达的全部共享态（bootstrap 装配）
pub struct WatchDeps {
    pub paths: LibraryPaths,
    pub config: Arc<LibraryConfig>,
    pub index: Arc<std::sync::Mutex<ItemIndex>>,
    pub store: Arc<MetadataStore>,
    pub fulltext: Option<Arc<crate::core::fulltext::FulltextIndex>>,
    pub bus: crate::core::events::EventBus,
    pub categories: Arc<NameRegistry>,
    pub tags: Arc<NameRegistry>,
    pub prefs: Arc<ViewPreferences>,
    pub global_filter: Arc<GlobalFilter>,
    pub locks: Arc<Locks>,
}

/// 启动文件监听线程（书库根 + 递归）。返回后监听在后台运行，随进程退出回收
pub fn spawn_watcher(deps: WatchDeps) {
    let WatchDeps {
        paths,
        config,
        index,
        store,
        fulltext,
        bus,
        categories,
        tags,
        prefs,
        global_filter,
        locks,
    } = deps;

    let (tx, rx) = std_mpsc::channel::<NotifyEvent>();
    let mut watcher = match notify::recommended_watcher(move |res: Result<NotifyEvent, notify::Error>| {
        if let Ok(event) = res {
            let _ = tx.send(event);
        }
    }) {
        Ok(w) => w,
        Err(e) => {
            tracing::error!("文件监听初始化失败（依赖周期重扫兜底）: {e}");
            return;
        }
    };
    if let Err(e) = watcher.watch(
        std::path::Path::new(&paths.root),
        notify::RecursiveMode::Recursive,
    ) {
        tracing::error!("文件监听启动失败（依赖周期重扫兜底）: {e}");
        return;
    }
    // watcher 移入线程保活
    std::thread::spawn(move || {
        let _watcher = watcher;
        let mut pending: Vec<NotifyEvent> = Vec::new();
        loop {
            let event = match rx.recv() {
                Ok(e) => e,
                Err(_) => return,
            };
            pending.push(event);
            // 防抖窗口：300ms 内聚合
            while let Ok(more) = rx.recv_timeout(Duration::from_millis(300)) {
                pending.push(more);
                if pending.len() > 4096 {
                    break;
                }
            }
            let batch = std::mem::take(&mut pending);
            handle_batch(&paths, &config, &index, &store, fulltext.as_deref(), &bus, &categories, &tags, &prefs, &global_filter, &locks, &batch);
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn handle_batch(
    paths: &LibraryPaths,
    config: &Arc<LibraryConfig>,
    index: &Arc<std::sync::Mutex<ItemIndex>>,
    store: &Arc<MetadataStore>,
    fulltext: Option<&crate::core::fulltext::FulltextIndex>,
    bus: &crate::core::events::EventBus,
    categories: &Arc<NameRegistry>,
    tags: &Arc<NameRegistry>,
    prefs: &Arc<ViewPreferences>,
    global_filter: &Arc<GlobalFilter>,
    locks: &Arc<Locks>,
    batch: &[NotifyEvent],
) {
    let snapshot = config.current();
    let exts = snapshot.extension_set();

    let mut folder_changed = false;

    for event in batch {
        for abs in &event.paths {
            let abs_str = abs.to_string_lossy().into_owned();
            let Some(rel) = paths.to_relative(&abs_str) else {
                continue;
            };
            if rel.is_empty() {
                continue;
            }

            // .sumi 内部：注册表/配置文件变更重载热生效；
            // 回收站位置（is_internal 对 trash 前缀返回 false）按普通文件处理，
            // 与扫描器的 trash 整体枚举口径一致（回收站位置参与索引）
            if LibraryPaths::is_internal(&rel) {
                match rel.as_str() {
                    p if p.ends_with("config.toml") => config.reload(),
                    p if p.ends_with("categories.toml") => categories.reload(),
                    p if p.ends_with("tags.toml") => tags.reload(),
                    p if p.ends_with("view.toml") => prefs.reload(),
                    p if p.ends_with("global_filter.toml") => {
                        global_filter.reload();
                        bus.emit_json(crate::core::events::names::GLOBAL_FILTER_CHANGED, &global_filter.snapshot());
                    }
                    p if p.ends_with("locks.toml") => {
                        locks.reload();
                        bus.emit_json(crate::core::events::names::LOCKS_CHANGED, &locks.snapshot_names());
                    }
                    _ => {}
                }
                continue;
            }

            let ext = LibraryPaths::ext_of(&rel);
            let is_file_like = !ext.is_empty();
            let _ = &rel;
            match event.kind {
                Create(_) | Modify(_) if is_file_like => {
                    // 回收站位置仅白名单适用；库内位置还要求非隐藏、非 ignore（与扫描器同口径）
                    let skip = if LibraryPaths::is_in_trash(&rel) {
                        false
                    } else {
                        LibraryPaths::is_hidden(&rel) || snapshot.matches_ignore(&rel)
                    };
                    if skip || !exts.contains(&ext) {
                        continue;
                    }
                    if let Some(abs_ok) = paths.to_absolute(&rel) {
                        if let Ok(meta) = std::fs::metadata(&abs_ok) {
                            if meta.is_file() {
                                let mtime = crate::core::paths::file_mtime_ms(&abs_ok);
                                let mut index = index.lock().unwrap();
                                let mut ctx = PipelineCtx { paths, index: &mut index, store, bus, fulltext };
                                apply_file_fact(&mut ctx, &rel, meta.len(), mtime);
                            }
                        }
                    }
                }
                Remove(_) => {
                    // 文件或目录消失：目录消失发 folder.changed（客户端重拉树）；
                    // 文件消失直接摘位置（后续周期扫描兜底漏事件）
                    if is_file_like {
                        let mut index = index.lock().unwrap();
                        let mut ctx = PipelineCtx { paths, index: &mut index, store, bus, fulltext };
                        apply_path_removed(&mut ctx, &rel);
                    } else {
                        folder_changed = true;
                    }
                }
                _ => {
                    // rename 等其他事件：目录结构可能变化
                    if !is_file_like {
                        folder_changed = true;
                    }
                }
            }
            if !is_file_like {
                folder_changed = true;
            }
        }
    }

    if folder_changed {
        bus.emit(
            crate::core::events::names::FOLDER_CHANGED,
            serde_json::json!({ "reason": "external" }),
        );
    }
}
