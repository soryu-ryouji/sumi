//! folder 端点组：真实目录的操作（fs 层 + 索引 + 注册表键级联）。
//! 树来自索引位置的目录聚合（folders 由 paths 派生）+ 磁盘目录枚举。

use crate::api::{envelope, envelope::codes, AccessLevel, AppState, SharedState};
use crate::core::paths::LibraryPaths;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use utoipa_axum::router::OpenApiRouter;

pub fn routes() -> OpenApiRouter<SharedState> {
    OpenApiRouter::new()
        .routes(utoipa_axum::routes!(list))
        .routes(utoipa_axum::routes!(create))
        .routes(utoipa_axum::routes!(update))
        .routes(utoipa_axum::routes!(delete))
        .routes(utoipa_axum::routes!(restore))
}

#[derive(Serialize, Clone)]
pub struct FolderNode {
    pub path: String,
    pub name: String,
    pub children: Vec<FolderNode>,
    pub modification_time: i64,
}

/// 递归建树：磁盘真实目录（含空目录——文件夹树是用户可见的组织主轴）
fn build_tree(state: &AppState, parent: &str) -> Vec<FolderNode> {
    let abs_parent = if parent.is_empty() {
        state.paths.root.clone()
    } else {
        match state.paths.to_absolute(parent) {
            Some(p) => p,
            None => return Vec::new(),
        }
    };
    let mut dirs: Vec<(String, i64)> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&abs_parent) {
        for entry in entries.flatten() {
            let Ok(meta) = entry.metadata() else { continue };
            if !meta.is_dir() {
                continue;
            }
            let abs = entry.path().to_string_lossy().into_owned();
            // 库根下 .sumi 整体跳过（trash 在其内，不进文件夹树）
            if rel_only_top(&abs, &state.paths.root) == ".sumi" {
                continue;
            }
            let Some(rel) = state.paths.to_relative(&abs) else { continue };
            if LibraryPaths::is_hidden(&rel) || state.config.current().matches_ignore(&rel) {
                continue;
            }
            let mtime = meta
                .modified()
                .map(crate::core::paths::unix_ms)
                .unwrap_or(0);
            dirs.push((rel, mtime));
        }
    }
    dirs.sort_by(|a, b| a.0.cmp(&b.0));
    dirs.into_iter()
        .map(|(rel, mtime)| {
            let name = rel.rsplit('/').next().unwrap_or(&rel).to_string();
            FolderNode {
                path: rel.clone(),
                name,
                children: build_tree(state, &rel),
                modification_time: mtime,
            }
        })
        .collect()
}

/// `GET /api/v1/folder/list`：完整文件夹树
#[utoipa::path(get, path = "/api/v1/folder/list", tag = "folder",
    responses((status = 200, description = "OK")))]
async fn list(State(state): State<SharedState>) -> impl IntoResponse {
    let tree = build_tree(&state, "");
    axum::Json(serde_json::json!({ "status": "success", "data": tree }))
}

#[derive(Deserialize, utoipa::ToSchema)]
struct FolderCreate {
    name: String,
    #[serde(default)]
    parent_path: Option<String>,
}

/// 绝对路径相对库根的首段（"." 表示库根本层文件名）
fn rel_only_top(abs: &str, root: &str) -> String {
    abs.strip_prefix(root)
        .map(|r| r.trim_start_matches('/'))
        .map(|r| r.split('/').next().unwrap_or("").to_string())
        .unwrap_or_default()
}

fn io_err(e: std::io::Error) -> envelope::ApiError {
    envelope::ApiError::internal(format!("IO 失败: {e}"))
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

/// `POST /api/v1/folder/create`
#[utoipa::path(post, path = "/api/v1/folder/create", tag = "folder",
    request_body = FolderCreate, responses((status = 200, description = "OK")))]
async fn create(
    State(state): State<SharedState>,
    Extension(access): Extension<AccessLevel>,
    envelope::JsonBody(body): envelope::JsonBody<FolderCreate>,
) -> Result<axum::Json<serde_json::Value>, envelope::ApiError> {
    require_writable(access)?;
    let name = body.name.trim();
    if name.is_empty() || name.contains('/') || name.contains('\\') {
        return Err(envelope::ApiError::invalid_param(format!("非法文件夹名: {name}")));
    }
    let parent = body.parent_path.as_deref().map(str::trim).unwrap_or("").trim_end_matches('/');
    if !parent.is_empty() && !LibraryPaths::is_valid_library_path(Some(parent)) {
        return Err(envelope::ApiError::folder_not_found(parent));
    }
    let rel = if parent.is_empty() { name.to_string() } else { format!("{parent}/{name}") };
    let abs = state
        .paths
        .to_absolute(&rel)
        .ok_or_else(|| envelope::ApiError::invalid_param("路径非法"))?;
    if std::path::Path::new(&abs).exists() {
        return Err(envelope::ApiError::file_exists(&rel));
    }
    std::fs::create_dir_all(&abs).map_err(io_err)?;
    let mtime = crate::core::paths::file_mtime_ms(&abs);
    state
        .bus
        .emit(crate::core::events::names::FOLDER_CHANGED, serde_json::json!({ "reason": "external" }));
    Ok(axum::Json(serde_json::json!({
        "status": "success",
        "data": FolderNode { path: rel, name: name.to_string(), children: vec![], modification_time: mtime }
    })))
}

use axum::Extension;

#[derive(Deserialize, utoipa::ToSchema)]
struct FolderUpdate {
    path: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    parent_path: Option<String>,
}

/// `POST /api/v1/folder/update`：重命名/移动真实目录（索引位置与注册表键级联跟随）
#[utoipa::path(post, path = "/api/v1/folder/update", tag = "folder",
    request_body = FolderUpdate, responses((status = 200, description = "OK")))]
async fn update(
    State(state): State<SharedState>,
    Extension(access): Extension<AccessLevel>,
    envelope::JsonBody(body): envelope::JsonBody<FolderUpdate>,
) -> Result<axum::Json<serde_json::Value>, envelope::ApiError> {
    require_writable(access)?;
    let old = body.path.trim().trim_end_matches('/');
    if !LibraryPaths::is_valid_library_path(Some(old)) {
        return Err(envelope::ApiError::folder_not_found(&body.path));
    }
    let old_name = old.rsplit('/').next().unwrap_or(old).to_string();
    let new_name = body.name.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let new_parent = body
        .parent_path
        .as_deref()
        .map(str::trim)
        .map(|s| s.trim_end_matches('/').to_string())
        .unwrap_or_else(|| match old.rfind('/') {
            Some(i) => old[..i].to_string(),
            None => String::new(),
        });
    if !new_parent.is_empty() && !LibraryPaths::is_valid_library_path(Some(&new_parent)) {
        return Err(envelope::ApiError::folder_not_found(&new_parent));
    }
    let name = new_name.unwrap_or(&old_name).to_string();
    if name.contains('/') || name.contains('\\') {
        return Err(envelope::ApiError::invalid_param(format!("非法文件夹名: {name}")));
    }
    let new = if new_parent.is_empty() { name.clone() } else { format!("{new_parent}/{name}") };
    if new == old {
        return Ok(axum::Json(serde_json::json!({
            "status": "success",
            "data": FolderNode { path: new, name, children: vec![], modification_time: 0 }
        })));
    }
    let (from_abs, to_abs) = match (state.paths.to_absolute(old), state.paths.to_absolute(&new)) {
        (Some(f), Some(t)) => (f, t),
        _ => return Err(envelope::ApiError::invalid_param("路径非法")),
    };
    if !std::path::Path::new(&from_abs).is_dir() {
        return Err(envelope::ApiError::folder_not_found(old));
    }
    if std::path::Path::new(&to_abs).exists() {
        return Err(envelope::ApiError::file_exists(&new));
    }
    std::fs::rename(&from_abs, &to_abs).map_err(io_err)?;

    // 索引位置前缀迁移（含回收站位置的对应条目不动——回收站里是独立副本）
    let mut index = state.index.lock().unwrap();
    let prefix = format!("{old}/");
    let mut changed: Vec<crate::core::item::ItemCore> = Vec::new();
    for item in index.iter() {
        if item.paths.iter().any(|p| p.path == old || p.path.starts_with(&prefix)) {
            let mut updated = item.clone();
            for p in &mut updated.paths {
                if p.path == old {
                    p.path = new.clone();
                } else if p.path.starts_with(&prefix) {
                    p.path = format!("{new}/{}", &p.path[prefix.len()..]);
                }
            }
            changed.push(updated);
        }
    }
    for item in &changed {
        state.store.upsert(item).map_err(envelope::ApiError::internal)?;
        index.upsert(item.clone());
    }
    drop(index);

    // 注册表键级联（view 排序偏好 / global_filter 隐藏 / locks 锁）
    let _ = state.prefs.migrate_folder_scope(old, &new);
    let _ = state.global_filter.migrate_folder(old, &new);
    let _ = state.locks.migrate_folder(old, &new);

    state
        .bus
        .emit(crate::core::events::names::FOLDER_CHANGED, serde_json::json!({ "reason": "external" }));
    Ok(axum::Json(serde_json::json!({
        "status": "success",
        "data": FolderNode { path: new, name, children: vec![], modification_time: crate::core::paths::file_mtime_ms(&to_abs) }
    })))
}

#[derive(Deserialize, utoipa::ToSchema)]
struct FolderPath {
    path: String,
}

/// `POST /api/v1/folder/delete`：目录整体移入回收站；目录已不存在时幂等清理索引残留
#[utoipa::path(post, path = "/api/v1/folder/delete", tag = "folder",
    request_body = FolderPath, responses((status = 200, description = "OK")))]
async fn delete(
    State(state): State<SharedState>,
    Extension(access): Extension<AccessLevel>,
    envelope::JsonBody(body): envelope::JsonBody<FolderPath>,
) -> Result<impl IntoResponse, envelope::ApiError> {
    require_writable(access)?;
    let rel = body.path.trim().trim_end_matches('/');
    if !LibraryPaths::is_valid_library_path(Some(rel)) {
        return Err(envelope::ApiError::folder_not_found(&body.path));
    }
    let Some(abs) = state.paths.to_absolute(rel) else {
        return Err(envelope::ApiError::folder_not_found(&body.path));
    };

    if std::path::Path::new(&abs).is_dir() {
        let trash_rel = LibraryPaths::library_to_trash_path(rel);
        let trash_abs = state
            .paths
            .to_absolute(&trash_rel)
            .ok_or_else(|| envelope::ApiError::internal("回收站路径非法"))?;
        if std::path::Path::new(&trash_abs).exists() {
            return Err(envelope::ApiError::file_exists(&trash_rel));
        }
        if let Some(parent) = std::path::Path::new(&trash_abs).parent() {
            std::fs::create_dir_all(parent).map_err(io_err)?;
        }
        std::fs::rename(&abs, &trash_abs).map_err(io_err)?;

        // 索引位置迁移到回收站前缀
        let mut index = state.index.lock().unwrap();
        let prefix = format!("{rel}/");
        let mut changed = Vec::new();
        for item in index.iter() {
            if item.paths.iter().any(|p| p.path == rel || p.path.starts_with(&prefix)) {
                let mut updated = item.clone();
                for p in &mut updated.paths {
                    if p.path == rel {
                        p.path = trash_rel.clone();
                    } else if p.path.starts_with(&prefix) {
                        p.path = format!("{trash_rel}/{}", &p.path[prefix.len()..]);
                    }
                }
                changed.push(updated);
            }
        }
        for item in &changed {
            state.store.upsert(item).map_err(envelope::ApiError::internal)?;
            index.upsert(item.clone());
        }
    } else {
        // 幂等删除：清索引位置与注册表残留（外部删除后的陈旧条目清理入口）
        let mut index = state.index.lock().unwrap();
        let prefix = format!("{rel}/");
        let mut changed = Vec::new();
        for item in index.iter() {
            if item.paths.iter().any(|p| p.path == rel || p.path.starts_with(&prefix)) {
                let mut updated = item.clone();
                updated
                    .paths
                    .retain(|p| p.path != rel && !p.path.starts_with(&prefix));
                changed.push(updated);
            }
        }
        for mut item in changed {
            if item.paths.is_empty() {
                crate::core::pipeline::drop_item(&mut crate::core::pipeline::PipelineCtx {
                    paths: &state.paths,
                    index: &mut index,
                    store: &state.store,
                    bus: &state.bus,
                    fulltext: state.fulltext.as_deref(),
                }, &item.id);
            } else {
                state.store.upsert(&item).map_err(envelope::ApiError::internal)?;
                index.upsert(item.clone());
            }
            let _ = &mut item;
        }
        let _ = state.prefs.remove_folder_prefix(rel);
        let _ = state.global_filter.remove_folder_prefix(rel);
        let _ = state.locks.remove_folder_prefix(rel);
    }
    Ok(envelope::success())
}

/// `POST /api/v1/folder/restore`：从回收站恢复目录
#[utoipa::path(post, path = "/api/v1/folder/restore", tag = "folder",
    request_body = FolderPath, responses((status = 200, description = "OK")))]
async fn restore(
    State(state): State<SharedState>,
    Extension(access): Extension<AccessLevel>,
    envelope::JsonBody(body): envelope::JsonBody<FolderPath>,
) -> Result<impl IntoResponse, envelope::ApiError> {
    require_writable(access)?;
    let original = body.path.trim().trim_end_matches('/');
    let trash_rel = LibraryPaths::library_to_trash_path(original);
    let Some(trash_abs) = state.paths.to_absolute(&trash_rel) else {
        return Err(envelope::ApiError::folder_not_found(&body.path));
    };
    if !std::path::Path::new(&trash_abs).is_dir() {
        return Err(envelope::ApiError::folder_not_found(&body.path));
    }
    let Some(dest_abs) = state.paths.to_absolute(original) else {
        return Err(envelope::ApiError::invalid_param("路径非法"));
    };
    if std::path::Path::new(&dest_abs).exists() {
        return Err(envelope::ApiError::file_exists(original));
    }
    if let Some(parent) = std::path::Path::new(&dest_abs).parent() {
        std::fs::create_dir_all(parent).map_err(io_err)?;
    }
    std::fs::rename(&trash_abs, &dest_abs).map_err(io_err)?;

    // 索引位置从回收站迁回
    let mut index = state.index.lock().unwrap();
    let prefix = format!("{trash_rel}/");
    let mut changed = Vec::new();
    for item in index.iter() {
        if item.paths.iter().any(|p| p.path == trash_rel || p.path.starts_with(&prefix)) {
            let mut updated = item.clone();
            for p in &mut updated.paths {
                if p.path == trash_rel {
                    p.path = original.to_string();
                } else if p.path.starts_with(&prefix) {
                    p.path = format!("{original}/{}", &p.path[prefix.len()..]);
                }
            }
            changed.push(updated);
        }
    }
    for item in &changed {
        state.store.upsert(item).map_err(envelope::ApiError::internal)?;
        index.upsert(item.clone());
    }
    Ok(envelope::success())
}
