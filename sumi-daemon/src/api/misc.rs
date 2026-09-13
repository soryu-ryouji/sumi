//! view / global_filter / lock / trash / library 端点组（core 层注册表已就绪，这里只做 HTTP 壳与事件）。

use crate::api::{envelope, envelope::codes, AccessLevel, AppState, SharedState};
use crate::core::item::valid_order_field;
use crate::core::locks::{SetLockError, UnlockError};
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Extension;
use serde::{Deserialize, Serialize};
use utoipa_axum::router::OpenApiRouter;

pub fn routes() -> OpenApiRouter<SharedState> {
    OpenApiRouter::new()
        .routes(utoipa_axum::routes!(view_preferences))
        .routes(utoipa_axum::routes!(view_preference_put))
        .routes(utoipa_axum::routes!(view_preference_delete))
        .routes(utoipa_axum::routes!(global_filter_list))
        .routes(utoipa_axum::routes!(global_filter_put))
        .routes(utoipa_axum::routes!(lock_list))
        .routes(utoipa_axum::routes!(lock_set))
        .routes(utoipa_axum::routes!(lock_remove))
        .routes(utoipa_axum::routes!(lock_unlock))
        .routes(utoipa_axum::routes!(trash_clear))
        .routes(utoipa_axum::routes!(library_info_get))
        .routes(utoipa_axum::routes!(library_info_patch))
        .routes(utoipa_axum::routes!(library_scan_put))
        .routes(utoipa_axum::routes!(library_storage_mode))
        .routes(utoipa_axum::routes!(library_reindex))
        .routes(utoipa_axum::routes!(library_rescan))
        .routes(utoipa_axum::routes!(library_refresh_cache))
        .routes(utoipa_axum::routes!(library_cleanup_index))
}

fn require_writable(access: AccessLevel) -> Result<(), envelope::ApiError> {
    match access {
        AccessLevel::Admin => Ok(()),
        AccessLevel::Viewer { writable: false } => Err(envelope::ApiError::new(
            codes::READ_ONLY,
            StatusCode::FORBIDDEN,
            "viewer token is read-only",
        )),
        AccessLevel::Viewer { writable: true } => Ok(()),
    }
}

fn require_admin(access: AccessLevel) -> Result<(), envelope::ApiError> {
    match access {
        AccessLevel::Admin => Ok(()),
        AccessLevel::Viewer { .. } => Err(envelope::ApiError::new(
            codes::READ_ONLY,
            StatusCode::FORBIDDEN,
            "admin required",
        )),
    }
}

// ---------- view ----------

/// `GET /api/v1/view/preferences`
#[utoipa::path(get, path = "/api/v1/view/preferences", tag = "view",
    responses((status = 200, description = "OK")))]
async fn view_preferences(State(state): State<SharedState>) -> impl IntoResponse {
    let all = state.prefs.all();
    let mut out = serde_json::Map::new();
    for (scope, pref) in all {
        out.insert(scope, serde_json::json!({ "order_by": pref.order_by, "order": pref.order }));
    }
    axum::Json(serde_json::json!({ "status": "success", "data": out }))
}

#[derive(Deserialize, utoipa::ToSchema)]
struct ViewPrefBody {
    scope: String,
    order_by: String,
    order: String,
}

fn valid_scope(scope: &str) -> bool {
    if let Some(rest) = scope.strip_prefix("folder:") {
        return !rest.is_empty() && !rest.starts_with('/') && !rest.contains("..");
    }
    for prefix in ["category:", "tag:", "author:", "series:"] {
        if let Some(rest) = scope.strip_prefix(prefix) {
            return !rest.is_empty();
        }
    }
    false
}

/// `PUT /api/v1/view/preference`
#[utoipa::path(put, path = "/api/v1/view/preference", tag = "view",
    request_body = ViewPrefBody, responses((status = 200, description = "OK")))]
async fn view_preference_put(
    State(state): State<SharedState>,
    Extension(access): Extension<AccessLevel>,
    envelope::JsonBody(body): envelope::JsonBody<ViewPrefBody>,
) -> Result<impl IntoResponse, envelope::ApiError> {
    require_writable(access)?;
    if !valid_scope(&body.scope) {
        return Err(envelope::ApiError::invalid_param(format!("非法 scope: {}", body.scope)));
    }
    if !valid_order_field(&body.order_by) {
        return Err(envelope::ApiError::invalid_param(format!("非法 order_by: {}", body.order_by)));
    }
    if body.order != "asc" && body.order != "desc" {
        return Err(envelope::ApiError::invalid_param(format!("非法 order: {}", body.order)));
    }
    state
        .prefs
        .set(
            &body.scope,
            crate::core::registry_file::ViewPreference {
                order_by: body.order_by,
                order: body.order,
            },
        )
        .map_err(envelope::ApiError::internal)?;
    Ok(envelope::success())
}

#[derive(Deserialize)]
struct ScopeQuery {
    scope: String,
}

/// `DELETE /api/v1/view/preference?scope=`
#[utoipa::path(delete, path = "/api/v1/view/preference", tag = "view",
    params(("scope" = String, Query)), responses((status = 200, description = "OK")))]
async fn view_preference_delete(
    State(state): State<SharedState>,
    Extension(access): Extension<AccessLevel>,
    Query(params): Query<ScopeQuery>,
) -> Result<impl IntoResponse, envelope::ApiError> {
    require_writable(access)?;
    state
        .prefs
        .remove(&params.scope)
        .map_err(envelope::ApiError::internal)?;
    Ok(envelope::success())
}

// ---------- global_filter ----------

/// `GET /api/v1/global_filter/list`
#[utoipa::path(get, path = "/api/v1/global_filter/list", tag = "global_filter",
    responses((status = 200, description = "OK")))]
async fn global_filter_list(State(state): State<SharedState>) -> impl IntoResponse {
    let snap = state.global_filter.snapshot();
    axum::Json(serde_json::json!({
        "status": "success",
        "data": {
            "folders": snap.folders, "categories": snap.categories,
            "tags": snap.tags, "authors": snap.authors,
        }
    }))
}

#[derive(Deserialize, utoipa::ToSchema)]
struct GlobalFilterBody {
    kind: String,
    name: String,
    hidden: bool,
}

/// `PUT /api/v1/global_filter`：标记/取消隐藏（幂等）
#[utoipa::path(put, path = "/api/v1/global_filter", tag = "global_filter",
    request_body = GlobalFilterBody, responses((status = 200, description = "OK")))]
async fn global_filter_put(
    State(state): State<SharedState>,
    Extension(access): Extension<AccessLevel>,
    envelope::JsonBody(body): envelope::JsonBody<GlobalFilterBody>,
) -> Result<impl IntoResponse, envelope::ApiError> {
    require_writable(access)?;
    if !matches!(body.kind.as_str(), "folder" | "category" | "tag" | "author") {
        return Err(envelope::ApiError::invalid_param(format!("非法 kind: {}", body.kind)));
    }
    if body.name.trim().is_empty() {
        return Err(envelope::ApiError::invalid_param("name 为空"));
    }
    state
        .global_filter
        .set(&body.kind, body.name.trim(), body.hidden)
        .map_err(envelope::ApiError::internal)?;
    let snap = state.global_filter.snapshot();
    state
        .bus
        .emit_json(crate::core::events::names::GLOBAL_FILTER_CHANGED, &snap);
    Ok(envelope::success())
}

// ---------- lock ----------

/// `GET /api/v1/lock/list`（仅名称，不含哈希）
#[utoipa::path(get, path = "/api/v1/lock/list", tag = "lock",
    responses((status = 200, description = "OK")))]
async fn lock_list(State(state): State<SharedState>) -> impl IntoResponse {
    let snap = state.locks.snapshot_names();
    axum::Json(serde_json::json!({
        "status": "success",
        "data": {
            "folders": snap.folders, "categories": snap.categories,
            "tags": snap.tags, "authors": snap.authors,
        }
    }))
}

fn set_lock_error(e: SetLockError) -> envelope::ApiError {
    match e {
        SetLockError::OldPasswordRequired => envelope::ApiError::new(
            codes::OLD_PASSWORD_REQUIRED,
            StatusCode::FORBIDDEN,
            "改密需携带正确旧密码",
        ),
        SetLockError::WrongPassword => envelope::ApiError::unauthorized("密码错误"),
        SetLockError::Throttled(remain) => envelope::ApiError::new(
            codes::THROTTLED,
            StatusCode::TOO_MANY_REQUESTS,
            format!("验证失败次数过多，冷却 {remain}s"),
        ),
        SetLockError::Internal(msg) => envelope::ApiError::internal(msg),
    }
}

fn unlock_error(e: UnlockError) -> envelope::ApiError {
    match e {
        UnlockError::NotFound => envelope::ApiError::lock_not_found(""),
        UnlockError::WrongPassword => envelope::ApiError::unauthorized("密码错误"),
        UnlockError::Throttled(remain) => envelope::ApiError::new(
            codes::THROTTLED,
            StatusCode::TOO_MANY_REQUESTS,
            format!("验证失败次数过多，冷却 {remain}s"),
        ),
    }
}

#[derive(Deserialize, utoipa::ToSchema)]
struct LockSetBody {
    dimension: String,
    name: String,
    password: String,
    #[serde(default)]
    old_password: Option<String>,
}

/// `POST /api/v1/lock/set`（admin 限定）
#[utoipa::path(post, path = "/api/v1/lock/set", tag = "lock",
    request_body = LockSetBody, responses((status = 200, description = "OK")))]
async fn lock_set(
    State(state): State<SharedState>,
    Extension(access): Extension<AccessLevel>,
    envelope::JsonBody(body): envelope::JsonBody<LockSetBody>,
) -> Result<impl IntoResponse, envelope::ApiError> {
    require_admin(access)?;
    if !matches!(body.dimension.as_str(), "folder" | "category" | "tag" | "author") {
        return Err(envelope::ApiError::invalid_param(format!("非法 dimension: {}", body.dimension)));
    }
    state
        .locks
        .set(&body.dimension, body.name.trim(), &body.password, body.old_password.as_deref())
        .map_err(set_lock_error)?;
    let snap = state.locks.snapshot_names();
    state.bus.emit_json(crate::core::events::names::LOCKS_CHANGED, &snap);
    Ok(envelope::success())
}

#[derive(Deserialize, utoipa::ToSchema)]
struct LockBody {
    dimension: String,
    name: String,
    password: String,
}

/// `POST /api/v1/lock/remove`（admin 限定；解除锁需密码）
#[utoipa::path(post, path = "/api/v1/lock/remove", tag = "lock",
    request_body = LockBody, responses((status = 200, description = "OK")))]
async fn lock_remove(
    State(state): State<SharedState>,
    Extension(access): Extension<AccessLevel>,
    envelope::JsonBody(body): envelope::JsonBody<LockBody>,
) -> Result<impl IntoResponse, envelope::ApiError> {
    require_admin(access)?;
    let removed = state
        .locks
        .remove(&body.dimension, body.name.trim(), &body.password)
        .map_err(set_lock_error)?;
    if !removed {
        return Err(envelope::ApiError::lock_not_found(&body.name));
    }
    let snap = state.locks.snapshot_names();
    state.bus.emit_json(crate::core::events::names::LOCKS_CHANGED, &snap);
    Ok(envelope::success())
}

#[derive(Serialize)]
struct UnlockResponse {
    unlock_token: String,
}

/// `POST /api/v1/lock/unlock`：任何有效 token 可解锁 → 发放随机票据（daemon 内存，重启失效）
#[utoipa::path(post, path = "/api/v1/lock/unlock", tag = "lock",
    request_body = LockBody, responses((status = 200, description = "OK")))]
async fn lock_unlock(
    State(state): State<SharedState>,
    Extension(_access): Extension<AccessLevel>,
    envelope::JsonBody(body): envelope::JsonBody<LockBody>,
) -> Result<axum::Json<serde_json::Value>, envelope::ApiError> {
    let ticket = state
        .locks
        .unlock(&body.dimension, body.name.trim(), &body.password)
        .map_err(unlock_error)?
        .unwrap_or_default();
    Ok(axum::Json(serde_json::json!({
        "status": "success",
        "data": UnlockResponse { unlock_token: ticket }
    })))
}

// ---------- trash ----------

/// `POST /api/v1/trash/clear`：彻底删除回收站全部内容（文件 + 元数据 + 封面缓存；不可恢复）
#[utoipa::path(post, path = "/api/v1/trash/clear", tag = "trash",
    responses((status = 200, description = "OK")))]
async fn trash_clear(
    State(state): State<SharedState>,
    Extension(access): Extension<AccessLevel>,
) -> Result<impl IntoResponse, envelope::ApiError> {
    require_writable(access)?;
    // 只剩回收站位置的条目：彻底删除
    let mut index = state.index.lock().unwrap();
    let ids: Vec<String> = index
        .iter()
        .filter(|i| !i.has_library_path())
        .map(|i| i.id.clone())
        .collect();
    let mut ctx = crate::core::pipeline::PipelineCtx {
        paths: &state.paths,
        index: &mut index,
        store: &state.store,
        bus: &state.bus,
        fulltext: state.fulltext.as_deref(),
    };
    for id in &ids {
        ctx.bus.emit(crate::core::events::names::ITEM_REMOVED, serde_json::json!({ "id": id }));
        crate::core::pipeline::drop_item(&mut ctx, id);
    }
    drop(ctx);
    drop(index);
    // 回收站目录清空
    if let Ok(entries) = std::fs::read_dir(&state.paths.trash_dir) {
        for entry in entries.flatten() {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
    Ok(envelope::success())
}

// ---------- library ----------

#[derive(Serialize)]
struct LibraryInfo {
    name: String,
    path: String,
    modification_time: i64,
    application_version: String,
    storage_mode: String,
    scan: ScanInfo,
}

#[derive(Serialize)]
struct ScanInfo {
    periodic: bool,
    interval: u64,
}

/// `GET /api/v1/library/info`
#[utoipa::path(get, path = "/api/v1/library/info", tag = "library",
    responses((status = 200, description = "OK")))]
async fn library_info_get(State(state): State<SharedState>) -> impl IntoResponse {
    let config = state.config.current();
    let info = LibraryInfo {
        name: state.config.display_name(&state.paths.root),
        path: state.paths.root.clone(),
        modification_time: crate::core::paths::file_mtime_ms(&state.paths.root),
        application_version: env!("CARGO_PKG_VERSION").to_string(),
        storage_mode: state.store.mode().as_str().to_string(),
        scan: ScanInfo {
            periodic: config.scan.periodic,
            interval: config.scan.interval,
        },
    };
    axum::Json(serde_json::json!({ "status": "success", "data": info }))
}

#[derive(Deserialize, utoipa::ToSchema)]
struct LibraryNameBody {
    name: String,
}

/// `PATCH /api/v1/library/info`：改库显示名（写 config.toml，广播 library.updated）
#[utoipa::path(patch, path = "/api/v1/library/info", tag = "library",
    request_body = LibraryNameBody, responses((status = 200, description = "OK")))]
async fn library_info_patch(
    State(state): State<SharedState>,
    Extension(access): Extension<AccessLevel>,
    envelope::JsonBody(body): envelope::JsonBody<LibraryNameBody>,
) -> Result<impl IntoResponse, envelope::ApiError> {
    require_writable(access)?;
    let name = body.name.trim();
    state
        .config
        .edit(|doc| {
            if name.is_empty() {
                doc.as_table_mut().remove("name");
            } else {
                doc["name"] = toml_edit::value(name);
            }
            Ok(())
        })
        .map_err(envelope::ApiError::internal)?;
    // 广播完整库信息
    let config = state.config.current();
    let info = LibraryInfo {
        name: state.config.display_name(&state.paths.root),
        path: state.paths.root.clone(),
        modification_time: crate::core::paths::file_mtime_ms(&state.paths.root),
        application_version: env!("CARGO_PKG_VERSION").to_string(),
        storage_mode: state.store.mode().as_str().to_string(),
        scan: ScanInfo {
            periodic: config.scan.periodic,
            interval: config.scan.interval,
        },
    };
    state
        .bus
        .emit_json(crate::core::events::names::LIBRARY_UPDATED, &info);
    Ok(envelope::success())
}

#[derive(Deserialize, utoipa::ToSchema)]
struct ScanBody {
    periodic: bool,
    #[serde(default)]
    interval: Option<u64>,
}

/// `PUT /api/v1/library/scan`：周期兜底重扫设置（写 [scan]，保存即热生效）
#[utoipa::path(put, path = "/api/v1/library/scan", tag = "library",
    request_body = ScanBody, responses((status = 200, description = "OK")))]
async fn library_scan_put(
    State(state): State<SharedState>,
    Extension(access): Extension<AccessLevel>,
    envelope::JsonBody(body): envelope::JsonBody<ScanBody>,
) -> Result<impl IntoResponse, envelope::ApiError> {
    require_writable(access)?;
    let interval = body.interval.unwrap_or(0).max(60);
    state
        .config
        .edit(|doc| {
            doc["scan"]["periodic"] = toml_edit::value(body.periodic);
            doc["scan"]["interval"] = toml_edit::value(interval as i64);
            Ok(())
        })
        .map_err(envelope::ApiError::internal)?;
    let config = state.config.current();
    let info = LibraryInfo {
        name: state.config.display_name(&state.paths.root),
        path: state.paths.root.clone(),
        modification_time: crate::core::paths::file_mtime_ms(&state.paths.root),
        application_version: env!("CARGO_PKG_VERSION").to_string(),
        storage_mode: state.store.mode().as_str().to_string(),
        scan: ScanInfo { periodic: config.scan.periodic, interval: config.scan.interval },
    };
    state
        .bus
        .emit_json(crate::core::events::names::LIBRARY_UPDATED, &info);
    Ok(axum::Json(serde_json::json!({ "status": "success", "data": info })))
}

#[derive(Deserialize, utoipa::ToSchema)]
struct StorageModeBody {
    mode: String,
}

/// `POST /api/v1/library/storage_mode`：切换元数据存储方案（全量迁移后重启生效）
#[utoipa::path(post, path = "/api/v1/library/storage_mode", tag = "library",
    request_body = StorageModeBody, responses((status = 200, description = "OK")))]
async fn library_storage_mode(
    State(state): State<SharedState>,
    Extension(access): Extension<AccessLevel>,
    envelope::JsonBody(body): envelope::JsonBody<StorageModeBody>,
) -> Result<impl IntoResponse, envelope::ApiError> {
    require_admin(access)?;
    let target = crate::core::metadata_store::StorageMode::parse(&body.mode)
        .ok_or_else(|| envelope::ApiError::invalid_param(format!("非法 mode: {}", body.mode)))?;
    let items: Vec<_> = state.index.lock().unwrap().iter().cloned().collect();
    state
        .store
        .migrate(target, &items)
        .map_err(envelope::ApiError::internal)?;
    Ok(envelope::success())
}

fn spawn_rescan(state: SharedState, sub_path: Option<String>) {
    std::thread::spawn(move || {
        let state = &*state;
        let snapshot = state.config.current();
        let (facts, scope) = match &sub_path {
            Some(p) => {
                // 子树重扫：范围内枚举，对账范围也限定在子树内
                let mut facts = crate::core::scanner::FileFacts::new();
                collect_subtree(&state, p, &mut facts);
                (facts, Some(p.clone()))
            }
            None => (
                crate::core::scanner::scan_library(&state.paths, &snapshot).unwrap_or_default(),
                None,
            ),
        };
        let mut index = state.index.lock().unwrap();
        let stats = apply_facts(state, &facts, &mut index, scope.as_deref());
        state.tasks.finish_scan(stats);
        state
            .bus
            .emit(crate::core::events::names::FOLDER_CHANGED, serde_json::json!({ "reason": "external" }));
    });
}

fn collect_subtree(state: &AppState, folder: &str, out: &mut crate::core::scanner::FileFacts) {
    let Some(abs_dir) = state.paths.to_absolute(folder) else { return };
    let snapshot = state.config.current();
    let exts = snapshot.extension_set();
    let mut stack = vec![abs_dir];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let Ok(meta) = entry.metadata() else { continue };
            let abs = entry.path().to_string_lossy().into_owned();
            if meta.is_dir() {
                if crate::core::paths::last_component_is(
                    &abs,
                    crate::core::paths::SUMI_DIR_NAME,
                ) {
                    continue;
                }
                stack.push(abs);
            } else if meta.is_file() {
                let Some(rel) = state.paths.to_relative(&abs) else { continue };
                if crate::core::paths::LibraryPaths::is_internal(&rel) {
                    continue;
                }
                if crate::core::paths::LibraryPaths::is_hidden(&rel)
                    || snapshot.matches_ignore(&rel)
                {
                    continue;
                }
                let ext = crate::core::paths::LibraryPaths::ext_of(&rel);
                if ext.is_empty() || !exts.contains(&ext) {
                    continue;
                }
                out.insert(rel, (meta.len(), meta.modified().map(crate::core::paths::unix_ms).unwrap_or(0)));
            }
        }
    }
}

/// 事实集合对账应用（reindex/rescan 共用）。scope = Some(folder) 时为子树重扫：
/// missing 检测限定在子树内（子树外路径不参与对账，不会被误判消失）
fn apply_facts(
    state: &AppState,
    facts: &crate::core::scanner::FileFacts,
    index: &mut crate::core::index::ItemIndex,
    scope: Option<&str>,
) -> crate::core::tasks::ScanStats {
    let started = std::time::Instant::now();
    let stats_start = crate::core::paths::unix_ms(std::time::SystemTime::now());
    use std::collections::HashMap;
    let mut known: HashMap<&str, &(u64, i64)> = facts.iter().map(|(k, v)| (k.as_str(), v)).collect();
    let mut to_process = Vec::new();
    let mut missing = Vec::new();
    let in_scope = |path: &str| -> bool {
        match scope {
            None => true,
            Some(folder) => {
                path == folder || path.starts_with(&format!("{folder}/"))
            }
        }
    };
    let indexed: Vec<crate::core::item::PathRecord> =
        index.iter().flat_map(|i| i.paths.iter().cloned()).collect();
    for record in &indexed {
        if !in_scope(&record.path) {
            continue;
        }
        match known.remove(record.path.as_str()) {
            Some(&(size, mtime)) => {
                if size != record.size || mtime != record.modification_time {
                    to_process.push((record.path.clone(), size, mtime));
                }
            }
            None => missing.push(record.path.clone()),
        }
    }
    to_process.extend(known.into_iter().map(|(rel, &(s, m))| (rel.to_string(), s, m)));
    to_process.sort();
    let fts = state.fulltext.as_deref();
    let mut ctx = crate::core::pipeline::PipelineCtx {
        paths: &state.paths,
        index,
        store: &state.store,
        bus: &state.bus,
        fulltext: fts,
    };
    let mut applied = 0;
    for (rel, size, mtime) in &to_process {
        if crate::core::pipeline::apply_file_fact(&mut ctx, rel, *size, *mtime).is_some() {
            applied += 1;
        }
    }
    for rel in &missing {
        crate::core::pipeline::apply_path_removed(&mut ctx, rel);
    }
    crate::core::tasks::ScanStats {
        started_unix_ms: stats_start,
        duration_ms: started.elapsed().as_millis() as u64,
        files: facts.len() as u64,
        dirty_dirs: 0,
        applied,
    }
}

/// `POST /api/v1/library/reindex`：全量重建索引（异步，立即返回）
#[utoipa::path(post, path = "/api/v1/library/reindex", tag = "library",
    responses((status = 200, description = "OK")))]
async fn library_reindex(
    State(state): State<SharedState>,
    Extension(access): Extension<AccessLevel>,
) -> Result<impl IntoResponse, envelope::ApiError> {
    require_admin(access)?;
    spawn_rescan(state.clone(), None);
    Ok(envelope::success())
}

#[derive(Deserialize, Default, utoipa::ToSchema)]
struct RescanBody {
    #[serde(default)]
    path: Option<String>,
}

/// `POST /api/v1/library/rescan`：强制重新遍历（可带 path 限定子树）
#[utoipa::path(post, path = "/api/v1/library/rescan", tag = "library",
    request_body = RescanBody, responses((status = 200, description = "OK")))]
async fn library_rescan(
    State(state): State<SharedState>,
    Extension(access): Extension<AccessLevel>,
    envelope::JsonBody(body): envelope::JsonBody<RescanBody>,
) -> Result<impl IntoResponse, envelope::ApiError> {
    require_writable(access)?;
    if let Some(p) = body.path.as_deref() {
        let p = p.trim().trim_end_matches('/');
        if !p.is_empty() && !crate::core::paths::LibraryPaths::is_valid_library_path(Some(p)) {
            return Err(envelope::ApiError::folder_not_found(p));
        }
    }
    spawn_rescan(state.clone(), body.path.clone());
    Ok(envelope::success())
}

#[derive(Deserialize, utoipa::ToSchema)]
struct RefreshCacheBody {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    value: Option<String>,
}

/// `POST /api/v1/library/refresh_cache`：按范围刷新派生缓存（补缺失模式）。
/// 候选筛选与派生分离：短锁取候选快照，重活（封面/全文派生）在锁外执行，
/// 派生后的尺寸变更短暂回锁持久化；dispatched 为本次派发的条目数
#[utoipa::path(post, path = "/api/v1/library/refresh_cache", tag = "library",
    request_body = RefreshCacheBody, responses((status = 200, description = "OK")))]
async fn library_refresh_cache(
    State(state): State<SharedState>,
    Extension(access): Extension<AccessLevel>,
    envelope::JsonBody(body): envelope::JsonBody<RefreshCacheBody>,
) -> Result<axum::Json<serde_json::Value>, envelope::ApiError> {
    require_writable(access)?;
    let kind = body.kind.clone();
    let value = body.value.clone().unwrap_or_default();
    // 候选快照（短锁）：范围内、库内可见、且存在缺失项（封面或全文）
    let candidates: Vec<crate::core::item::ItemCore> = {
        let index = state.index.lock().unwrap();
        index
            .iter()
            .filter(|item| match kind.as_str() {
                "library" => true,
                "folder" => {
                    let f = value.trim().trim_end_matches('/');
                    item.paths.iter().any(|p| {
                        let dir = crate::core::paths::LibraryPaths::dir_of(&p.path);
                        f.is_empty() || dir == f || dir.starts_with(&format!("{f}/"))
                    })
                }
                "category" => item.categories.iter().any(|c| c == &value),
                "tag" => item.tags.iter().any(|t| t == &value),
                "author" => item.authors.iter().any(|a| a == &value),
                _ => false,
            })
            .filter(|item| item.has_library_path())
            .filter(|item| {
                let cover_missing = !std::path::Path::new(&crate::core::cover::cache_cover_path(
                    &state.paths.cache_covers_dir,
                    &item.id,
                ))
                .exists();
                let fts_missing = state
                    .fulltext
                    .as_ref()
                    .map(|fts| !fts.contains(&item.id))
                    .unwrap_or(false);
                cover_missing || fts_missing
            })
            .cloned()
            .collect()
    };
    let dispatched = candidates.len() as u64;
    // 后台执行（锁外重活）
    std::thread::spawn(move || {
        for mut item in candidates {
            let primary = item.primary_path().to_string();
            let Some(abs) = state.paths.to_absolute(&primary) else { continue };
            let cover_missing = !std::path::Path::new(&crate::core::cover::cache_cover_path(
                &state.paths.cache_covers_dir,
                &item.id,
            ))
            .exists();
            if cover_missing {
                // 封面缺失：重跑解析派生（书目回填尊重 overridden，封面重建，尺寸回填）
                crate::core::pipeline::derive_book_facts(
                    &state.paths,
                    state.fulltext.as_deref(),
                    &mut item,
                    &primary,
                    &abs,
                );
                // 派生结果回写（derive 只改内存副本；条目仍在索引才回写）
                let mut index = state.index.lock().unwrap();
                if index.get(&item.id).is_some() && state.store.upsert(&item).is_ok() {
                    index.upsert(item.clone());
                }
            } else if let Some(fts) = state.fulltext.as_ref() {
                // 全文缺失：仅补 FTS（封面仍在，无需重跑封面链）
                if !fts.contains(&item.id) {
                    if let Ok(bytes) = std::fs::read(&abs) {
                        let name = crate::core::paths::LibraryPaths::name_of(&primary).to_string();
                        let ext = crate::core::paths::LibraryPaths::ext_of(&primary);
                        let parsed = crate::core::parser::parse(&name, &ext, &bytes);
                        if let Some(text) = parsed.fulltext {
                            let _ = fts.upsert(&item.id, &text);
                        }
                    }
                }
            }
        }
    });
    Ok(axum::Json(serde_json::json!({
        "status": "success",
        "data": { "dispatched": dispatched, "removed": 0 }
    })))
}

/// `POST /api/v1/library/cleanup_index`：索引体检（清除隐藏/ignore/白名单外/源文件已删的残留位置）
#[utoipa::path(post, path = "/api/v1/library/cleanup_index", tag = "library",
    responses((status = 200, description = "OK")))]
async fn library_cleanup_index(
    State(state): State<SharedState>,
    Extension(access): Extension<AccessLevel>,
) -> Result<axum::Json<serde_json::Value>, envelope::ApiError> {
    require_admin(access)?;
    let config = state.config.current();
    let exts = config.extension_set();
    let mut index = state.index.lock().unwrap();
    let all: Vec<crate::core::item::ItemCore> = index.iter().cloned().collect();
    let mut checked = 0u64;
    let mut hidden = 0u64;
    let mut ignored = 0u64;
    let mut missing = 0u64;
    let mut ctx = crate::core::pipeline::PipelineCtx {
        paths: &state.paths,
        index: &mut index,
        store: &state.store,
        bus: &state.bus,
        fulltext: state.fulltext.as_deref(),
    };
    for item in all {
        let mut updated = item.clone();
        let mut removed = false;
        updated.paths.retain(|p| {
            checked += 1;
            if crate::core::paths::LibraryPaths::is_in_trash(&p.path) {
                return true; // 回收站条目不参与
            }
            if crate::core::paths::LibraryPaths::is_hidden(&p.path) {
                hidden += 1;
                removed = true;
                return false;
            }
            let ext = crate::core::paths::LibraryPaths::ext_of(&p.path);
            if config.matches_ignore(&p.path) || !exts.contains(&ext) {
                ignored += 1;
                removed = true;
                return false;
            }
            let exists = state
                .paths
                .to_absolute(&p.path)
                .map(|abs| std::path::Path::new(&abs).exists())
                .unwrap_or(false);
            if !exists {
                missing += 1;
                removed = true;
                return false;
            }
            true
        });
        if removed {
            if updated.paths.is_empty() {
                crate::core::pipeline::drop_item(&mut ctx, &item.id);
                state.bus.emit(crate::core::events::names::ITEM_REMOVED, serde_json::json!({ "id": item.id }));
            } else {
                state.store.upsert(&updated).map_err(envelope::ApiError::internal)?;
                ctx.index.upsert(updated.clone());
                let dto = crate::api::item::project_item(&state, &updated);
                state.bus.emit_json(crate::core::events::names::ITEM_UPDATED, &dto);
            }
        }
    }
    Ok(axum::Json(serde_json::json!({
        "status": "success",
        "data": {
            "checked": checked, "removed": hidden + ignored + missing,
            "hidden": hidden, "ignored": ignored, "missing": missing
        }
    })))
}
