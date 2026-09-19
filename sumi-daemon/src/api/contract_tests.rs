//! OpenAPI 契约测试：固化文件（openapi.json）与代码生成 schema 必须逐字一致——
//! 改 API 后 `cargo run -- --dump-openapi > openapi.json` 重新固化，否则本测试失败。
//! 另含 REST 行为回归（axum oneshot 直连，不起真实端口）：临时库 + 真实文件 + 完整请求链。

use crate::api::{build_router, AppState, SharedState};
use crate::core::config::LibraryConfig;
use crate::core::events::EventBus;
use crate::core::index::ItemIndex;
use crate::core::locks::Locks;
use crate::core::metadata_store::MetadataStore;
use crate::core::paths::LibraryPaths;
use crate::core::registry_file::{GlobalFilter, NameRegistry, ViewPreferences};
use crate::core::startup::StartupState;
use crate::core::tasks::TaskTracker;
use crate::settings::Settings;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

fn temp_library(tag: &str) -> LibraryPaths {
    let dir = std::env::temp_dir().join(format!("sumi-contract-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let paths =
        LibraryPaths::new(dir.to_str().unwrap(), Some(dir.join("cache").to_str().unwrap().to_string()));
    paths.ensure_layout();
    paths
}

fn test_state(paths: &LibraryPaths) -> SharedState {
    Arc::new(AppState {
        settings: Settings {
            library_root: paths.root.clone(),
            port: 0,
            token: "test-token".into(),
            cache_parent: None,
            web_dist: None,
        },
        paths: paths.clone(),
        config: Arc::new(LibraryConfig::load(&paths.config_file)),
        startup: Arc::new(StartupState::new()),
        tasks: Arc::new(TaskTracker::new()),
        bus: EventBus::new(),
        sse_lagged: Arc::new(AtomicU64::new(0)),
        index: Arc::new(Mutex::new(ItemIndex::new())),
        store: Arc::new(MetadataStore::open(paths.clone())),
        fulltext: None,
        categories: Arc::new(NameRegistry::load(&paths.categories_file)),
        tags: Arc::new(NameRegistry::load(&paths.tags_file)),
        prefs: Arc::new(ViewPreferences::load(&paths.view_file)),
        global_filter: Arc::new(GlobalFilter::load(&paths.global_filter_file)),
        locks: Arc::new(Locks::load(&paths.locks_file)),
    })
}

/// 带 admin token 的 POST 请求（Router 可克隆，oneshot 消费副本）
async fn post_json(
    router: &axum::Router,
    path: &str,
    body: serde_json::Value,
) -> (axum::http::StatusCode, serde_json::Value) {
    let req = axum::http::Request::builder()
        .method(axum::http::Method::POST)
        .uri(path)
        .header(axum::http::header::AUTHORIZATION, "Bearer test-token")
        .header(axum::http::header::CONTENT_TYPE, "application/json")
        .body(axum::body::Body::from(body.to_string()))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20).await.unwrap();
    let json = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    (status, json)
}

#[test]
fn openapi_json_in_sync() {
    let generated = crate::api::build_openapi_json();
    // 换行归一：Windows autocrlf 检出的 CRLF 与生成端 LF 差异不代表契约漂移
    let committed = include_str!("../../openapi.json").replace("\r\n", "\n");
    assert_eq!(
        generated, committed,
        "openapi.json 与代码不同步：运行 `cargo run -- --dump-openapi > openapi.json` 重新固化"
    );
}

/// 回归：回收站中的书籍禁止改名/移动（item/update 带 name/folder_path → 400），
/// 纯元数据编辑不受影响。对齐 hawk：回收站文件必须先恢复才能移动。
#[tokio::test]
async fn update_rejects_rename_or_move_for_trashed_item() {
    let paths = temp_library("trash-move");
    let state = test_state(&paths);
    state.startup.set_ready();
    let router = build_router(state);

    // 入库：真实文件 → item
    std::fs::create_dir_all(format!("{}/novels", paths.root)).unwrap();
    let file_abs = format!("{}/novels/三体.txt", paths.root);
    std::fs::write(&file_abs, "第一章 测试正文").unwrap();
    let (status, resp) =
        post_json(&router, "/api/v1/item/add", serde_json::json!({ "path": file_abs })).await;
    assert_eq!(status, axum::http::StatusCode::OK, "item/add 失败: {resp}");
    let id = resp["data"]["item"]["id"].as_str().expect("add 响应缺 data.item.id").to_string();

    // 移入回收站
    let (status, resp) =
        post_json(&router, "/api/v1/item/delete", serde_json::json!({ "id": id })).await;
    assert_eq!(status, axum::http::StatusCode::OK, "item/delete 失败: {resp}");

    // 带 folder_path 的移动 → 400 + 请先恢复
    let (status, resp) = post_json(
        &router,
        "/api/v1/item/update",
        serde_json::json!({ "id": id, "folder_path": "其他" }),
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::BAD_REQUEST, "回收站移动未被拦截: {resp}");
    assert_eq!(resp["error"]["code"], "INVALID_PARAM", "错误码不符: {resp}");
    assert!(
        resp["error"]["message"].as_str().unwrap_or("").contains("请先恢复"),
        "错误文案不符: {resp}"
    );

    // 带 name 的改名 → 400
    let (status, resp) = post_json(
        &router,
        "/api/v1/item/update",
        serde_json::json!({ "id": id, "name": "新名字" }),
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::BAD_REQUEST, "回收站改名未被拦截: {resp}");

    // 纯元数据编辑不受影响 → 200 且评分落库
    let (status, resp) = post_json(
        &router,
        "/api/v1/item/update",
        serde_json::json!({ "id": id, "star": 3 }),
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::OK, "回收站内纯元数据更新失败: {resp}");
    assert_eq!(resp["data"]["star"], serde_json::json!(3), "评分未落库: {resp}");
}
