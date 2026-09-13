//! 分类/标签/作者/系列端点组：两注册表维度 + 两派生维度。
//! 重命名/合并/删除级联应用到全部 item（items.updated 事件）与注册表键（view/global_filter/locks）。

use crate::api::{envelope, envelope::codes, AccessLevel, SharedState};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Extension;
use serde::{Deserialize, Serialize};
use utoipa_axum::router::OpenApiRouter;

pub fn routes() -> OpenApiRouter<SharedState> {
    OpenApiRouter::new()
        .routes(utoipa_axum::routes!(category_list))
        .routes(utoipa_axum::routes!(category_create))
        .routes(utoipa_axum::routes!(category_update))
        .routes(utoipa_axum::routes!(category_delete))
        .routes(utoipa_axum::routes!(tag_list))
        .routes(utoipa_axum::routes!(tag_create))
        .routes(utoipa_axum::routes!(tag_update))
        .routes(utoipa_axum::routes!(tag_delete))
        .routes(utoipa_axum::routes!(author_list))
        .routes(utoipa_axum::routes!(author_update))
        .routes(utoipa_axum::routes!(author_delete))
        .routes(utoipa_axum::routes!(series_list))
        .routes(utoipa_axum::routes!(series_update))
        .routes(utoipa_axum::routes!(series_delete))
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

#[derive(Serialize)]
struct CountEntry {
    name: String,
    count: u64,
}

/// 注册表 ∪ item 赋值并集（含空预创建；count 为库内 item 数）
fn merged_counts(
    state: &crate::api::AppState,
    registry: &crate::core::registry_file::NameRegistry,
    dimension: &str,
) -> Vec<CountEntry> {
    let mut counts: std::collections::BTreeMap<String, u64> = state
        .index
        .lock()
        .unwrap()
        .dimension_counts(dimension)
        .into_iter()
        .collect();
    for name in registry.names() {
        counts.entry(name).or_insert(0);
    }
    counts
        .into_iter()
        .map(|(name, count)| CountEntry { name, count })
        .collect()
}

#[derive(Deserialize, utoipa::ToSchema)]
struct NameBody {
    name: String,
}

#[derive(Deserialize, utoipa::ToSchema)]
struct RenameBody {
    name: String,
    new_name: String,
}

/// 名称维度重命名：批量应用到全部 item + 注册表键级联 + 合并语义（目标已存在）
fn rename_dimension(
    state: &crate::api::AppState,
    dimension: &str,
    old: &str,
    new: &str,
    registry: Option<&crate::core::registry_file::NameRegistry>,
) -> Result<(usize, Vec<crate::api::item::ItemDto>), envelope::ApiError> {
    let mut index = state.index.lock().unwrap();
    let ids = index.ids_with_named(dimension, old);
    let mut updated = Vec::new();
    for id in &ids {
        if let Some(mut item) = index.get(id).cloned() {
            let changed = match dimension {
                "category" => {
                    item.categories.retain(|c| c != old);
                    if !item.categories.contains(&new.to_string()) {
                        item.categories.push(new.to_string());
                    }
                    true
                }
                "tag" => {
                    item.tags.retain(|t| t != old);
                    if !item.tags.contains(&new.to_string()) {
                        item.tags.push(new.to_string());
                    }
                    true
                }
                "author" => {
                    item.authors.retain(|a| a != old);
                    if !item.authors.contains(&new.to_string()) {
                        item.authors.push(new.to_string());
                    }
                    true
                }
                "series" => {
                    item.series = new.to_string();
                    true
                }
                _ => false,
            };
            if changed {
                state.store.upsert(&item).map_err(envelope::ApiError::internal)?;
                index.upsert(item.clone());
                updated.push(crate::api::item::project_item(state, &item));
            }
        }
    }
    drop(index);
    // 注册表键级联（排序偏好保留目标键；隐藏/锁合并保留目标）
    let _ = state.prefs.migrate_named_scope(dimension, old, new);
    let _ = state.global_filter.migrate_named(dimension, old, new);
    let _ = state.locks.migrate_named(dimension, old, new);
    if let Some(reg) = registry {
        reg.rename(old, new).map_err(envelope::ApiError::internal)?;
    }
    Ok((updated.len(), updated))
}

fn delete_dimension(
    state: &crate::api::AppState,
    dimension: &str,
    name: &str,
    registry: Option<&crate::core::registry_file::NameRegistry>,
) -> Result<usize, envelope::ApiError> {
    let mut index = state.index.lock().unwrap();
    let ids = index.ids_with_named(dimension, name);
    let mut updated = Vec::new();
    for id in &ids {
        if let Some(mut item) = index.get(id).cloned() {
            match dimension {
                "category" => item.categories.retain(|c| c != name),
                "tag" => item.tags.retain(|t| t != name),
                "author" => item.authors.retain(|a| a != name),
                "series" => {
                    item.series = String::new();
                    item.series_index = 0.0;
                }
                _ => {}
            }
            state.store.upsert(&item).map_err(envelope::ApiError::internal)?;
            index.upsert(item.clone());
            updated.push(crate::api::item::project_item(state, &item));
        }
    }
    drop(index);
    let _ = state.prefs.remove(&format!("{dimension}:{name}"));
    let _ = state.global_filter.remove_named(dimension, name);
    let _ = state.locks.remove_named(dimension, name);
    if let Some(reg) = registry {
        reg.remove(name).map_err(envelope::ApiError::internal)?;
    }
    Ok(updated.len())
}

fn emit_items_updated(state: &crate::api::AppState, dtos: Vec<crate::api::item::ItemDto>) {
    if !dtos.is_empty() {
        state
            .bus
            .emit_json(crate::core::events::names::ITEMS_UPDATED, &dtos);
    }
}

macro_rules! registry_endpoints {
    ($dim:literal, $reg:ident, $list:ident, $create:ident, $update:ident, $delete:ident, $exists:expr, $missing:expr) => {
        #[utoipa::path(get, path = concat!("/api/v1/", $dim, "/list"), tag = $dim,
            responses((status = 200, description = "OK")))]
        async fn $list(State(state): State<SharedState>) -> impl IntoResponse {
            let data = merged_counts(&state, &state.$reg, $dim);
            axum::Json(serde_json::json!({ "status": "success", "data": data }))
        }

        #[utoipa::path(post, path = concat!("/api/v1/", $dim, "/create"), tag = $dim,
            request_body = NameBody, responses((status = 200, description = "OK")))]
        async fn $create(
            State(state): State<SharedState>,
            Extension(access): Extension<AccessLevel>,
            envelope::JsonBody(body): envelope::JsonBody<NameBody>,
        ) -> Result<impl IntoResponse, envelope::ApiError> {
            require_writable(access)?;
            let name = body.name.trim();
            if name.is_empty() {
                return Err(envelope::ApiError::invalid_param("名称为空"));
            }
            match state.$reg.insert(name) {
                Ok(true) => Ok(envelope::success()),
                Ok(false) => Err($exists(name)),
                Err(e) => Err(envelope::ApiError::internal(e)),
            }
        }

        #[utoipa::path(post, path = concat!("/api/v1/", $dim, "/update"), tag = $dim,
            request_body = RenameBody, responses((status = 200, description = "OK")))]
        async fn $update(
            State(state): State<SharedState>,
            Extension(access): Extension<AccessLevel>,
            envelope::JsonBody(body): envelope::JsonBody<RenameBody>,
        ) -> Result<impl IntoResponse, envelope::ApiError> {
            require_writable(access)?;
            let old = body.name.trim();
            let new = body.new_name.trim();
            if old.is_empty() || new.is_empty() {
                return Err(envelope::ApiError::invalid_param("名称为空"));
            }
            // 注册表维度：源不存在（注册表与 item 赋值均无此名）→ NOT_FOUND
            {
                let index = state.index.lock().unwrap();
                let known = !index.ids_with_named($dim, old).is_empty() || state.$reg.contains(old);
                if !known {
                    return Err($missing(&body.name));
                }
            }
            let (count, dtos) = rename_dimension(&state, $dim, old, new, Some(&state.$reg))?;
            emit_items_updated(&state, dtos);
            let _ = count;
            Ok(envelope::success())
        }

        #[utoipa::path(post, path = concat!("/api/v1/", $dim, "/delete"), tag = $dim,
            request_body = NameBody, responses((status = 200, description = "OK")))]
        async fn $delete(
            State(state): State<SharedState>,
            Extension(access): Extension<AccessLevel>,
            envelope::JsonBody(body): envelope::JsonBody<NameBody>,
        ) -> Result<impl IntoResponse, envelope::ApiError> {
            require_writable(access)?;
            delete_dimension(&state, $dim, body.name.trim(), Some(&state.$reg))?;
            Ok(envelope::success())
        }
    };
}

// category / tag：注册表维度（create 存在；重命名前先校验源存在）
fn category_exists(n: &str) -> envelope::ApiError { envelope::ApiError::category_exists(n) }
fn category_missing(n: &str) -> envelope::ApiError { envelope::ApiError::category_not_found(n) }
fn tag_exists(n: &str) -> envelope::ApiError {
    envelope::ApiError::new(codes::TAG_EXISTS_FALLBACK, StatusCode::CONFLICT, format!("tag already exists: {n}"))
}
fn tag_missing(n: &str) -> envelope::ApiError { envelope::ApiError::tag_not_found(n) }

registry_endpoints!("category", categories, category_list, category_create, category_update, category_delete,
    category_exists, category_missing);
registry_endpoints!("tag", tags, tag_list, tag_create, tag_update, tag_delete,
    tag_exists, tag_missing);

macro_rules! derived_endpoints {
    ($dim:literal, $list:ident, $update:ident, $delete:ident, $not_found:expr) => {
        #[utoipa::path(get, path = concat!("/api/v1/", $dim, "/list"), tag = $dim,
            responses((status = 200, description = "OK")))]
        async fn $list(State(state): State<SharedState>) -> impl IntoResponse {
            let data: Vec<CountEntry> = state
                .index
                .lock()
                .unwrap()
                .dimension_counts($dim)
                .into_iter()
                .map(|(name, count)| CountEntry { name, count })
                .collect();
            axum::Json(serde_json::json!({ "status": "success", "data": data }))
        }

        #[utoipa::path(post, path = concat!("/api/v1/", $dim, "/update"), tag = $dim,
            request_body = RenameBody, responses((status = 200, description = "OK")))]
        async fn $update(
            State(state): State<SharedState>,
            Extension(access): Extension<AccessLevel>,
            envelope::JsonBody(body): envelope::JsonBody<RenameBody>,
        ) -> Result<impl IntoResponse, envelope::ApiError> {
            require_writable(access)?;
            // 派生维度：源不存在（注册表聚合中无此名）→ NOT_FOUND
            {
                let index = state.index.lock().unwrap();
                if index.ids_with_named($dim, body.name.trim()).is_empty() {
                    return Err($not_found(&body.name));
                }
            }
            rename_dimension(&state, $dim, body.name.trim(), body.new_name.trim(), None)?;
            Ok(envelope::success())
        }

        #[utoipa::path(post, path = concat!("/api/v1/", $dim, "/delete"), tag = $dim,
            request_body = NameBody, responses((status = 200, description = "OK")))]
        async fn $delete(
            State(state): State<SharedState>,
            Extension(access): Extension<AccessLevel>,
            envelope::JsonBody(body): envelope::JsonBody<NameBody>,
        ) -> Result<impl IntoResponse, envelope::ApiError> {
            require_writable(access)?;
            delete_dimension(&state, $dim, body.name.trim(), None)?;
            Ok(envelope::success())
        }
    };
}

// author / series：派生维度（无 create）
fn author_missing(n: &str) -> envelope::ApiError { envelope::ApiError::author_not_found(n) }
fn series_missing(n: &str) -> envelope::ApiError { envelope::ApiError::series_not_found(n) }
derived_endpoints!("author", author_list, author_update, author_delete, author_missing);
derived_endpoints!("series", series_list, series_update, series_delete, series_missing);
