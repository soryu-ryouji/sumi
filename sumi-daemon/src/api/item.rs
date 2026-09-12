//! item 端点组：查询（list/skeleton/aggregate/detail/count）、封面（GET/PUT/DELETE）、
//! 原文件（file，Range）、元数据读写（update/batch_update）、入库（add/upload）、
//! 回收站（delete/restore）、查看（open/show_in_folder）、阅读（toc/content/resource）、
//! 重解析（refresh_metadata）。契约见 API 文档 item 节。

use crate::api::{envelope, envelope::codes, AccessLevel, AppState, LockView, SharedState};
use crate::core::cover;
use crate::core::item::{ItemCore, PathRecord, ORDER_FIELDS};
use crate::core::paths::LibraryPaths;
use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Extension;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Mutex;
use utoipa_axum::router::OpenApiRouter;

pub fn routes() -> OpenApiRouter<SharedState> {
    OpenApiRouter::new()
        .routes(utoipa_axum::routes!(list))
        .routes(utoipa_axum::routes!(skeleton))
        .routes(utoipa_axum::routes!(aggregate))
        .routes(utoipa_axum::routes!(detail))
        .routes(utoipa_axum::routes!(count))
        .routes(utoipa_axum::routes!(cover_get))
        .routes(utoipa_axum::routes!(cover_put))
        .routes(utoipa_axum::routes!(cover_delete))
        .routes(utoipa_axum::routes!(file))
}

// ---------- DTO 投影 ----------

/// Item DTO（契约与 item/list 响应一致；SSE 负载同构）
#[derive(Serialize, Clone)]
pub struct ItemDto {
    pub id: String,
    pub name: String,
    pub ext: String,
    pub size: u64,
    pub title: String,
    pub authors: Vec<String>,
    pub publisher: String,
    pub pubdate: String,
    pub isbn: String,
    pub language: String,
    pub series: String,
    pub series_index: f64,
    pub description: String,
    pub url: String,
    pub tags: Vec<String>,
    pub categories: Vec<String>,
    pub paths: Vec<String>,
    pub folders: Vec<String>,
    pub star: i64,
    pub read_status: String,
    pub progress: f64,
    pub progress_loc: String,
    pub last_read_time: i64,
    pub annotation: String,
    pub added_time: i64,
    pub modification_time: i64,
    pub custom_cover: bool,
}

/// 投影：内存 ItemCore → DTO（name/ext/size 取主路径；folders 由非回收站位置派生；
/// custom_cover 由文件存在性派生——文件即真源）
pub fn project_item(state: &AppState, item: &ItemCore) -> ItemDto {
    let primary = item.primary_path().to_string();
    let size = item
        .paths
        .iter()
        .find(|p| p.path == primary)
        .map(|p| p.size)
        .unwrap_or(0);
    let mut folders: Vec<String> = item
        .paths
        .iter()
        .filter(|p| !LibraryPaths::is_in_trash(&p.path))
        .map(|p| LibraryPaths::dir_of(&p.path).to_string())
        .filter(|f| !f.is_empty())
        .collect();
    folders.sort();
    folders.dedup();
    let custom_cover =
        std::path::Path::new(&format!("{}/{}.webp", state.paths.covers_dir, item.id)).exists();
    ItemDto {
        id: item.id.clone(),
        name: LibraryPaths::name_of(&primary).to_string(),
        ext: LibraryPaths::ext_of(&primary),
        size,
        title: item.title.clone(),
        authors: item.authors.clone(),
        publisher: item.publisher.clone(),
        pubdate: item.pubdate.clone(),
        isbn: item.isbn.clone(),
        language: item.language.clone(),
        series: item.series.clone(),
        series_index: item.series_index,
        description: item.description.clone(),
        url: item.url.clone(),
        tags: item.tags.clone(),
        categories: item.categories.clone(),
        paths: item.paths.iter().map(|p| p.path.clone()).collect(),
        folders,
        star: item.star,
        read_status: item.read_status.clone(),
        progress: item.progress,
        progress_loc: item.progress_loc.clone(),
        last_read_time: item.last_read_time,
        annotation: item.annotation.clone(),
        added_time: item.added_time,
        modification_time: item
            .paths
            .iter()
            .map(|p| p.modification_time)
            .max()
            .unwrap_or(0),
        custom_cover,
    }
}

// ---------- 查询请求 ----------

#[derive(Deserialize, Default, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ListQuery {
    #[serde(default)]
    pub ids: Vec<String>,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub categories_match: Option<String>,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default)]
    pub series: Option<String>,
    #[serde(default)]
    pub publisher: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub isbn: Option<String>,
    #[serde(default)]
    pub read_status: Option<String>,
    #[serde(default)]
    pub exclude_categories: Vec<String>,
    #[serde(default)]
    pub exclude_tags: Vec<String>,
    #[serde(default)]
    pub exclude_folders: Vec<String>,
    #[serde(default)]
    pub exclude_authors: Vec<String>,
    #[serde(default)]
    pub star: Option<i64>,
    #[serde(default)]
    pub folders: Vec<String>,
    #[serde(default)]
    pub folders_exact: Option<bool>,
    #[serde(default)]
    pub without_categories: Option<bool>,
    #[serde(default)]
    pub without_tags: Option<bool>,
    #[serde(default)]
    pub without_authors: Option<bool>,
    #[serde(default)]
    pub ext: Option<String>,
    #[serde(default)]
    pub annotation: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub in_trash: Option<bool>,
    #[serde(default)]
    pub order_by: Option<String>,
    #[serde(default)]
    pub order: Option<String>,
    #[serde(default)]
    pub offset: Option<u64>,
    #[serde(default)]
    pub limit: Option<u64>,
}

/// 过滤引擎：全部条件 AND；返回 (过滤排序后的条目快照, 全量计数与字节合计)
fn query_items(
    state: &AppState,
    query: &ListQuery,
    lock_view: &LockView,
) -> Result<(Vec<ItemCore>, usize, u64), envelope::ApiError> {
    // 校验枚举参数
    if let Some(rs) = &query.read_status {
        if !crate::core::item::valid_read_status(rs) {
            return Err(envelope::ApiError::invalid_param(format!("非法 read_status: {rs}")));
        }
    }
    if let Some(order_by) = &query.order_by {
        if !crate::core::item::valid_order_field(order_by) {
            return Err(envelope::ApiError::invalid_param(format!("非法 order_by: {order_by}")));
        }
    }
    if let Some(o) = &query.order {
        if o != "asc" && o != "desc" {
            return Err(envelope::ApiError::invalid_param(format!("非法 order: {o}")));
        }
    }

    // 主动筛选命中未解锁锁 → 403 LOCKED
    for f in &query.folders {
        if lock_view.filter_locked("folder", f) {
            return Err(envelope::ApiError::locked(format!("folder {f} is locked")));
        }
    }
    for c in &query.categories {
        if lock_view.filter_locked("category", c) {
            return Err(envelope::ApiError::locked(format!("category {c} is locked")));
        }
    }
    for t in &query.tags {
        if lock_view.filter_locked("tag", t) {
            return Err(envelope::ApiError::locked(format!("tag {t} is locked")));
        }
    }
    for a in &query.authors {
        if lock_view.filter_locked("author", a) {
            return Err(envelope::ApiError::locked(format!("author {a} is locked")));
        }
    }

    // 全文检索候选集（先 FTS 求交，缩小遍历面）
    let fts_hits: Option<HashSet<String>> = match (&query.content, &state.fulltext) {
        (Some(content), Some(fts)) if !content.trim().is_empty() => Some(
            fts.search(content)
                .map_err(envelope::ApiError::internal)?
                .into_iter()
                .collect(),
        ),
        (Some(content), None) if !content.trim().is_empty() => {
            // 全文索引不可用：不命中（可重建缓存，契约允许空结果）
            Some(HashSet::new())
        }
        _ => None,
    };

    let index = state.index.lock().unwrap();
    let in_trash = query.in_trash.unwrap_or(false);

    let keywords_lower: Vec<String> = query.keywords.iter().map(|k| k.to_lowercase()).collect();
    let categories_all = query.categories_match.as_deref() == Some("all");

    let mut candidates: Vec<&ItemCore> = index
        .iter()
        .filter(|item| {
            // 回收站视图：只保留全部位置在回收站的条目；常规视图反之
            let only_trash = !item.has_library_path();
            if only_trash != in_trash {
                return false;
            }
            // 锁排除（仅全局视图；回收站视图不排除）
            if !in_trash && lock_view.hides_item(item) {
                return false;
            }
            if let Some(hits) = &fts_hits {
                if !hits.contains(&item.id) {
                    return false;
                }
            }
            if !query.ids.is_empty() && !query.ids.iter().any(|id| id == &item.id) {
                return false;
            }
            if !keywords_lower.is_empty() {
                let title = item.title.to_lowercase();
                let annotation = item.annotation.to_lowercase();
                for kw in &keywords_lower {
                    let hit_authors = item.authors.iter().any(|a| a.to_lowercase().contains(kw));
                    if !title.contains(kw) && !annotation.contains(kw) && !hit_authors {
                        return false;
                    }
                }
            }
            if !query.tags.is_empty() && !query.tags.iter().all(|t| item.tags.contains(t)) {
                return false;
            }
            if !query.categories.is_empty() {
                let hit = if categories_all {
                    query.categories.iter().all(|c| item.categories.contains(c))
                } else {
                    query.categories.iter().any(|c| item.categories.contains(c))
                };
                if !hit {
                    return false;
                }
            }
            if !query.authors.is_empty() && !query.authors.iter().any(|a| item.authors.contains(a)) {
                return false;
            }
            if let Some(series) = &query.series {
                if &item.series != series {
                    return false;
                }
            }
            if let Some(v) = &query.publisher {
                if &item.publisher != v {
                    return false;
                }
            }
            if let Some(v) = &query.language {
                if &item.language != v {
                    return false;
                }
            }
            if let Some(v) = &query.isbn {
                if &item.isbn != v {
                    return false;
                }
            }
            if let Some(v) = &query.read_status {
                if &item.read_status != v {
                    return false;
                }
            }
            if !query.exclude_categories.is_empty()
                && query.exclude_categories.iter().any(|c| item.categories.contains(c))
            {
                return false;
            }
            if !query.exclude_tags.is_empty()
                && query.exclude_tags.iter().any(|t| item.tags.contains(t))
            {
                return false;
            }
            if !query.exclude_authors.is_empty()
                && query.exclude_authors.iter().any(|a| item.authors.contains(a))
            {
                return false;
            }
            if !query.exclude_folders.is_empty() {
                let excluded = query.exclude_folders.iter().filter(|f| !f.is_empty()).any(|f| {
                    item.paths.iter().any(|p| {
                        p.path == *f || p.path.starts_with(&format!("{f}/"))
                    })
                });
                if excluded {
                    return false;
                }
            }
            if let Some(star) = query.star {
                if item.star != star {
                    return false;
                }
            }
            if !query.folders.is_empty() {
                let exact = query.folders_exact.unwrap_or(false);
                let hit = item.paths.iter().any(|p| {
                    if LibraryPaths::is_in_trash(&p.path) {
                        return false;
                    }
                    let dir = LibraryPaths::dir_of(&p.path);
                    query.folders.iter().any(|f| {
                        if f.is_empty() {
                            // 库根
                            dir.is_empty()
                        } else if exact {
                            dir == *f
                        } else {
                            dir == *f || dir.starts_with(&format!("{f}/"))
                        }
                    })
                });
                if !hit {
                    return false;
                }
            }
            if query.without_categories.unwrap_or(false) && !item.categories.is_empty() {
                return false;
            }
            if query.without_tags.unwrap_or(false) && !item.tags.is_empty() {
                return false;
            }
            if query.without_authors.unwrap_or(false) && !item.authors.is_empty() {
                return false;
            }
            if let Some(ext) = &query.ext {
                let primary = item.primary_path();
                if &LibraryPaths::ext_of(primary) != ext {
                    return false;
                }
            }
            if let Some(v) = &query.annotation {
                if !item.annotation.contains(v.as_str()) {
                    return false;
                }
            }
            if let Some(v) = &query.url {
                if !item.url.contains(v.as_str()) {
                    return false;
                }
            }
            true
        })
        .collect();

    // 排序（主键同值按 id 字典序打破平局，保证逐位确定）
    let order_by = query.order_by.as_deref().unwrap_or("added_time");
    let desc = query.order.as_deref() != Some("asc");
    candidates.sort_by(|a, b| {
        let ord = compare_by(order_by, a, b);
        let ord = if desc { ord.reverse() } else { ord };
        ord.then_with(|| a.id.cmp(&b.id))
    });

    let total = candidates.len() as u64;
    let total_size = candidates.iter().map(|i| primary_size(i)).sum();
    let items = candidates.into_iter().cloned().collect();
    Ok((items, total as usize, total_size))
}

/// 排序主键比较（数值主键统一 i128；文本主键小写化；progress 1 位小数 ×1000 精确编码）
fn compare_by(order_by: &str, a: &ItemCore, b: &ItemCore) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match order_by {
        "title" => a.title.to_lowercase().cmp(&b.title.to_lowercase()),
        "author" => a
            .authors
            .first()
            .map(|s| s.to_lowercase())
            .unwrap_or_default()
            .cmp(&b.authors.first().map(|s| s.to_lowercase()).unwrap_or_default()),
        "size" => (primary_size(a) as i128).cmp(&(primary_size(b) as i128)),
        "star" => a.star.cmp(&b.star),
        "progress" => ((a.progress * 1000.0) as i128).cmp(&((b.progress * 1000.0) as i128)),
        "last_read_time" => a.last_read_time.cmp(&b.last_read_time),
        "modification_time" => mtime_of(a).cmp(&mtime_of(b)),
        "pubdate" => a.pubdate.cmp(&b.pubdate),
        _ => a.added_time.cmp(&b.added_time),
    }
}

fn mtime_of(item: &ItemCore) -> i64 {
    item.paths.iter().map(|p| p.modification_time).max().unwrap_or(0)
}

fn primary_size(item: &ItemCore) -> u64 {
    let primary = item.primary_path();
    item.paths
        .iter()
        .find(|p| p.path == primary)
        .map(|p| p.size)
        .unwrap_or(0)
}

// ---------- 端点 ----------

#[derive(Serialize)]
struct ListData {
    items: Vec<ItemDto>,
    total: u64,
    total_size: u64,
    offset: u64,
    limit: u64,
}

/// `POST /api/v1/item/list`：查询 item 列表（过滤条件复杂故 POST；全部 AND）
#[utoipa::path(post, path = "/api/v1/item/list", tag = "item",
    request_body = ListQuery, responses((status = 200, description = "OK")))]
async fn list(
    State(state): State<SharedState>,
    Extension(lock_view): Extension<LockView>,
    envelope::JsonBody(query): envelope::JsonBody<ListQuery>,
) -> Result<axum::Json<serde_json::Value>, envelope::ApiError> {
    let (items, total, total_size) = query_items(&state, &query, &lock_view)?;
    let total = total as u64;
    let total_size = total_size as u64;
    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(50).min(1000);
    let window: Vec<ItemDto> = items
        .into_iter()
        .skip(offset as usize)
        .take(limit as usize)
        .map(|item| project_item(&state, &item))
        .collect();
    Ok(axum::Json(serde_json::json!({
        "status": "success",
        "data": ListData {
            items: window,
            total,
            total_size,
            offset,
            limit,
        }
    })))
}

#[derive(Serialize)]
struct SkeletonItem {
    id: String,
    path: String,
    width: u32,
    height: u32,
    star: i64,
    size: u64,
}

#[derive(Serialize)]
struct SkeletonData {
    items: Vec<SkeletonItem>,
    total_size: u64,
}

/// `POST /api/v1/item/skeleton`：全量布局骨架（与 list 同序，不分页，只返回布局所需最低字段）
#[utoipa::path(post, path = "/api/v1/item/skeleton", tag = "item",
    request_body = ListQuery, responses((status = 200, description = "OK")))]
async fn skeleton(
    State(state): State<SharedState>,
    Extension(lock_view): Extension<LockView>,
    envelope::JsonBody(query): envelope::JsonBody<ListQuery>,
) -> Result<axum::Json<serde_json::Value>, envelope::ApiError> {
    let (items, _, total_size) = query_items(&state, &query, &lock_view)?;
    let skeleton: Vec<SkeletonItem> = items
        .into_iter()
        .map(|item| {
            let primary = item.primary_path().to_string();
            SkeletonItem {
                id: item.id.clone(),
                path: primary,
                width: item.cover_width,
                height: item.cover_height,
                star: item.star,
                size: primary_size(&item),
            }
        })
        .collect();
    Ok(axum::Json(serde_json::json!({
        "status": "success",
        "data": SkeletonData { items: skeleton, total_size }
    })))
}

#[derive(Deserialize)]
struct AggregateQuery {
    #[serde(default)]
    ids: Vec<String>,
}

#[derive(Serialize)]
struct AggregateData {
    tags: Vec<String>,
    categories: Vec<String>,
    authors: Vec<String>,
    missing_ids: Vec<String>,
}

/// `POST /api/v1/item/aggregate`：选择集共有特性聚合（交集）
#[utoipa::path(post, path = "/api/v1/item/aggregate", tag = "item",
    responses((status = 200, description = "OK")))]
async fn aggregate(
    State(state): State<SharedState>,
    Extension(_lock_view): Extension<LockView>,
    envelope::JsonBody(query): envelope::JsonBody<AggregateQuery>,
) -> Result<axum::Json<serde_json::Value>, envelope::ApiError> {
    let index = state.index.lock().unwrap();
    let mut tags: Option<HashSet<String>> = None;
    let mut categories: Option<HashSet<String>> = None;
    let mut authors: Option<HashSet<String>> = None;
    let mut missing = Vec::new();
    let mut seen = HashSet::new();
    for id in &query.ids {
        if !seen.insert(id.clone()) {
            continue;
        }
        match index.get(id) {
            Some(item) => {
                let t: HashSet<String> = item.tags.iter().cloned().collect();
                tags = Some(match tags {
                    Some(prev) => prev.intersection(&t).cloned().collect(),
                    None => t,
                });
                let c: HashSet<String> = item.categories.iter().cloned().collect();
                categories = Some(match categories {
                    Some(prev) => prev.intersection(&c).cloned().collect(),
                    None => c,
                });
                let a: HashSet<String> = item.authors.iter().cloned().collect();
                authors = Some(match authors {
                    Some(prev) => prev.intersection(&a).cloned().collect(),
                    None => a,
                });
            }
            None => missing.push(id.clone()),
        }
    }
    let mut sort_uniq = |set: Option<HashSet<String>>| -> Vec<String> {
        let mut v: Vec<String> = set.unwrap_or_default().into_iter().collect();
        v.sort();
        v
    };
    Ok(axum::Json(serde_json::json!({
        "status": "success",
        "data": AggregateData {
            tags: sort_uniq(tags),
            categories: sort_uniq(categories),
            authors: sort_uniq(authors),
            missing_ids: missing,
        }
    })))
}

/// `GET /api/v1/item/detail?id=`：单个 Item；不存在 ITEM_NOT_FOUND
#[utoipa::path(get, path = "/api/v1/item/detail", tag = "item",
    params(("id" = String, Query)), responses((status = 200, description = "OK")))]
async fn detail(
    State(state): State<SharedState>,
    Extension(lock_view): Extension<LockView>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Result<axum::Json<serde_json::Value>, envelope::ApiError> {
    let id = params.get("id").map(String::as_str).unwrap_or("");
    let index = state.index.lock().unwrap();
    let item = index.get(id).ok_or_else(|| envelope::ApiError::item_not_found(id))?;
    if lock_view.hides_item(item) {
        return Err(envelope::ApiError::locked("item is locked"));
    }
    let dto = project_item(&state, item);
    drop(index);
    Ok(axum::Json(serde_json::json!({ "status": "success", "data": dto })))
}

/// `GET /api/v1/item/count`：库内 item 总数（不含回收站）
#[utoipa::path(get, path = "/api/v1/item/count", tag = "item",
    responses((status = 200, description = "OK")))]
async fn count(State(state): State<SharedState>) -> impl IntoResponse {
    let total = state.index.lock().unwrap().library_count();
    axum::Json(serde_json::json!({ "status": "success", "data": total }))
}

// ---------- 封面 ----------

#[derive(Deserialize)]
struct IdQuery {
    id: String,
}

/// `GET /api/v1/item/cover?id=`：封面（自定义优先于自动提取；自动封面四分支响应契约见 API 文档）
#[utoipa::path(get, path = "/api/v1/item/cover", tag = "item",
    params(("id" = String, Query)), responses((status = 200, description = "封面 webp 字节")))]
async fn cover_get(
    State(state): State<SharedState>,
    Extension(lock_view): Extension<LockView>,
    Query(params): Query<IdQuery>,
) -> Response {
    let item = {
        let index = state.index.lock().unwrap();
        match index.get(&params.id) {
            Some(item) => item.clone(),
            None => {
                return envelope::ApiError::item_not_found(&params.id).into_response();
            }
        }
    };
    if lock_view.hides_item(&item) {
        return envelope::ApiError::locked("item is locked").into_response();
    }

    // 1. 自定义封面（用户数据，参与同步）
    let custom = format!("{}/{}.webp", state.paths.covers_dir, item.id);
    if let Ok(bytes) = std::fs::read(&custom) {
        return cover_response(bytes);
    }
    // 2. 派生缓存命中
    let cached = cover::cache_cover_path(&state.paths.cache_covers_dir, &item.id);
    if let Ok(bytes) = std::fs::read(&cached) {
        return cover_response(bytes);
    }
    // 3. 可即时生成（txt/md/docx 排版封面）
    if cover::can_generate_cover(&LibraryPaths::ext_of(item.primary_path())) {
        let (bytes, w, h) = cover::generated_cover(&item.title, &item.authors);
        if !bytes.is_empty() {
            let _ = crate::core::config::atomic_write(&cached, &bytes);
            // 尺寸回填（派生信息是内容的纯函数）
            update_cover_dims(&state, &item.id, w, h);
            return cover_response(bytes);
        }
    }
    // 4. 需重计算 → 404（后台生成中；refresh_metadata 手动修复入口）
    (StatusCode::NOT_FOUND, "cover not ready").into_response()
}

fn update_cover_dims(state: &AppState, id: &str, w: u32, h: u32) {
    let mut index = state.index.lock().unwrap();
    if let Some(item) = index.get(id).cloned() {
        if item.cover_width != w || item.cover_height != h {
            let mut updated = item;
            updated.cover_width = w;
            updated.cover_height = h;
            if state.store.upsert(&updated).is_ok() {
                index.upsert(updated);
            }
        }
    }
}

fn cover_response(bytes: Vec<u8>) -> Response {
    (
        [(header::CONTENT_TYPE, "image/webp"), (header::CACHE_CONTROL, "immutable")],
        bytes,
    )
        .into_response()
}

#[derive(Deserialize, utoipa::ToSchema)]
struct CoverPut {
    id: String,
    img_base64: String,
}

/// `PUT /api/v1/item/cover`：设置自定义封面（解码校验→缩入 1024→webp→.sumi/covers/）
#[utoipa::path(put, path = "/api/v1/item/cover", tag = "item",
    request_body = CoverPut, responses((status = 200, description = "OK")))]
async fn cover_put(
    State(state): State<SharedState>,
    Extension(access): Extension<AccessLevel>,
    envelope::JsonBody(body): envelope::JsonBody<CoverPut>,
) -> Result<axum::Json<serde_json::Value>, envelope::ApiError> {
    require_writable(access)?;
    let raw = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &body.img_base64)
        .map_err(|_| envelope::ApiError::invalid_param("img_base64 解码失败"))?;
    let processed = cover::process_cover(&raw)
        .ok_or_else(|| envelope::ApiError::new(codes::UNSUPPORTED_FORMAT, StatusCode::BAD_REQUEST, "内容不可解码为图像"))?;
    let (_, w, h) = (processed.0.clone(), processed.1, processed.2);

    let mut index = state.index.lock().unwrap();
    let item = index
        .get(&body.id)
        .cloned()
        .ok_or_else(|| envelope::ApiError::item_not_found(&body.id))?;
    let target = format!("{}/{}.webp", state.paths.covers_dir, item.id);
    crate::core::config::atomic_write(&target, &processed.0)
        .map_err(|e| envelope::ApiError::internal(format!("封面写入失败: {e}")))?;
    let mut updated = item;
    updated.cover_width = w;
    updated.cover_height = h;
    state
        .store
        .upsert(&updated)
        .map_err(envelope::ApiError::internal)?;
    index.upsert(updated.clone());
    let dto = project_item(&state, &updated);
    drop(index);
    state.bus.emit_json(crate::core::events::names::ITEM_UPDATED, &dto);
    Ok(axum::Json(serde_json::json!({ "status": "success", "data": dto })))
}

/// `DELETE /api/v1/item/cover?id=`：移除自定义封面（幂等；回退自动提取链）
#[utoipa::path(delete, path = "/api/v1/item/cover", tag = "item",
    params(("id" = String, Query)), responses((status = 200, description = "OK")))]
async fn cover_delete(
    State(state): State<SharedState>,
    Extension(access): Extension<AccessLevel>,
    Query(params): Query<IdQuery>,
) -> Result<impl IntoResponse, envelope::ApiError> {
    require_writable(access)?;
    let target = format!("{}/{}.webp", state.paths.covers_dir, params.id);
    let existed = std::path::Path::new(&target).exists();
    if existed {
        std::fs::remove_file(&target)
            .map_err(|e| envelope::ApiError::internal(format!("封面删除失败: {e}")))?;
    }
    let index = state.index.lock().unwrap();
    if let Some(item) = index.get(&params.id) {
        let dto = project_item(&state, item);
        drop(index);
        state.bus.emit_json(crate::core::events::names::ITEM_UPDATED, &dto);
    }
    Ok(envelope::success())
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

// ---------- 原文件 ----------

/// `GET /api/v1/item/file?id=`：原书文件字节（支持 Range；Cache-Control immutable）
#[utoipa::path(get, path = "/api/v1/item/file", tag = "item",
    params(("id" = String, Query)), responses((status = 200, description = "原文件字节")))]
async fn file(
    State(state): State<SharedState>,
    Extension(lock_view): Extension<LockView>,
    Query(params): Query<IdQuery>,
    headers: HeaderMap,
) -> Response {
    let item = {
        let index = state.index.lock().unwrap();
        match index.get(&params.id) {
            Some(item) => item.clone(),
            None => return envelope::ApiError::item_not_found(&params.id).into_response(),
        }
    };
    if lock_view.hides_item(&item) {
        return envelope::ApiError::locked("item is locked").into_response();
    }
    let primary = item.primary_path().to_string();
    let Some(abs) = state.paths.to_absolute(&primary) else {
        return (StatusCode::NOT_FOUND, "file missing").into_response();
    };
    let Ok(bytes) = std::fs::read(&abs) else {
        return (StatusCode::NOT_FOUND, "file missing").into_response();
    };
    let content_type = mime_guess::from_path(&abs).first_or_octet_stream().to_string();

    // Range 请求（单段；pdf.js 流式取页与 CBZ 边下边开依赖）
    if let Some(range) = headers.get(header::RANGE).and_then(|v| v.to_str().ok()) {
        if let Some((start, end)) = parse_range(range, bytes.len() as u64) {
            let end = end.min(bytes.len() as u64 - 1);
            let slice = bytes[start as usize..=end as usize].to_vec();
            return (
                StatusCode::PARTIAL_CONTENT,
                [
                    (header::CONTENT_TYPE, content_type.clone()),
                    (
                        header::CONTENT_RANGE,
                        format!("bytes {start}-{end}/{}", bytes.len()),
                    ),
                    (header::ACCEPT_RANGES, "bytes".to_string()),
                    (header::CACHE_CONTROL, "immutable".to_string()),
                ],
                slice,
            )
                .into_response();
        }
    }
    (
        [
            (header::CONTENT_TYPE, content_type),
            (header::ACCEPT_RANGES, "bytes".to_string()),
            (header::CACHE_CONTROL, "immutable".to_string()),
        ],
        bytes,
    )
        .into_response()
}

fn parse_range(range: &str, total: u64) -> Option<(u64, u64)> {
    let spec = range.strip_prefix("bytes=")?;
    let (start, end) = spec.split_once('-')?;
    let start: u64 = start.parse().ok()?;
    let end: u64 = if end.is_empty() {
        total.saturating_sub(1)
    } else {
        end.parse().ok()?
    };
    if start > end || start >= total {
        return None;
    }
    Some((start, end))
}

/// 共享的 Mutex guard 便利别名（写路径使用）
pub type IndexGuard<'a> = std::sync::MutexGuard<'a, crate::core::index::ItemIndex>;
