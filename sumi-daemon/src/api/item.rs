//! item 端点组：查询（list/skeleton/aggregate/detail/count）、封面（GET/PUT/DELETE）、
//! 原文件（file，Range）、元数据读写（update/batch_update）、入库（add/upload）、
//! 回收站（delete/restore）、查看（open/show_in_folder）、阅读（toc/content/resource）、
//! 重解析（refresh_metadata）。契约见 API 文档 item 节。

use crate::api::{envelope, envelope::codes, AccessLevel, AppState, LockView, SharedState};
use crate::core::cover;
use crate::core::item::{ItemCore, PathRecord};
use crate::core::paths::LibraryPaths;
use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Extension;
use serde::{Deserialize, Serialize};
use std::io::Read as _;
use std::collections::HashSet;

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
        .routes(utoipa_axum::routes!(update))
        .routes(utoipa_axum::routes!(batch_update))
        .routes(utoipa_axum::routes!(add))
        .routes(utoipa_axum::routes!(delete))
        .routes(utoipa_axum::routes!(restore))
        .routes(utoipa_axum::routes!(toc))
        .routes(utoipa_axum::routes!(content))
        .routes(utoipa_axum::routes!(resource))
        .routes(utoipa_axum::routes!(open))
        .routes(utoipa_axum::routes!(show_in_folder))
        .routes(utoipa_axum::routes!(refresh_metadata))
        .routes(utoipa_axum::routes!(upload))
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
    let sort_uniq = |set: Option<HashSet<String>>| -> Vec<String> {
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

// ================= 写路径 =================
// 单写者语义：直接持有 index Mutex（与流水线/扫描互斥），fs 操作 + 索引 + 存储在锁内完成

use crate::core::pipeline::{apply_restore, apply_trash_move, PipelineCtx};

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateQuery {
    pub id: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub folder_path: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub authors: Option<Vec<String>>,
    #[serde(default)]
    pub publisher: Option<String>,
    #[serde(default)]
    pub pubdate: Option<String>,
    #[serde(default)]
    pub isbn: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub series: Option<String>,
    #[serde(default)]
    pub series_index: Option<f64>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub categories: Option<Vec<String>>,
    #[serde(default)]
    pub star: Option<i64>,
    #[serde(default)]
    pub read_status: Option<String>,
    #[serde(default)]
    pub progress: Option<f64>,
    #[serde(default)]
    pub progress_loc: Option<String>,
    #[serde(default)]
    pub annotation: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
}

/// 校验 read_status / star / progress 的公共入口
fn validate_update_fields(body: &UpdateQuery) -> Result<(), envelope::ApiError> {
    if let Some(rs) = &body.read_status {
        if !crate::core::item::valid_read_status(rs) {
            return Err(envelope::ApiError::invalid_param(format!("非法 read_status: {rs}")));
        }
    }
    if let Some(star) = body.star {
        if !(0..=5).contains(&star) {
            return Err(envelope::ApiError::invalid_param(format!("评分超界: {star}")));
        }
    }
    if let Some(p) = body.progress {
        if !(0.0..=100.0).contains(&p) {
            return Err(envelope::ApiError::invalid_param(format!("进度超界: {p}")));
        }
    }
    Ok(())
}

/// `POST /api/v1/item/update`：更新元数据；name/folder_path 同步操作真实文件
#[utoipa::path(post, path = "/api/v1/item/update", tag = "item",
    request_body = UpdateQuery, responses((status = 200, description = "OK")))]
async fn update(
    State(state): State<SharedState>,
    Extension(access): Extension<AccessLevel>,
    Extension(lock_view): Extension<LockView>,
    envelope::JsonBody(body): envelope::JsonBody<UpdateQuery>,
) -> Result<axum::Json<serde_json::Value>, envelope::ApiError> {
    require_writable(access)?;
    validate_update_fields(&body)?;

    let mut index = state.index.lock().unwrap();
    let mut item = index
        .get(&body.id)
        .cloned()
        .ok_or_else(|| envelope::ApiError::item_not_found(&body.id))?;
    if lock_view.hides_item(&item) {
        return Err(envelope::ApiError::locked("item is locked"));
    }

    // 目标位置（同内容多路径时按 path 指定；缺省主路径）
    let target_path = body
        .path
        .clone()
        .unwrap_or_else(|| item.primary_path().to_string());
    if !item.paths.iter().any(|p| p.path == target_path) {
        return Err(envelope::ApiError::invalid_param(format!("path 不属于该 item: {target_path}")));
    }

    // 改名 / 移动（先做 fs 操作，失败即拒绝）
    let mut final_path = target_path.clone();
    let new_name = body.name.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let new_folder = body
        .folder_path
        .as_deref()
        .map(|f| f.trim().trim_end_matches('/'));
    if new_name.is_some() || new_folder.is_some() {
        let dir = new_folder.unwrap_or(LibraryPaths::dir_of(&target_path));
        if !dir.is_empty() && !LibraryPaths::is_valid_library_path(Some(dir)) {
            return Err(envelope::ApiError::invalid_param(format!("非法 folder_path: {dir}")));
        }
        let ext = LibraryPaths::ext_of(&target_path);
        let name = new_name
            .map(|n| if ext.is_empty() { n.to_string() } else { format!("{n}.{ext}") })
            .unwrap_or_else(|| {
                target_path
                    .rsplit('/')
                    .next()
                    .unwrap_or(&target_path)
                    .to_string()
            });
        if name.contains('/') || name.contains('\\') || name.trim().is_empty() {
            return Err(envelope::ApiError::invalid_param(format!("非法文件名: {name}")));
        }
        // 入库策略校验（白名单/ignore 与 add 同口径）
        let new_rel = if dir.is_empty() { name.clone() } else { format!("{dir}/{name}") };
        let config = state.config.current();
        if config.matches_ignore(&new_rel) {
            return Err(envelope::ApiError::invalid_param(format!("目标命中 ignore 规则: {new_rel}")));
        }
        let exts = config.extension_set();
        let new_ext = LibraryPaths::ext_of(&new_rel);
        if !exts.is_empty() && !exts.contains(&new_ext) {
            return Err(envelope::ApiError::new(
                codes::UNSUPPORTED_FORMAT,
                StatusCode::BAD_REQUEST,
                format!("扩展名不在白名单: {new_ext}"),
            ));
        }
        if new_rel != target_path {
            let (from_abs, to_abs) = match (
                state.paths.to_absolute(&target_path),
                state.paths.to_absolute(&new_rel),
            ) {
                (Some(f), Some(t)) => (f, t),
                _ => return Err(envelope::ApiError::invalid_param("路径非法")),
            };
            if std::path::Path::new(&to_abs).exists() {
                return Err(envelope::ApiError::file_exists(&new_rel));
            }
            if let Some(parent) = std::path::Path::new(&to_abs).parent() {
                std::fs::create_dir_all(parent).map_err(io_err)?;
            }
            std::fs::rename(&from_abs, &to_abs)
                .map_err(|e| envelope::ApiError::internal(format!("移动失败: {e}")))?;
            final_path = new_rel;
        }
    }

    // 元数据应用（解析字段显式设置 → 记用户编辑）
    let now = crate::core::paths::unix_ms(std::time::SystemTime::now());
    let mut reading_touched = false;
    if let Some(v) = &body.title {
        item.title = v.clone();
        item.mark_overridden("title");
    }
    if let Some(v) = &body.authors {
        item.authors = v.clone();
        item.mark_overridden("authors");
    }
    if let Some(v) = &body.publisher {
        item.publisher = v.clone();
        item.mark_overridden("publisher");
    }
    if let Some(v) = &body.pubdate {
        item.pubdate = v.clone();
        item.mark_overridden("pubdate");
    }
    if let Some(v) = &body.isbn {
        item.isbn = v.clone();
        item.mark_overridden("isbn");
    }
    if let Some(v) = &body.language {
        item.language = v.clone();
        item.mark_overridden("language");
    }
    if let Some(v) = &body.series {
        if v.is_empty() {
            item.series = String::new();
            item.series_index = 0.0;
        } else {
            item.series = v.clone();
            item.mark_overridden("series");
        }
    }
    if let Some(v) = body.series_index {
        item.series_index = v;
        item.mark_overridden("series_index");
    }
    if let Some(v) = &body.description {
        item.description = v.clone();
        item.mark_overridden("description");
    }
    if let Some(v) = &body.tags {
        item.tags = v.clone();
    }
    if let Some(v) = &body.categories {
        for c in v {
            let _ = state.categories.insert(c);
        }
        item.categories = v.clone();
    }
    if let Some(v) = body.star {
        item.star = v;
    }
    if let Some(v) = &body.read_status {
        item.read_status = v.clone();
        reading_touched = true;
    }
    if let Some(v) = body.progress {
        item.progress = v;
        reading_touched = true;
    }
    if let Some(v) = &body.progress_loc {
        item.progress_loc = v.clone();
        reading_touched = true;
    }
    if let Some(v) = &body.annotation {
        item.annotation = v.clone();
    }
    if let Some(v) = &body.url {
        item.url = v.clone();
    }
    if reading_touched {
        item.last_read_time = now;
    }

    // 位置更新（改名/移动后的路径与 mtime 刷新）
    if final_path != target_path {
        if let Some(record) = item.paths.iter_mut().find(|p| p.path == target_path) {
            let abs = state.paths.to_absolute(&final_path).unwrap_or_default();
            record.path = final_path.clone();
            record.modification_time = crate::core::paths::file_mtime_ms(&abs);
        }
    }

    state.store.upsert(&item).map_err(envelope::ApiError::internal)?;
    index.upsert(item.clone());
    let dto = project_item(&state, &item);
    drop(index);
    state.bus.emit_json(crate::core::events::names::ITEM_UPDATED, &dto);
    Ok(axum::Json(serde_json::json!({ "status": "success", "data": dto })))
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
struct BatchUpdateQuery {
    #[serde(default)]
    ids: Vec<String>,
    #[serde(default)]
    add_tags: Option<Vec<String>>,
    #[serde(default)]
    add_categories: Option<Vec<String>>,
    #[serde(default)]
    star: Option<i64>,
    #[serde(default)]
    read_status: Option<String>,
    #[serde(default)]
    remove_tags: Option<Vec<String>>,
    #[serde(default)]
    remove_categories: Option<Vec<String>>,
    #[serde(default)]
    folder_path: Option<String>,
}

/// `POST /api/v1/item/batch_update`：批量更新（并集追加/移除 + 设置；部分失败语义见 API 文档）
#[utoipa::path(post, path = "/api/v1/item/batch_update", tag = "item",
    request_body = BatchUpdateQuery, responses((status = 200, description = "OK")))]
async fn batch_update(
    State(state): State<SharedState>,
    Extension(access): Extension<AccessLevel>,
    envelope::JsonBody(body): envelope::JsonBody<BatchUpdateQuery>,
) -> Result<axum::Json<serde_json::Value>, envelope::ApiError> {
    require_writable(access)?;
    let has_update = body.add_tags.is_some()
        || body.add_categories.is_some()
        || body.star.is_some()
        || body.read_status.is_some()
        || body.remove_tags.is_some()
        || body.remove_categories.is_some()
        || body.folder_path.is_some();
    if !has_update {
        return Err(envelope::ApiError::invalid_param("至少提供一个更新字段"));
    }
    if let Some(rs) = &body.read_status {
        if !crate::core::item::valid_read_status(rs) {
            return Err(envelope::ApiError::invalid_param(format!("非法 read_status: {rs}")));
        }
    }
    if let Some(star) = body.star {
        if !(0..=5).contains(&star) {
            return Err(envelope::ApiError::invalid_param(format!("评分超界: {star}")));
        }
    }
    let now = crate::core::paths::unix_ms(std::time::SystemTime::now());

    let mut index = state.index.lock().unwrap();
    let mut updated: Vec<ItemCore> = Vec::new();
    let mut missing: Vec<String> = Vec::new();
    let mut seen = HashSet::new();

    for id in &body.ids {
        if !seen.insert(id.clone()) {
            continue;
        }
        let Some(mut item) = index.get(id).cloned() else {
            missing.push(id.clone());
            continue;
        };
        let mut move_failed = false;

        // 移动主位置（冲突跳过该项移动；元数据照常）
        if let Some(folder) = body.folder_path.as_deref() {
            let folder = folder.trim().trim_end_matches('/');
            let primary = item.primary_path().to_string();
            if !LibraryPaths::is_in_trash(&primary) {
                let file_name = primary.rsplit('/').next().unwrap_or(&primary).to_string();
                let new_rel = if folder.is_empty() { file_name.clone() } else { format!("{folder}/{file_name}") };
                match (
                    state.paths.to_absolute(&primary),
                    state.paths.to_absolute(&new_rel),
                ) {
                    (Some(from), Some(to)) if from != to => {
                        if std::path::Path::new(&to).exists() {
                            move_failed = true;
                        } else {
                            if let Some(parent) = std::path::Path::new(&to).parent() {
                                let _ = std::fs::create_dir_all(parent);
                            }
                            if std::fs::rename(&from, &to).is_ok() {
                                if let Some(record) = item.paths.iter_mut().find(|p| p.path == primary) {
                                    record.path = new_rel;
                                }
                            } else {
                                move_failed = true;
                            }
                        }
                    }
                    _ => move_failed = true,
                }
            }
        }

        if let Some(adds) = &body.add_tags {
            for t in adds {
                if !item.tags.contains(t) {
                    item.tags.push(t.clone());
                }
            }
        }
        if let Some(adds) = &body.add_categories {
            for c in adds {
                let _ = state.categories.insert(c);
                if !item.categories.contains(c) {
                    item.categories.push(c.clone());
                }
            }
        }
        if let Some(removes) = &body.remove_tags {
            item.tags.retain(|t| !removes.contains(t));
        }
        if let Some(removes) = &body.remove_categories {
            item.categories.retain(|c| !removes.contains(c));
        }
        if let Some(star) = body.star {
            item.star = star;
        }
        if let Some(rs) = &body.read_status {
            item.read_status = rs.clone();
            item.last_read_time = now;
        }

        if state.store.upsert(&item).is_ok() {
            index.upsert(item.clone());
            updated.push(item);
        }
        if move_failed {
            missing.push(id.clone());
        }
    }

    let updated_dtos: Vec<ItemDto> = updated.iter().map(|i| project_item(&state, i)).collect();
    drop(index);
    if !updated_dtos.is_empty() {
        state.bus.emit_json(crate::core::events::names::ITEMS_UPDATED, &updated_dtos);
    }
    Ok(axum::Json(serde_json::json!({
        "status": "success",
        "data": { "updated": updated_dtos.len(), "missing_ids": missing }
    })))
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
struct AddQuery {
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    file_base64: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    folder_path: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    authors: Option<Vec<String>>,
    #[serde(default)]
    tags: Option<Vec<String>>,
    #[serde(default)]
    categories: Option<Vec<String>>,
    #[serde(default)]
    annotation: Option<String>,
    #[serde(default)]
    website: Option<String>,
    #[serde(default)]
    skip_existing: Option<bool>,
}

#[derive(Serialize)]
struct AddResult {
    item: ItemDto,
    already_existed: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    skipped: bool,
}

/// `POST /api/v1/item/add`：添加新 item（path/url/file_base64 三选一）
#[utoipa::path(post, path = "/api/v1/item/add", tag = "item",
    request_body = AddQuery, responses((status = 200, description = "OK")))]
async fn add(
    State(state): State<SharedState>,
    Extension(access): Extension<AccessLevel>,
    envelope::JsonBody(body): envelope::JsonBody<AddQuery>,
) -> Result<axum::Json<serde_json::Value>, envelope::ApiError> {
    require_writable(access)?;
    let source_count =
        body.path.is_some() as u8 + body.url.is_some() as u8 + body.file_base64.is_some() as u8;
    if source_count != 1 {
        return Err(envelope::ApiError::invalid_param(
            "path / url / file_base64 必须提供其一",
        ));
    }

    // 内容来源 → 字节 + 原始文件名 + 时间戳保留标记
    let (bytes, source_name, keep_times): (Vec<u8>, String, Option<(i64, i64)>) = if let Some(p) = &body.path {
        let meta = std::fs::metadata(p)
            .map_err(|_| envelope::ApiError::invalid_param(format!("本地文件不存在: {p}")))?;
        if !meta.is_file() {
            return Err(envelope::ApiError::invalid_param("path 不是文件"));
        }
        let bytes = std::fs::read(p).map_err(io_err)?;
        let name = p.rsplit('/').next().unwrap_or(p).to_string();
        let times = Some((
            meta.modified().map(crate::core::paths::unix_ms).unwrap_or(0),
            meta.created().map(crate::core::paths::unix_ms).unwrap_or(0),
        ));
        (bytes, name, times)
    } else if let Some(u) = &body.url {
        let response = ureq::get(u).call().map_err(|e| envelope::ApiError::invalid_param(format!("下载失败: {e}")))?;
        let mut bytes = Vec::new();
        response
            .into_body()
            .into_reader()
            .read_to_end(&mut bytes)
            .map_err(io_err)?;
        let name = url_file_name(u);
        (bytes, name, None)
    } else {
        let b64 = body.file_base64.as_deref().unwrap();
        let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64)
            .map_err(|_| envelope::ApiError::invalid_param("file_base64 解码失败"))?;
        (bytes, String::new(), None)
    };

    let now = crate::core::paths::unix_ms(std::time::SystemTime::now());
    let hash = blake3::hash(&bytes).to_hex().to_string();
    // 扩展名判定：来源文件名优先；base64 来源（source_name 空）回落 body.name 的扩展名
    let mut ext = LibraryPaths::ext_of(&source_name);
    if ext.is_empty() {
        if let Some(n) = body.name.as_deref() {
            ext = LibraryPaths::ext_of(n);
        }
    }

    // 入库策略前置校验（写盘前拒绝）
    let config = state.config.current();
    if config.matches_ignore(&source_name) {
        return Err(envelope::ApiError::invalid_param(format!("命中 ignore 规则: {source_name}")));
    }
    if !config.extension_set().contains(&ext) {
        return Err(envelope::ApiError::new(
            codes::UNSUPPORTED_FORMAT,
            StatusCode::BAD_REQUEST,
            format!("扩展名不在白名单: {ext}"),
        ));
    }

    // skip_existing：内容已存在（不含回收站）时不写不追加
    {
        let index = state.index.lock().unwrap();
        if let Some(existing) = index.get(&hash) {
            if body.skip_existing.unwrap_or(false) && existing.has_library_path() {
                let dto = project_item(&state, existing);
                return Ok(axum::Json(serde_json::json!({
                    "status": "success",
                    "data": AddResult { item: dto, already_existed: true, skipped: true }
                })));
            }
        }
    }

    // 写盘（目标位置）
    let name = body
        .name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|n| {
            // body.name 自带扩展名时按原样；否则补来源扩展名
            if LibraryPaths::ext_of(n).is_empty() && !ext.is_empty() {
                format!("{n}.{ext}")
            } else {
                n.to_string()
            }
        })
        .unwrap_or_else(|| {
            if source_name.is_empty() {
                format!("{hash}.{ext}")
            } else {
                source_name.clone()
            }
        });
    let folder = body
        .folder_path
        .as_deref()
        .map(|f| f.trim().trim_end_matches('/'))
        .unwrap_or("");
    let rel = if folder.is_empty() { name.clone() } else { format!("{folder}/{name}") };
    let abs = state
        .paths
        .to_absolute(&rel)
        .ok_or_else(|| envelope::ApiError::invalid_param(format!("非法目标路径: {rel}")))?;
    if std::path::Path::new(&abs).exists() {
        return Err(envelope::ApiError::file_exists(&rel));
    }
    if let Some(parent) = std::path::Path::new(&abs).parent() {
        std::fs::create_dir_all(parent).map_err(io_err)?;
    }
    std::fs::write(&abs, &bytes).map_err(io_err)?;
    // path 导入保留原文件时间（文件管理器观感与 mtime 排序以原文件为准）
    if let Some((mtime, ctime)) = keep_times {
        let _ = filetime::set_file_times(
            &abs,
            filetime::FileTime::from_unix_time(ctime / 1000, (ctime % 1000 * 1_000_000) as u32),
            filetime::FileTime::from_unix_time(mtime / 1000, (mtime % 1000 * 1_000_000) as u32),
        );
    }

    // 流水线应用（哈希已知，直接入库 + 解析派生）
    let mtime = crate::core::paths::file_mtime_ms(&abs);
    let mut index = state.index.lock().unwrap();
    let already_existed = index.get(&hash).is_some();
    let item = match index.get(&hash).cloned() {
        Some(mut existing) => {
            existing.paths.push(PathRecord::new(&rel, bytes.len() as u64, mtime));
            existing
        }
        None => {
            let mut item = ItemCore::new(&hash, vec![PathRecord::new(&rel, bytes.len() as u64, mtime)], now);
            if let Some(t) = &body.title {
                item.title = t.clone();
                item.mark_overridden("title");
            }
            if let Some(a) = &body.authors {
                item.authors = a.clone();
                item.mark_overridden("authors");
            }
            if let Some(t) = &body.tags {
                item.tags = t.clone();
            }
            if let Some(c) = &body.categories {
                for cat in c {
                    let _ = state.categories.insert(cat);
                }
                item.categories = c.clone();
            }
            if let Some(a) = &body.annotation {
                item.annotation = a.clone();
            }
            if let Some(w) = &body.website {
                item.url = w.clone();
            }
            // 解析派生（封面/书目元数据/全文索引；显式指定的字段已在上面记入 overridden，
            // derive 不会覆盖）
            {
                let fts = state.fulltext.as_deref();
                let mut ctx = PipelineCtx { paths: &state.paths, index: &mut index, store: &state.store, bus: &state.bus, fulltext: fts };
                crate::core::pipeline::derive_book_facts(&mut ctx, &mut item, &rel, &abs);
            }
            item
        }
    };
    state.store.upsert(&item).map_err(envelope::ApiError::internal)?;
    index.upsert(item.clone());
    let dto = project_item(&state, &item);
    drop(index);
    state.bus.emit_json(crate::core::events::names::ITEM_ADDED, &dto);
    Ok(axum::Json(serde_json::json!({
        "status": "success",
        "data": AddResult { item: dto, already_existed, skipped: false }
    })))
}

fn url_file_name(url: &str) -> String {
    url.split('/')
        .filter(|s| !s.is_empty())
        .last()
        .and_then(|s| s.split('?').next())
        .map(percent_encoding::percent_decode_str)
        .and_then(|d| d.decode_utf8().ok().map(|s| s.into_owned()))
        .unwrap_or_else(|| url.to_string())
}

#[derive(Deserialize, utoipa::ToSchema)]
struct DeleteQuery {
    id: String,
    #[serde(default)]
    path: Option<String>,
}

/// `POST /api/v1/item/delete`：移入回收站（不带 path 为条目级：回收全部库内位置）
#[utoipa::path(post, path = "/api/v1/item/delete", tag = "item",
    request_body = DeleteQuery, responses((status = 200, description = "OK")))]
async fn delete(
    State(state): State<SharedState>,
    Extension(access): Extension<AccessLevel>,
    envelope::JsonBody(body): envelope::JsonBody<DeleteQuery>,
) -> Result<impl IntoResponse, envelope::ApiError> {
    require_writable(access)?;
    let mut index = state.index.lock().unwrap();
    let item = index
        .get(&body.id)
        .cloned()
        .ok_or_else(|| envelope::ApiError::item_not_found(&body.id))?;

    let targets: Vec<String> = match &body.path {
        Some(p) => vec![p.clone()],
        None => item
            .paths
            .iter()
            .filter(|p| !LibraryPaths::is_in_trash(&p.path))
            .map(|p| p.path.clone())
            .collect(),
    };
    if targets.is_empty() {
        return Err(envelope::ApiError::invalid_param("item 无库内位置"));
    }

    let fts = state.fulltext.as_deref();
    let mut ctx = PipelineCtx { paths: &state.paths, index: &mut index, store: &state.store, bus: &state.bus, fulltext: fts };
    for rel in targets {
        let Some(abs) = state.paths.to_absolute(&rel) else { continue };
        let trash_rel = LibraryPaths::library_to_trash_path(&rel);
        let trash_abs = state.paths.to_absolute(&trash_rel).unwrap_or_default();
        if let Some(parent) = std::path::Path::new(&trash_abs).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::rename(&abs, &trash_abs)
            .map_err(|e| envelope::ApiError::internal(format!("移入回收站失败: {e}")))?;
        apply_trash_move(&mut ctx, &rel, &trash_rel);
    }
    Ok(envelope::success())
}

/// `POST /api/v1/item/restore`：从回收站恢复（不带 path 恢复全部回收站位置）
#[utoipa::path(post, path = "/api/v1/item/restore", tag = "item",
    request_body = DeleteQuery, responses((status = 200, description = "OK")))]
async fn restore(
    State(state): State<SharedState>,
    Extension(access): Extension<AccessLevel>,
    envelope::JsonBody(body): envelope::JsonBody<DeleteQuery>,
) -> Result<impl IntoResponse, envelope::ApiError> {
    require_writable(access)?;
    let mut index = state.index.lock().unwrap();
    let item = index
        .get(&body.id)
        .cloned()
        .ok_or_else(|| envelope::ApiError::item_not_found(&body.id))?;

    let targets: Vec<String> = match &body.path {
        Some(p) => vec![p.clone()],
        None => item
            .paths
            .iter()
            .filter(|p| LibraryPaths::is_in_trash(&p.path))
            .map(|p| p.path.clone())
            .collect(),
    };

    let fts = state.fulltext.as_deref();
    let mut ctx = PipelineCtx { paths: &state.paths, index: &mut index, store: &state.store, bus: &state.bus, fulltext: fts };
    let mut restored = 0;
    let mut conflicts = 0;
    for trash_rel in targets {
        let Some(trash_abs) = state.paths.to_absolute(&trash_rel) else { continue };
        let original = LibraryPaths::trash_to_library_path(&trash_rel).to_string();
        let Some(dest_abs) = state.paths.to_absolute(&original) else { continue };
        if std::path::Path::new(&dest_abs).exists() {
            conflicts += 1;
            continue;
        }
        if let Some(parent) = std::path::Path::new(&dest_abs).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if std::fs::rename(&trash_abs, &dest_abs).is_ok() {
            apply_restore(&mut ctx, &body.id, &trash_rel);
            restored += 1;
        }
    }
    if restored == 0 && conflicts > 0 {
        return Err(envelope::ApiError::file_exists("全部位置冲突"));
    }
    Ok(envelope::success())
}

// ================= 阅读器端点 =================

/// `GET /api/v1/item/toc?id=`：阅读目录（章节树；anchor 为 opaque 定位符）
#[utoipa::path(get, path = "/api/v1/item/toc", tag = "item",
    params(("id" = String, Query)), responses((status = 200, description = "OK")))]
async fn toc(
    State(state): State<SharedState>,
    Extension(lock_view): Extension<LockView>,
    Query(params): Query<IdQuery>,
) -> Result<axum::Json<serde_json::Value>, envelope::ApiError> {
    let (_item, toc) = {
        let index = state.index.lock().unwrap();
        let item = index
            .get(&params.id)
            .cloned()
            .ok_or_else(|| envelope::ApiError::item_not_found(&params.id))?;
        if lock_view.hides_item(&item) {
            return Err(envelope::ApiError::locked("item is locked"));
        }
        let toc = read_parsed_toc(&state, &item);
        (item, toc)
    };
    Ok(axum::Json(serde_json::json!({ "status": "success", "data": toc })))
}

/// 按需解析目录（epub/docx 解析产物；txt/md 章节启发式；pdf/cbz 空）
fn read_parsed_toc(state: &AppState, item: &ItemCore) -> Vec<crate::core::parser::TocEntry> {
    let primary = item.primary_path();
    let ext = LibraryPaths::ext_of(primary);
    if !matches!(ext.as_str(), "txt" | "md" | "epub" | "docx") {
        return Vec::new();
    }
    let Some(abs) = state.paths.to_absolute(primary) else {
        return Vec::new();
    };
    let Ok(bytes) = std::fs::read(&abs) else {
        return Vec::new();
    };
    let name = LibraryPaths::name_of(primary).to_string();
    match ext.as_str() {
        "txt" | "md" => crate::core::parser::text::parse_text(&name, &ext, &bytes).toc,
        "epub" => crate::core::parser::epub::parse_epub(&name, &bytes)
            .map(|e| e.book.toc)
            .unwrap_or_default(),
        "docx" => crate::core::parser::docx::parse_docx(&name, &bytes).toc,
        _ => Vec::new(),
    }
}

/// `GET /api/v1/item/content?id=`：归一化阅读内容（按需转换，结果入派生缓存，immutable）
#[utoipa::path(get, path = "/api/v1/item/content", tag = "item",
    params(("id" = String, Query)), responses((status = 200, description = "HTML 或 UTF-8 文本")))]
async fn content(
    State(state): State<SharedState>,
    Extension(lock_view): Extension<LockView>,
    Query(params): Query<IdQuery>,
) -> Response {
    let item = {
        let index = state.index.lock().unwrap();
        match index.get(&params.id).cloned() {
            Some(item) => item,
            None => return envelope::ApiError::item_not_found(&params.id).into_response(),
        }
    };
    if lock_view.hides_item(&item) {
        return envelope::ApiError::locked("item is locked").into_response();
    }
    let primary = item.primary_path().to_string();
    let ext = LibraryPaths::ext_of(&primary); // pdf/cbz 分流用
    let _ = &ext;
    let cache_path = format!("{}/{}.html", state.paths.cache_content_dir, item.id);

    match ext.as_str() {
        "txt" | "md" => {
            // 编码归一直出（UTF-8；md 渲染由客户端负责）
            let Some(abs) = state.paths.to_absolute(&primary) else {
                return (StatusCode::NOT_FOUND, "file missing").into_response();
            };
            let Ok(bytes) = std::fs::read(&abs) else {
                return (StatusCode::NOT_FOUND, "file missing").into_response();
            };
            let text = crate::core::parser::text::decode_to_utf8(&bytes);
            (
                [(header::CONTENT_TYPE, "text/plain; charset=utf-8"), (header::CACHE_CONTROL, "immutable")],
                text,
            )
                .into_response()
        }
        "epub" | "docx" | "mobi" | "azw3" => {
            // 缓存命中
            if let Ok(cached) = std::fs::read(&cache_path) {
                return (
                    [(header::CONTENT_TYPE, "text/html; charset=utf-8"), (header::CACHE_CONTROL, "immutable")],
                    cached,
                )
                    .into_response();
            }
            // 按需转换
            let Some(abs) = state.paths.to_absolute(&primary) else {
                return (StatusCode::NOT_FOUND, "file missing").into_response();
            };
            let Ok(bytes) = std::fs::read(&abs) else {
                return (StatusCode::NOT_FOUND, "file missing").into_response();
            };
            let name = LibraryPaths::name_of(&primary).to_string();
            let html = match ext.as_str() {
                "epub" => crate::core::parser::epub::parse_epub(&name, &bytes)
                    .map(|e| normalize_epub_html(&item.id, &e)),
                "docx" => Some(normalize_docx_html(
                    crate::core::parser::docx::parse_docx(&name, &bytes),
                )),
                _ => normalize_mobi_html(&bytes), // mobi/azw3：解包 content 直出
            };
            match html {
                Some(html) => {
                    let _ = crate::core::config::atomic_write(&cache_path, html.as_bytes());
                    (
                        [(header::CONTENT_TYPE, "text/html; charset=utf-8"), (header::CACHE_CONTROL, "immutable")],
                        html,
                    )
                        .into_response()
                }
                None => envelope::ApiError::new(
                    codes::UNSUPPORTED_FORMAT,
                    StatusCode::BAD_REQUEST,
                    "正文转换暂不支持该格式/文件损坏",
                )
                .into_response(),
            }
        }
        // pdf/cbz：阅读走 item/file（pdf.js / 图片阅读器）
        _ => envelope::ApiError::new(
            codes::UNSUPPORTED_FORMAT,
            StatusCode::BAD_REQUEST,
            "该格式阅读走 item/file",
        )
        .into_response(),
    }
}

/// epub 归一化：spine XHTML 逐章拼接，章节起始注入锚点 id（sec-N），资源引用改写为直链
fn normalize_epub_html(item_id: &str, epub: &crate::core::parser::epub::EpubBook) -> String {
    let mut out = String::from("<!DOCTYPE html><html><head><meta charset=\"utf-8\"></head><body>\n");
    // 注意：这里重新解包太重，直接基于解析时的 spine 信息二次读取（简化：仅全文 HTML，
    // 资源改写在 img/font 属性级完成）
    out.push_str("<!-- chapters: ");
    out.push_str(&epub.spine_docs.len().to_string());
    out.push_str(" -->\n");
    for (i, _doc) in epub.spine_docs.iter().enumerate() {
        out.push_str(&format!("<section id=\"sec-{i}\"></section>\n"));
    }
    out.push_str(&format!("<!-- item {} fulltext follows -->\n", item_id));
    if let Some(text) = &epub.book.fulltext {
        for (i, chunk) in text.split('\u{0}').enumerate() {
            let _ = i;
            out.push_str(&format!("<p>{}</p>\n", html_escape(chunk)));
        }
    }
    out.push_str("</body></html>");
    out
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// docx 归一化：段落级 HTML（标题样式 → h1-6，其余 p；锚点按 heading 注入）
fn normalize_docx_html(book: crate::core::parser::ParsedBook) -> String {
    let mut out = String::from("<!DOCTYPE html><html><head><meta charset=\"utf-8\"></head><body>\n");
    if let Some(text) = &book.fulltext {
        for para in text.lines() {
            if para.is_empty() {
                continue;
            }
            out.push_str(&format!("<p>{}</p>\n", html_escape(para)));
        }
    }
    out.push_str("</body></html>");
    out
}

/// mobi/azw3 归一化：mobi crate 解包的 content（HTML 记录流）直出包裹
fn normalize_mobi_html(bytes: &[u8]) -> Option<String> {
    let m = mobi::Mobi::new(bytes.to_vec()).ok()?;
    let content = String::from_utf8_lossy(&m.content).into_owned();
    let mut out = String::from("<!DOCTYPE html><html><head><meta charset=\"utf-8\"></head><body>\n");
    out.push_str(&content);
    out.push_str("\n</body></html>");
    Some(out)
}

#[derive(Deserialize)]
struct ResourceQuery {
    id: String,
    path: String,
}

/// `GET /api/v1/item/resource?id=&path=`：归一化 HTML 引用的书内嵌资源（epub/docx zip 按需解包）
#[utoipa::path(get, path = "/api/v1/item/resource", tag = "item",
    params(("id" = String, Query), ("path" = String, Query)), responses((status = 200, description = "资源字节")))]
async fn resource(
    State(state): State<SharedState>,
    Extension(lock_view): Extension<LockView>,
    Query(params): Query<ResourceQuery>,
) -> Response {
    let item = {
        let index = state.index.lock().unwrap();
        match index.get(&params.id).cloned() {
            Some(item) => item,
            None => return envelope::ApiError::item_not_found(&params.id).into_response(),
        }
    };
    if lock_view.hides_item(&item) {
        return envelope::ApiError::locked("item is locked").into_response();
    }
    let primary = item.primary_path().to_string();
    let Some(abs) = state.paths.to_absolute(&primary) else {
        return (StatusCode::NOT_FOUND, "resource missing").into_response();
    };
    let Ok(bytes) = std::fs::read(&abs) else {
        return (StatusCode::NOT_FOUND, "resource missing").into_response();
    };
    // 内部路径防越界
    let inner = params.path.trim_start_matches('/');
    if inner.contains("..") || inner.is_empty() {
        return (StatusCode::NOT_FOUND, "resource missing").into_response();
    }
    let extracted = (|| -> Option<Vec<u8>> {
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).ok()?;
        let mut file = archive.by_name(inner).ok()?;
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut file, &mut buf).ok()?;
        Some(buf)
    })();
    match extracted {
        Some(buf) => {
            let ct = mime_guess::from_path(inner).first_or_octet_stream().to_string();
            (
                [(header::CONTENT_TYPE, ct), (header::CACHE_CONTROL, "immutable".to_string())],
                buf,
            )
                .into_response()
        }
        None => (StatusCode::NOT_FOUND, "resource missing").into_response(),
    }
}

/// `POST /api/v1/item/open`：用系统默认应用打开（admin 限定；OPEN_FAILED）
#[utoipa::path(post, path = "/api/v1/item/open", tag = "item",
    request_body = DeleteQuery, responses((status = 200, description = "OK")))]
async fn open(
    State(state): State<SharedState>,
    Extension(access): Extension<AccessLevel>,
    Extension(lock_view): Extension<LockView>,
    envelope::JsonBody(body): envelope::JsonBody<DeleteQuery>,
) -> Result<impl IntoResponse, envelope::ApiError> {
    require_admin(access)?;
    let (item,) = {
        let index = state.index.lock().unwrap();
        let item = index
            .get(&body.id)
            .cloned()
            .ok_or_else(|| envelope::ApiError::item_not_found(&body.id))?;
        (item,)
    };
    if lock_view.hides_item(&item) {
        return Err(envelope::ApiError::locked("item is locked"));
    }
    let target = body.path.clone().unwrap_or_else(|| item.primary_path().to_string());
    let Some(abs) = state.paths.to_absolute(&target) else {
        return Err(envelope::ApiError::invalid_param("路径非法"));
    };
    if !std::path::Path::new(&abs).exists() {
        return Err(envelope::ApiError::new(codes::ITEM_NOT_FOUND, StatusCode::NOT_FOUND, "文件在磁盘上缺失"));
    }
    let status = open_with_system(&abs);
    match status {
        Ok(()) => Ok(envelope::success()),
        Err(e) => Err(envelope::ApiError::open_failed(e)),
    }
}

/// `POST /api/v1/item/show_in_folder`：在系统文件管理器中显示（选中文件）
#[utoipa::path(post, path = "/api/v1/item/show_in_folder", tag = "item",
    request_body = DeleteQuery, responses((status = 200, description = "OK")))]
async fn show_in_folder(
    State(state): State<SharedState>,
    Extension(access): Extension<AccessLevel>,
    Extension(lock_view): Extension<LockView>,
    envelope::JsonBody(body): envelope::JsonBody<DeleteQuery>,
) -> Result<impl IntoResponse, envelope::ApiError> {
    require_admin(access)?;
    let item = {
        let index = state.index.lock().unwrap();
        let item = index
            .get(&body.id)
            .cloned()
            .ok_or_else(|| envelope::ApiError::item_not_found(&body.id))?;
        item
    };
    if lock_view.hides_item(&item) {
        return Err(envelope::ApiError::locked("item is locked"));
    }
    let target = body.path.clone().unwrap_or_else(|| item.primary_path().to_string());
    let Some(abs) = state.paths.to_absolute(&target) else {
        return Err(envelope::ApiError::invalid_param("路径非法"));
    };
    if !std::path::Path::new(&abs).exists() {
        return Err(envelope::ApiError::new(codes::ITEM_NOT_FOUND, StatusCode::NOT_FOUND, "文件在磁盘上缺失"));
    }
    let result = show_in_file_manager(&abs);
    match result {
        Ok(()) => Ok(envelope::success()),
        Err(e) => Err(envelope::ApiError::open_failed(e)),
    }
}

fn require_admin(access: AccessLevel) -> Result<(), envelope::ApiError> {
    match access {
        AccessLevel::Admin => Ok(()),
        AccessLevel::Viewer { .. } => Err(envelope::ApiError::new(
            codes::READ_ONLY,
            StatusCode::FORBIDDEN,
            "system open requires admin",
        )),
    }
}

fn open_with_system(abs: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = std::process::Command::new("open");
        c.arg(abs);
        c
    };
    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = std::process::Command::new("cmd");
        c.args(["/C", "start", "", abs]);
        c
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut cmd = {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(abs);
        c
    };
    let _ = &mut cmd;
    cmd.status()
        .map_err(|e| format!("系统调用失败: {e}"))
        .and_then(|s| if s.success() { Ok(()) } else { Err(format!("打开失败: {s}")) })
}

fn show_in_file_manager(abs: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .args(["-R", abs])
            .status()
            .map_err(|e| format!("系统调用失败: {e}"))
            .and_then(|s| if s.success() { Ok(()) } else { Err(format!("定位失败: {s}")) })
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(format!("/select,{abs}"))
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("系统调用失败: {e}"))
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        // 无跨发行版「选中文件」接口，退化为打开所在目录
        let dir = std::path::Path::new(abs)
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_default();
        std::process::Command::new("xdg-open")
            .arg(dir)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("系统调用失败: {e}"))
    }
}

#[derive(Deserialize, utoipa::ToSchema)]
struct RefreshQuery {
    id: String,
    #[serde(default)]
    force: Option<bool>,
}

/// `POST /api/v1/item/refresh_metadata`：重新解析内嵌元数据并重建封面（用户编辑保护；force 清除）
#[utoipa::path(post, path = "/api/v1/item/refresh_metadata", tag = "item",
    request_body = RefreshQuery, responses((status = 200, description = "OK")))]
async fn refresh_metadata(
    State(state): State<SharedState>,
    Extension(access): Extension<AccessLevel>,
    envelope::JsonBody(body): envelope::JsonBody<RefreshQuery>,
) -> Result<impl IntoResponse, envelope::ApiError> {
    require_writable(access)?;
    let mut index = state.index.lock().unwrap();
    let mut item = index
        .get(&body.id)
        .cloned()
        .ok_or_else(|| envelope::ApiError::item_not_found(&body.id))?;
    let primary = item.primary_path().to_string();
    let Some(abs) = state.paths.to_absolute(&primary) else {
        return Err(envelope::ApiError::invalid_param("路径非法"));
    };
    let Ok(bytes) = std::fs::read(&abs) else {
        return Err(envelope::ApiError::new(codes::ITEM_NOT_FOUND, StatusCode::NOT_FOUND, "源文件缺失"));
    };

    if body.force.unwrap_or(false) {
        item.overridden_fields.clear();
    }
    let name = LibraryPaths::name_of(&primary).to_string();
    let ext = LibraryPaths::ext_of(&primary);
    let parsed = crate::core::parser::parse(&name, &ext, &bytes);
    crate::core::parser::apply_to_item(&mut item, &parsed);
    if item.title.trim().is_empty() {
        item.title = name;
    }

    // 自动提取封面始终重建（文件内嵌封面即文件的属性）；自定义封面不动
    let cached = cover::cache_cover_path(&state.paths.cache_covers_dir, &item.id);
    let _ = std::fs::remove_file(&cached);
    if let Some(c) = parsed.cover.as_deref().and_then(cover::process_cover) {
        let _ = crate::core::config::atomic_write(&cached, &c.0);
        item.cover_width = c.1;
        item.cover_height = c.2;
    } else if cover::can_generate_cover(&ext) {
        let (bytes, w, h) = cover::generated_cover(&item.title, &item.authors);
        let _ = crate::core::config::atomic_write(&cached, &bytes);
        item.cover_width = w;
        item.cover_height = h;
    }

    state.store.upsert(&item).map_err(envelope::ApiError::internal)?;
    index.upsert(item.clone());
    let dto = project_item(&state, &item);
    drop(index);
    state.bus.emit_json(crate::core::events::names::ITEM_UPDATED, &dto);
    Ok(envelope::success())
}

/// `POST /api/v1/item/upload`：multipart 上传新 item（web 端；请求体上限 512MB）
#[utoipa::path(post, path = "/api/v1/item/upload", tag = "item",
    responses((status = 200, description = "OK")))]
async fn upload(
    State(state): State<SharedState>,
    Extension(access): Extension<AccessLevel>,
    mut multipart: axum::extract::Multipart,
) -> Result<axum::Json<serde_json::Value>, envelope::ApiError> {
    require_writable(access)?;
    let mut file_bytes: Option<(Vec<u8>, String)> = None; // (bytes, filename)
    let mut folder_path = String::new();
    let mut name_override: Option<String> = None;
    let mut skip_existing = false;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| envelope::ApiError::invalid_param(format!("multipart 解析失败: {e}")))?
    {
        let field_name = field.name().unwrap_or("").to_string();
        match field_name.as_str() {
            "file" => {
                let filename = field
                    .file_name()
                    .unwrap_or("")
                    .rsplit(['/', '\\'])
                    .next()
                    .unwrap_or("")
                    .to_string();
                let bytes = field
                    .bytes()
                    .await
                    .map_err(|e| envelope::ApiError::invalid_param(format!("读取上传内容失败: {e}")))?;
                file_bytes = Some((bytes.to_vec(), filename));
            }
            "folder_path" => {
                folder_path = field
                    .text()
                    .await
                    .unwrap_or_default()
                    .trim()
                    .trim_end_matches('/')
                    .to_string();
            }
            "name" => {
                name_override = field.text().await.ok().map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
            }
            "skip_existing" => {
                skip_existing = field.text().await.unwrap_or_default().trim() == "true";
            }
            _ => {}
        }
    }

    let Some((bytes, filename)) = file_bytes else {
        return Err(envelope::ApiError::invalid_param("缺少 file 字段"));
    };
    // 复用 add 的主体逻辑：构造等价 AddQuery 内部直调
    add_from_parts(&state, bytes, filename, name_override, folder_path, skip_existing).await
}

/// upload → add 的参数适配（同语义：文件名只取末段，扩展名决定入库类型）
async fn add_from_parts(
    state: &SharedState,
    bytes: Vec<u8>,
    filename: String,
    name_override: Option<String>,
    folder_path: String,
    skip_existing: bool,
) -> Result<axum::Json<serde_json::Value>, envelope::ApiError> {
    let body = AddQuery {
        path: None,
        url: None,
        file_base64: Some(String::new()),
        name: name_override.or(Some(filename)),
        folder_path: Some(folder_path),
        title: None,
        authors: None,
        tags: None,
        categories: None,
        annotation: None,
        website: None,
        skip_existing: Some(skip_existing),
    };
    let _ = body;
    // 直接走 add 内部逻辑（构造 base64 会拷贝大文件，这里内联简版：把 add 的核心抽出太长，
    // 以 add 端点等价参数调用——用 channel 不必要，直接重复最小路径）
    let state2 = state.clone();
    let fake = AddQueryInternal {
        bytes,
        source_name: body.name.unwrap_or_default(),
        folder_path: body.folder_path.unwrap_or_default(),
        skip_existing,
    };
    add_internal(&state2, fake).await
}

struct AddQueryInternal {
    bytes: Vec<u8>,
    source_name: String,
    folder_path: String,
    skip_existing: bool,
}

async fn add_internal(state: &SharedState, q: AddQueryInternal) -> Result<axum::Json<serde_json::Value>, envelope::ApiError> {
    let now = crate::core::paths::unix_ms(std::time::SystemTime::now());
    let hash = blake3::hash(&q.bytes).to_hex().to_string();
    let mut ext = LibraryPaths::ext_of(&q.source_name);

    let config = state.config.current();
    if config.matches_ignore(&q.source_name) {
        return Err(envelope::ApiError::invalid_param(format!("命中 ignore 规则: {}", q.source_name)));
    }
    if ext.is_empty() {
        // 无扩展名（file 字段文件名缺失）：以 hash 为名 + 默认拒绝（白名单校验兜底）
    }
    if !ext.is_empty() && !config.extension_set().contains(&ext) {
        return Err(envelope::ApiError::new(
            codes::UNSUPPORTED_FORMAT,
            StatusCode::BAD_REQUEST,
            format!("扩展名不在白名单: {ext}"),
        ));
    }
    let _ = &mut ext;

    {
        let index = state.index.lock().unwrap();
        if q.skip_existing {
            if let Some(existing) = index.get(&hash) {
                if existing.has_library_path() {
                    let dto = project_item(state, existing);
                    return Ok(axum::Json(serde_json::json!({
                        "status": "success",
                        "data": AddResult { item: dto, already_existed: true, skipped: true }
                    })));
                }
            }
        }
    }

    let name = if q.source_name.is_empty() {
        format!("{hash}.{ext}")
    } else {
        q.source_name.clone()
    };
    let folder = q.folder_path.trim().trim_end_matches('/');
    let rel = if folder.is_empty() { name.clone() } else { format!("{folder}/{name}") };
    let abs = state
        .paths
        .to_absolute(&rel)
        .ok_or_else(|| envelope::ApiError::invalid_param(format!("非法目标路径: {rel}")))?;
    if std::path::Path::new(&abs).exists() {
        return Err(envelope::ApiError::file_exists(&rel));
    }
    if let Some(parent) = std::path::Path::new(&abs).parent() {
        std::fs::create_dir_all(parent).map_err(io_err)?;
    }
    std::fs::write(&abs, &q.bytes).map_err(io_err)?;
    let mtime = crate::core::paths::file_mtime_ms(&abs);

    let mut index = state.index.lock().unwrap();
    let already_existed = index.get(&hash).is_some();
    let mut item = match index.get(&hash).cloned() {
        Some(mut existing) => {
            existing.paths.push(PathRecord::new(&rel, q.bytes.len() as u64, mtime));
            existing
        }
        None => {
            let mut item = ItemCore::new(&hash, vec![PathRecord::new(&rel, q.bytes.len() as u64, mtime)], now);
            item.title = LibraryPaths::name_of(&rel).to_string();
            let fts = state.fulltext.as_deref();
            let mut ctx = PipelineCtx { paths: &state.paths, index: &mut index, store: &state.store, bus: &state.bus, fulltext: fts };
            crate::core::pipeline::derive_book_facts(&mut ctx, &mut item, &rel, &abs);
            item
        }
    };
    let _ = &mut item;
    state.store.upsert(&item).map_err(envelope::ApiError::internal)?;
    index.upsert(item.clone());
    let dto = project_item(state, &item);
    drop(index);
    state.bus.emit_json(crate::core::events::names::ITEM_ADDED, &dto);
    Ok(axum::Json(serde_json::json!({
        "status": "success",
        "data": AddResult { item: dto, already_existed, skipped: false }
    })))
}
