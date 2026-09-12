//! 扫描 runner：遍历书库目录得到文件清单（只读目录不读内容）→ 与内存索引比对 →
//! 差异经哈希并行确认后由流水线应用。startup 四阶段（sync/scan/hash/apply）与运行期
//! 周期兜底重扫共用本实现；[scan] 配置控制周期。

use crate::core::config::LibraryConfigSnapshot;
use crate::core::index::ItemIndex;
use crate::core::item::PathRecord;
use crate::core::metadata_store::MetadataStore;
use crate::core::paths::{LibraryPaths, unix_ms};
use crate::core::pipeline::{apply_file_fact, apply_path_removed, PipelineCtx};
use crate::core::startup::{Phase, StartupState};
use crate::core::tasks::{ScanStats, TaskTracker};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

/// 目录枚举结果：rel → (size, mtime)
pub type FileFacts = HashMap<String, (u64, i64)>;

/// 遍历书库目录（含回收站；跳过 .sumi 内部与 ignore/隐藏/白名单外的文件）
pub fn scan_library(paths: &LibraryPaths, config: &LibraryConfigSnapshot) -> std::io::Result<FileFacts> {
    let exts = config.extension_set();
    let mut facts = FileFacts::new();
    let mut stack = vec![paths.root.clone()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!("目录不可读 {dir}: {e}");
                continue;
            }
        };
        for entry in entries.flatten() {
            let Ok(meta) = entry.metadata() else { continue };
            let abs = entry.path().to_string_lossy().into_owned();
            if meta.is_dir() {
                // .sumi/ 整体跳过（回收站在其下单独枚举）
                if abs.ends_with(crate::core::paths::SUMI_DIR_NAME) {
                    continue;
                }
                stack.push(abs);
            } else if meta.is_file() {
                let Some(rel) = paths.to_relative(&abs) else { continue };
                if LibraryPaths::is_internal(&rel) {
                    // .sumi 内部只有 trash/ 参与索引
                    if !LibraryPaths::is_in_trash(&rel) {
                        continue;
                    }
                } else if LibraryPaths::is_hidden(&rel) || config.matches_ignore(&rel) {
                    continue;
                }
                let ext = LibraryPaths::ext_of(&rel);
                if ext.is_empty() || !exts.contains(&ext) {
                    continue;
                }
                let mtime = meta.modified().map(unix_ms).unwrap_or(0);
                facts.insert(rel.clone(), (meta.len(), mtime));
            }
        }
    }
    // 回收站条目：保留目录结构整体枚举
    Ok(facts)
}

/// 与内存索引比对并应用差异（启动扫描与运行期重扫共用）。
/// progress 回调按阶段上报；startup 为 StartupState，运行期为 TaskTracker
#[allow(clippy::too_many_arguments)]
pub fn reconcile_with_index(
    paths: &LibraryPaths,
    config: &LibraryConfigSnapshot,
    index: &mut ItemIndex,
    store: &MetadataStore,
    bus: &crate::core::events::EventBus,
    fulltext: Option<&crate::core::fulltext::FulltextIndex>,
    progress: &dyn Fn(Phase, u64, u64),
) -> ScanStats {
    let started = Instant::now();
    let stats_start = unix_ms(std::time::SystemTime::now());

    // scan 阶段：枚举
    progress(Phase::Scan, 0, 0);
    let facts = scan_library(paths, config).unwrap_or_default();

    // 差异收集：新路径 / 事实变化（size/mtime 与索引记录不一致）/ 消失位置
    let mut to_process: Vec<(String, u64, i64)> = Vec::new();
    let mut missing: Vec<String> = Vec::new();
    let mut known: HashMap<&str, &(u64, i64)> = facts.iter().map(|(k, v)| (k.as_str(), v)).collect();

    let indexed_paths: Vec<PathRecord> = index
        .iter()
        .flat_map(|item| item.paths.iter().cloned())
        .collect();
    for record in &indexed_paths {
        match known.remove(record.path.as_str()) {
            Some(&(size, mtime)) => {
                if size != record.size || mtime != record.modification_time {
                    to_process.push((record.path.clone(), size, mtime));
                }
            }
            None => missing.push(record.path.clone()),
        }
    }
    // 剩余 known 即新文件
    let mut new_files: Vec<(String, u64, i64)> = known
        .into_iter()
        .map(|(rel, &(size, mtime))| (rel.to_string(), size, mtime))
        .collect();
    to_process.append(&mut new_files);
    // 新文件先处理（迁移判定依赖位置归属）
    to_process.sort();

    // hash 阶段：串行经流水线应用（哈希本身在 apply_file_fact 内完成；
    // 并行化优化留待压测后引入——正确性优先，语义与单写者模型一致）
    let total = to_process.len() as u64;
    let mut applied = 0u64;
    let mut ctx = PipelineCtx { paths, index, store, bus, fulltext };
    for (rel, size, mtime) in &to_process {
        progress(Phase::Hash, applied, total);
        if apply_file_fact(&mut ctx, rel, *size, *mtime).is_some() {
            applied += 1;
        }
    }

    // 消失位置对账（监听漏掉的删除一并收敛）
    progress(Phase::Apply, 0, missing.len() as u64);
    for rel in &missing {
        apply_path_removed(&mut ctx, rel);
    }

    ScanStats {
        started_unix_ms: stats_start,
        duration_ms: started.elapsed().as_millis() as u64,
        files: facts.len() as u64,
        dirty_dirs: 0,
        applied,
    }
}

/// 启动期注水 + 对账（sync 阶段：store 全量读 → index；外部 TOML 变更并入）
pub fn hydrate_from_store(
    store: &MetadataStore,
    index: &mut ItemIndex,
    startup: &Arc<StartupState>,
) {
    startup.set_phase(Phase::Sync, 0, 0);
    let items = store.load_all();
    index.hydrate(items);
}

/// 运行期周期兜底重扫（[scan] 配置驱动）
pub fn periodic_rescan_loop(
    paths: LibraryPaths,
    config: Arc<crate::core::config::LibraryConfig>,
    shared: Arc<std::sync::Mutex<ItemIndex>>,
    store: Arc<MetadataStore>,
    bus: crate::core::events::EventBus,
    fulltext: Option<std::sync::Arc<crate::core::fulltext::FulltextIndex>>,
    tasks: Arc<TaskTracker>,
    startup: Arc<StartupState>,
) {
    std::thread::spawn(move || {
        loop {
            let snapshot = config.current();
            if !snapshot.scan.periodic {
                std::thread::sleep(std::time::Duration::from_secs(5));
                continue;
            }
            let interval = snapshot.scan.interval.max(60);
            std::thread::sleep(std::time::Duration::from_secs(interval));
            if !startup.is_ready() {
                continue;
            }
            let mut index = shared.lock().unwrap();
            let stats = reconcile_with_index(
                &paths,
                &snapshot,
                &mut index,
                &store,
                &bus,
                fulltext.as_deref(),
                &|_phase, _p, _t| {},
            );
            tasks.finish_scan(stats);
            bus.emit(names::FOLDER_CHANGED_EVENT, serde_json::json!({ "reason": "external" }));
        }
    });
}

/// 事件名常量引用（避免循环导入）
mod names {
    pub const FOLDER_CHANGED_EVENT: &str = crate::core::events::names::FOLDER_CHANGED;
}
