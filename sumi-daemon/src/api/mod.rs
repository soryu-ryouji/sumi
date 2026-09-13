//! 共享应用状态与路由装配。api/ → core/ 单向依赖。

use crate::core::config::LibraryConfig;
use crate::core::events::EventBus;
use crate::core::paths::LibraryPaths;
use crate::core::startup::StartupState;
use crate::core::tasks::TaskTracker;
use crate::settings::Settings;
use axum::response::IntoResponse;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;

pub mod app;
#[cfg(test)]
mod contract_tests;
pub mod envelope;
pub mod events;
pub mod folder;
pub mod item;
pub mod misc;
pub mod taxonomy;

/// 请求级扩展：当前 token 的访问级别，app/info 据此报告。
/// Viewer 携带该 token 的写能力（[web] 的 writable/separate/write_token 共同决定，每请求解析，配置热生效）
#[derive(Clone, Copy)]
pub enum AccessLevel {
    Admin,
    Viewer { writable: bool },
}

pub struct AppState {
    pub settings: Settings,
    // ---- 存储底座：路径布局 / 库配置 / 启动进度 ----
    pub paths: LibraryPaths,
    pub config: Arc<LibraryConfig>,
    pub startup: Arc<StartupState>,
    /// 后台任务积压（app/status 与 task.progress 的同一数据源）
    pub tasks: Arc<TaskTracker>,
    /// SSE 事件总线
    pub bus: EventBus,
    /// SSE 订阅因消费落后被断开（lagged）的累计次数（app/status 观测）
    pub sse_lagged: Arc<AtomicU64>,
    // ---- 索引与存储（写路径经 Mutex<ItemIndex> 单写者）----
    pub index: Arc<std::sync::Mutex<crate::core::index::ItemIndex>>,
    pub store: Arc<crate::core::metadata_store::MetadataStore>,
    /// 全文索引（打开失败为 None：查询退化不命中，可重建）
    pub fulltext: Option<Arc<crate::core::fulltext::FulltextIndex>>,
    // ---- 注册表 ----
    pub categories: Arc<crate::core::registry_file::NameRegistry>,
    pub tags: Arc<crate::core::registry_file::NameRegistry>,
    pub prefs: Arc<crate::core::registry_file::ViewPreferences>,
    pub global_filter: Arc<crate::core::registry_file::GlobalFilter>,
    pub locks: Arc<crate::core::locks::Locks>,
}

pub type SharedState = Arc<AppState>;

/// 重建 OpenAPI 文档并序列化为 pretty JSON（LF 行尾）——--dump-openapi 的输出
/// （固化到 sumi-daemon/openapi.json，契约测试校验同步）
pub fn build_openapi_json() -> String {
    let (_router, mut doc) = api_router();
    doc.info.title = "sumi-daemon | v1".to_string();
    doc.info.version = "1.0.0".to_string();
    doc.servers = Some(vec![utoipa::openapi::Server::new(
        "http://127.0.0.1:27381/",
    )]);
    let mut json = serde_json::to_string_pretty(&doc).expect("OpenAPI 文档序列化失败");
    json.push('\n');
    json
}

/// API 路由与 OpenAPI 文档的同一来源：OpenApiRouter 收集 #[utoipa::path] 标注的端点，
/// split 出路由与文档（文档由 --dump-openapi 固化到 openapi.json）
pub fn api_router() -> (axum::Router<SharedState>, utoipa::openapi::OpenApi) {
    utoipa_axum::router::OpenApiRouter::new()
        .merge(app::routes())
        .merge(events::routes())
        .merge(item::routes())
        .merge(folder::routes())
        .merge(taxonomy::routes())
        .merge(misc::routes())
        .split_for_parts()
}

/// 构建路由：中间件链 CORS → ReadyGate → Auth → Endpoints。
/// ready_gate 在 auth 外层：启动期 /api/* 一律 503 NOT_READY（契约），不因缺 token 变 401
pub fn build_router(state: SharedState) -> axum::Router {
    use axum::routing::get;
    let (api_routes, _doc) = api_router();
    axum::Router::new()
        .route("/health", get(health))
        .merge(api_routes)
        .with_state(state.clone())
        // 请求体上限 512MB（item/upload 契约值）
        .layer(axum::extract::DefaultBodyLimit::max(512 * 1024 * 1024))
        // axum 中后注册的 layer 在外层：请求依次经过 cors → ready_gate → auth
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            ready_gate,
        ))
        .layer(axum::middleware::from_fn(cors))
        // 最外层兜底：handler/中间件 panic 转 500，不拖垮进程
        .layer(tower_http::catch_panic::CatchPanicLayer::new())
}

/// `GET /health`：就绪探活，不带 /api/v1 前缀，无需 token。
/// 初始索引完成前 503，完成后 200。只用于区分进程状态，进度与就绪判断以 app/startup 为准。
async fn health(axum::extract::State(state): axum::extract::State<SharedState>) -> impl axum::response::IntoResponse {
    if state.startup.is_ready() {
        (axum::http::StatusCode::OK, "ok")
    } else {
        (axum::http::StatusCode::SERVICE_UNAVAILABLE, "starting")
    }
}

/// CORS 全放开（localhost 工具，token 兜底）；唯一例外：token 发现端点不带 CORS 头
/// （跨源网页 JS 读不到响应，只有持 host_permissions 的扩展能读）
async fn cors(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::http::header;
    let is_token_discovery = req.uri().path() == "/api/v1/app/token";
    // 预检短路：OPTIONS 不携带凭据、不执行任何操作，直接 204 + 放开头
    if req.method() == axum::http::Method::OPTIONS && !is_token_discovery {
        let mut resp = axum::response::Response::new(axum::body::Body::empty());
        *resp.status_mut() = axum::http::StatusCode::NO_CONTENT;
        let headers = resp.headers_mut();
        headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*".parse().unwrap());
        headers.insert(header::ACCESS_CONTROL_ALLOW_METHODS, "*".parse().unwrap());
        headers.insert(header::ACCESS_CONTROL_ALLOW_HEADERS, "*".parse().unwrap());
        return resp;
    }
    let mut resp = next.run(req).await;
    if !is_token_discovery {
        let headers = resp.headers_mut();
        headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*".parse().unwrap());
        headers.insert(header::ACCESS_CONTROL_ALLOW_METHODS, "*".parse().unwrap());
        headers.insert(header::ACCESS_CONTROL_ALLOW_HEADERS, "*".parse().unwrap());
    }
    resp
}

/// Token 鉴权：/api/* 请求必须携带 Authorization: Bearer <token>；
/// SSE 与直链（cover/file/toc/content/resource）无法设置请求头，改用查询参数 ?token=。
/// 双 token：admin（启动时生成/传入，全权限）与 viewer（config.toml [web]，三档权限）。
/// 例外：GET /api/v1/app/token（token 发现端点）无鉴权，安全性见端点实现。
async fn auth(
    axum::extract::State(state): axum::extract::State<SharedState>,
    mut req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let path = req.uri().path();
    if !path.starts_with("/api") {
        return next.run(req).await;
    }
    if path == "/api/v1/app/token" && req.method() == axum::http::Method::GET {
        return next.run(req).await;
    }

    let access = resolve_access(&state, &req);
    let Some(access) = access else {
        return (
            axum::http::StatusCode::UNAUTHORIZED,
            axum::Json(
                serde_json::json!({ "status": "error", "error": { "code": envelope::codes::UNAUTHORIZED, "message": "missing or invalid token" } }),
            ),
        )
            .into_response();
    };

    if matches!(access, AccessLevel::Viewer { writable: false }) && !is_viewer_allowed(&req) {
        return (
            axum::http::StatusCode::FORBIDDEN,
            axum::Json(
                serde_json::json!({ "status": "error", "error": { "code": envelope::codes::READ_ONLY, "message": "viewer token is read-only" } }),
            ),
        )
            .into_response();
    }

    req.extensions_mut().insert(access);

    // 解锁票据 → 已解锁集合（X-Sumi-Unlock 头；直链端点放行 ?unlock= 查询参数，通道与 token 一致）
    let mut tickets = Vec::new();
    if let Some(hv) = req.headers().get("x-sumi-unlock") {
        if let Ok(v) = hv.to_str() {
            tickets.extend(v.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string));
        }
    }
    let allow_query_unlock = matches!(
        req.uri().path(),
        "/api/v1/events"
            | "/api/v1/item/cover"
            | "/api/v1/item/file"
            | "/api/v1/item/toc"
            | "/api/v1/item/content"
            | "/api/v1/item/resource"
    );
    if allow_query_unlock {
        if let Some(query) = req.uri().query() {
            for pair in query.split('&') {
                if let Some((k, v)) = pair.split_once('=') {
                    if k == "unlock" {
                        tickets.extend(v.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string));
                    }
                }
            }
        }
    }
    let unlocked = state.locks.unlocked_dimensions(&tickets);
    req.extensions_mut().insert(LockView {
        all: state.locks.snapshot_names(),
        unlocked,
    });
    next.run(req).await
}

/// 请求级锁视野：未解锁锁的排除判定（列表过滤与内容直连共用）。
/// all = 全部锁集合，unlocked = 本请求票据已解锁集合；locked = all 中未解锁部分
#[derive(Clone)]
pub struct LockView {
    pub all: crate::core::registry_file::DimensionSet,
    pub unlocked: crate::core::registry_file::DimensionSet,
}

impl LockView {
    fn named_locked(&self, dimension: &str, name: &str) -> bool {
        self.all.named_contains(dimension, name) && !self.unlocked.named_contains(dimension, name)
    }

    fn folder_locked(&self, path: &str) -> bool {
        self.all.folder_hit(path) && !self.unlocked.folder_hit(path)
    }

    pub fn has_any_locks(&self) -> bool {
        !(self.all.folders.is_empty()
            && self.all.categories.is_empty()
            && self.all.tags.is_empty()
            && self.all.authors.is_empty())
    }

    /// 条目是否被未解锁的锁覆盖（OR 剔除：命中任一未解锁锁即不可见。
    /// 位置维度：存在任一无锁或已解锁的非回收站位置即放行；属性维度与位置无关）
    pub fn hides_item(&self, item: &crate::core::item::ItemCore) -> bool {
        if !self.has_any_locks() {
            return false;
        }
        // 属性维度（分类/标签/作者）与位置无关
        if item.categories.iter().any(|c| self.named_locked("category", c))
            || item.tags.iter().any(|t| self.named_locked("tag", t))
            || item.authors.iter().any(|a| self.named_locked("author", a))
        {
            return true;
        }
        // 位置维度：任一非回收站位置「无锁或已解锁」即放行
        let library_paths: Vec<&str> = item
            .paths
            .iter()
            .filter(|p| !crate::core::paths::LibraryPaths::is_in_trash(&p.path))
            .map(|p| p.path.as_str())
            .collect();
        if library_paths.is_empty() {
            return false; // 回收站视图不排除锁
        }
        library_paths.iter().all(|p| self.folder_locked(p))
    }

    /// 主动筛选目标是否命中未解锁锁（item/list 的 folders/categories/tags/authors 参数 → 403）
    pub fn filter_locked(&self, dimension: &str, name: &str) -> bool {
        if dimension == "folder" {
            self.folder_locked(name)
        } else {
            self.named_locked(dimension, name)
        }
    }
}

/// 返回 access 级别（含 viewer 的 per-token 写能力），token 无效返回 None
fn resolve_access(state: &AppState, req: &axum::extract::Request) -> Option<AccessLevel> {
    let headers = req.headers();
    if let Some(auth) = headers.get(axum::http::header::AUTHORIZATION) {
        if let Ok(value) = auth.to_str() {
            if let Some(token) = value.strip_prefix("Bearer ") {
                if token == state.settings.token {
                    return Some(AccessLevel::Admin);
                }
                if let Some(access) = viewer_access(state, token) {
                    return Some(access);
                }
            }
        }
    }

    // EventSource 与 <img>/阅读器直链均无法设置请求头，这些 GET 端点放行查询参数 token
    let path = req.uri().path();
    let allow_query_token = req.method() == axum::http::Method::GET
        && matches!(
            path,
            "/api/v1/events"
                | "/api/v1/item/cover"
                | "/api/v1/item/file"
                | "/api/v1/item/toc"
                | "/api/v1/item/content"
                | "/api/v1/item/resource"
        );
    if allow_query_token {
        if let Some(query) = req.uri().query() {
            let token: Option<&str> = query
                .split('&')
                .filter_map(|pair| pair.split_once('='))
                .find(|(k, _)| *k == "token")
                .map(|(_, v)| v);
            if let Some(token) = token {
                if token == state.settings.token {
                    return Some(AccessLevel::Admin);
                }
                if let Some(access) = viewer_access(state, token) {
                    return Some(access);
                }
            }
        }
    }
    None
}

/// 局域网 viewer token 的访问能力（每请求解析，配置变更热生效）：
/// - token：未拆分时随 writable，拆分（separate_write_token）时恒只读
/// - write_token：仅在启用写 + 拆分时有效，恒可写
fn viewer_access(state: &AppState, token: &str) -> Option<AccessLevel> {
    let web = &state.config.current().web;
    if !web.enabled {
        return None;
    }
    if !web.token.is_empty() && web.token == token {
        return Some(AccessLevel::Viewer {
            writable: web.writable && !web.separate_write_token,
        });
    }
    if web.separate_write_token
        && web.writable
        && !web.write_token.is_empty()
        && web.write_token == token
    {
        return Some(AccessLevel::Viewer { writable: true });
    }
    None
}

/// viewer（局域网 web 查看）默认只读：仅放行 GET 与查询类 POST；
/// 可写 token（[web] writable，拆分时为 write_token）解除限制（每请求经 current() 校验，保存即热生效）
fn is_viewer_allowed(req: &axum::extract::Request) -> bool {
    if req.method() == axum::http::Method::GET {
        return true;
    }
    // 查询类 POST（复杂过滤结构），语义只读
    matches!(
        req.uri().path(),
        "/api/v1/item/list" | "/api/v1/item/skeleton" | "/api/v1/item/aggregate"
    )
}

/// 启动网关：初始索引完成前拒绝一切 /api/* 请求（503 NOT_READY），仅放行 /api/v1/app/startup。
/// /health 与 /openapi 不在 /api 前缀下，由各自端点自行处理
async fn ready_gate(
    axum::extract::State(state): axum::extract::State<SharedState>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let path = req.uri().path();
    if !state.startup.is_ready() && path.starts_with("/api/") && path != "/api/v1/app/startup" {
        return envelope::ApiError::new(
            envelope::codes::NOT_READY,
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "initial index is still building",
        )
        .into_response();
    }
    next.run(req).await
}
