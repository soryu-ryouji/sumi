//! 组件组装与启动编排：先监听端口、初始索引后台构建（先监听、后索引的启动模型）。
//! 桌面版由 Electron 预选端口并注入 token；本模块对调用方无感知。

use crate::api::{self, AppState};
use crate::core::config::LibraryConfig;
use crate::core::events::EventBus;
use crate::core::paths::LibraryPaths;
use crate::core::startup::StartupState;
use crate::core::tasks::TaskTracker;
use crate::settings::Settings;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;

pub async fn run(settings: Settings) {
    let paths = LibraryPaths::new(&settings.library_root, settings.cache_parent.clone());
    if let Some(reason) = paths.cache_location_error() {
        eprintln!("{reason}（--cache-parent 与 --library 冲突）");
        std::process::exit(2);
    }
    paths.ensure_layout();

    let config = Arc::new(LibraryConfig::load(&paths.config_file));
    let state = Arc::new(AppState {
        settings: settings.clone(),
        paths,
        config,
        startup: Arc::new(StartupState::new()),
        tasks: Arc::new(TaskTracker::new()),
        bus: EventBus::new(),
        sse_lagged: std::sync::Arc::new(AtomicU64::new(0)),
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

    // 初始索引：本波为骨架占位（直接就绪），流水线接入后替换为真实构建
    // TODO(S4): startup 四阶段（sync/scan/hash/apply）+ 运行期监听
    state.startup.set_ready();

    axum::serve(listener, app).await.expect("HTTP 服务异常退出");
}
