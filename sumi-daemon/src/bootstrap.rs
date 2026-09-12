//! 组件组装与启动编排：先监听端口、初始索引后台构建（先监听、后索引的启动模型）。
//! 启动流程（阶段名与 app/startup 的 phase 对应）：
//! sync（注水+元数据对账）→ scan（遍历）→ hash（哈希）→ apply（应用）→ ready，
/// 随后文件监听与周期兜底重扫接管增量维护。

use crate::api::{self, AppState};
use crate::core::config::LibraryConfig;
use crate::core::events::EventBus;
use crate::core::index::ItemIndex;
use crate::core::locks::Locks;
use crate::core::metadata_store::MetadataStore;
use crate::core::paths::LibraryPaths;
use crate::core::registry_file::{GlobalFilter, NameRegistry, ViewPreferences};
use crate::core::scanner::{hydrate_from_store, periodic_rescan_loop, reconcile_with_index};
use crate::core::startup::{Phase, StartupState};
use crate::core::tasks::TaskTracker;
use crate::core::watcher::spawn_watcher;
use crate::settings::Settings;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};

pub async fn run(settings: Settings) {
    let paths = LibraryPaths::new(&settings.library_root, settings.cache_parent.clone());
    if let Some(reason) = paths.cache_location_error() {
        eprintln!("{reason}（--cache-parent 与 --library 冲突）");
        std::process::exit(2);
    }
    paths.ensure_layout();

    let config = Arc::new(LibraryConfig::load(&paths.config_file));
    let store = Arc::new(MetadataStore::open(paths.clone()));
    // 全文索引（库外缓存 index.db；打开失败只退化全文检索，不阻断启动）
    let fulltext = crate::core::fulltext::FulltextIndex::open(&paths.index_db_file)
        .map(Arc::new)
        .map_err(|e| tracing::warn!("全文索引不可用: {e}"))
        .ok();
    let index = Arc::new(Mutex::new(ItemIndex::new()));
    let startup = Arc::new(StartupState::new());
    let tasks = Arc::new(TaskTracker::new());
    let bus = EventBus::new();

    let categories = Arc::new(NameRegistry::load(&paths.categories_file));
    let tags = Arc::new(NameRegistry::load(&paths.tags_file));
    let prefs = Arc::new(ViewPreferences::load(&paths.view_file));
    let global_filter = Arc::new(GlobalFilter::load(&paths.global_filter_file));
    let locks = Arc::new(Locks::load(&paths.locks_file));

    let state = Arc::new(AppState {
        settings: settings.clone(),
        paths: paths.clone(),
        config: config.clone(),
        startup: startup.clone(),
        tasks: tasks.clone(),
        bus: bus.clone(),
        sse_lagged: Arc::new(AtomicU64::new(0)),
        index: index.clone(),
        store: store.clone(),
        categories,
        tags,
        prefs,
        global_filter,
        locks,
    });

    // 先监听端口；初始索引后台构建（就绪网关期间 /api/* 返回 503 NOT_READY）
    let app = api::build_router(state.clone());
    let addr = format!("127.0.0.1:{}", settings.port);
    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(listener) => listener,
        Err(e) => {
            eprintln!("端口 {addr} 监听失败: {e}（桌面端会回退动态分配，服务器形态请检查端口占用）");
            std::process::exit(2);
        }
    };
    tracing::info!("sumi-daemon 监听 http://{addr}（书库: {}）", settings.library_root);

    // 初始索引（后台线程）：sync → scan → hash → apply → ready
    {
        let paths = paths.clone();
        let config = config.clone();
        let index = index.clone();
        let store = store.clone();
        let bus = bus.clone();
        let startup = startup.clone();
        let tasks = tasks.clone();
        let fulltext_for_scan = fulltext.clone();
        std::thread::spawn(move || {
            // sync：注水（缓存快路径在 S5 接 index.db 后启用，当前全量读权威层）
            hydrate_from_store(&store, &mut index.lock().unwrap(), &startup);
            // scan/hash/apply：遍历比对 + 哈希确认 + 应用
            let snapshot = config.current();
            let stats = reconcile_with_index(
                &paths,
                &snapshot,
                &mut index.lock().unwrap(),
                &store,
                &bus,
                fulltext_for_scan.as_deref(),
                &|phase: Phase, processed, total| {
                    let name = match phase {
                        Phase::Sync => "sync",
                        Phase::Scan => "scan",
                        Phase::Hash => "hash",
                        Phase::Apply => "apply",
                    };
                    startup.set_phase(phase, processed, total);
                    let _ = name;
                },
            );
            tasks.finish_scan(stats);
            startup.set_ready();
            tracing::info!("初始索引完成（{} item）", index.lock().unwrap().len());
        });
    }

    // 文件监听 + 周期兜底重扫
    spawn_watcher(crate::core::watcher::WatchDeps {
        paths: paths.clone(),
        config: config.clone(),
        index: index.clone(),
        store: store.clone(),
        fulltext: fulltext.clone(),
        bus: bus.clone(),
        categories: state.categories.clone(),
        tags: state.tags.clone(),
        prefs: state.prefs.clone(),
        global_filter: state.global_filter.clone(),
        locks: state.locks.clone(),
    });
    periodic_rescan_loop(paths, config, index, store, bus, fulltext, tasks, startup);

    axum::serve(listener, app).await.expect("HTTP 服务异常退出");
}
