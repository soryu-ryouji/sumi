//! app 端点组：info / startup / status / token（lan 待 LanSupervisor 就位后并入）。

use crate::api::{envelope, AccessLevel, SharedState};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::Serialize;
use utoipa::ToSchema;

#[derive(Serialize, ToSchema)]
pub struct AppInfo {
    pub version: String,
    pub platform: String,
    pub exec_path: String,
    pub access: String,
    pub writable: bool,
}

#[derive(Serialize, ToSchema)]
pub struct StartupPayload {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub processed: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct QueueCounters {
    pub pending: u64,
    pub active: u64,
}

#[derive(Serialize, ToSchema)]
pub struct IndexCounters {
    pub pending: u64,
    pub active: u64,
    pub phase: Option<String>,
    pub processed: Option<u64>,
    pub total: Option<u64>,
}

#[derive(Serialize, ToSchema)]
pub struct LastScan {
    pub started_unix_ms: i64,
    pub duration_ms: u64,
    pub files: u64,
    pub dirty_dirs: u64,
    pub applied: u64,
}

#[derive(Serialize, ToSchema)]
pub struct StatusPayload {
    pub cover: QueueCounters,
    pub index: IndexCounters,
    pub last_scan: Option<LastScan>,
    pub queue_overflow: bool,
    pub sse_lagged: u64,
}

fn platform_name() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "windows"
    }
    #[cfg(target_os = "macos")]
    {
        "macos"
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        "linux"
    }
}

/// `GET /api/v1/app/info`：当前 sumi-daemon 的运行信息（客户端环境能力判定）。
/// access/writable 按当前 token 报告（viewer 的写能力由 [web] 配置决定，保存即热生效）。
#[utoipa::path(get, path = "/api/v1/app/info", tags = ["app"],
    responses((status = 200, description = "OK", body = envelope::Envelope<AppInfo>)))]
pub async fn info(
    State(_state): State<SharedState>,
    axum::Extension(access): axum::Extension<AccessLevel>,
) -> impl IntoResponse {
    let (access_str, writable) = match access {
        AccessLevel::Admin => ("admin".to_string(), true),
        AccessLevel::Viewer { writable } => ("viewer".to_string(), writable),
    };
    axum::Json(envelope::Envelope::ok(AppInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        platform: platform_name().to_string(),
        exec_path: std::env::current_exe()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default(),
        access: access_str,
        writable,
    }))
}

/// `GET /api/v1/app/startup`：启动状态与索引构建进度（就绪网关唯一放行端点）。
#[utoipa::path(get, path = "/api/v1/app/startup", tags = ["app"],
    responses((status = 200, description = "OK", body = envelope::Envelope<StartupPayload>)))]
pub async fn startup(_state: State<SharedState>) -> impl IntoResponse {
    use crate::core::startup::StartupStatus;
    let payload = match _state.startup.snapshot() {
        StartupStatus::Starting { phase, processed, total } => StartupPayload {
            status: Some("starting".into()),
            phase: Some(
                match phase {
                    crate::core::startup::Phase::Sync => "sync",
                    crate::core::startup::Phase::Scan => "scan",
                    crate::core::startup::Phase::Hash => "hash",
                    crate::core::startup::Phase::Apply => "apply",
                }
                .into(),
            ),
            processed: Some(processed),
            total: Some(total),
            message: None,
        },
        StartupStatus::Ready => StartupPayload {
            status: Some("ready".into()),
            phase: None,
            processed: None,
            total: None,
            message: None,
        },
        StartupStatus::Error(message) => StartupPayload {
            status: Some("error".into()),
            phase: None,
            processed: None,
            total: None,
            message: Some(message),
        },
    };
    axum::Json(envelope::Envelope::ok(payload))
}

/// `GET /api/v1/app/status`：后台任务积压快照（轮询型客户端用；SSE 客户端订阅 task.progress）。
#[utoipa::path(get, path = "/api/v1/app/status", tags = ["app"],
    responses((status = 200, description = "OK", body = envelope::Envelope<StatusPayload>)))]
pub async fn status(State(state): State<SharedState>) -> impl IntoResponse {
    let (cover_pending, cover_active, index_pending, index_active) = state.tasks.counters();
    let phase = state.tasks.phase_snapshot();
    let last_scan = state.tasks.last_scan().map(|s| LastScan {
        started_unix_ms: s.started_unix_ms,
        duration_ms: s.duration_ms,
        files: s.files,
        dirty_dirs: s.dirty_dirs,
        applied: s.applied,
    });
    let payload = StatusPayload {
        cover: QueueCounters {
            pending: cover_pending,
            active: cover_active,
        },
        index: IndexCounters {
            pending: index_pending,
            active: index_active,
            phase: phase.as_ref().map(|p| p.phase.to_string()),
            processed: phase.as_ref().map(|p| p.processed),
            total: phase.as_ref().map(|p| p.total),
        },
        last_scan,
        queue_overflow: state
            .tasks
            .queue_overflow
            .load(std::sync::atomic::Ordering::Relaxed),
        sse_lagged: state
            .sse_lagged
            .load(std::sync::atomic::Ordering::Relaxed),
    };
    let mut value = serde_json::to_value(&payload).unwrap_or_default();
    // config_error 观测字段：解析错误时保留上次有效配置继续运行（正常不出现）
    if let Some(err) = state.config.config_error() {
        if let Some(obj) = value.as_object_mut() {
            obj.insert("config_error".into(), serde_json::json!(err));
        }
    }
    axum::Json(value)
}

/// `GET /api/v1/app/token`：免鉴权 token 发现端点（浏览器插件等生态客户端零配置接入）。
/// 安全性：响应不带 CORS 头（跨源网页 JS 读不到，持 host_permissions 的扩展可读）；
/// Host 必须是环回地址（防 DNS rebinding 伪装同源读取），否则 INVALID_HOST。
#[utoipa::path(get, path = "/api/v1/app/token", tags = ["app"],
    responses((status = 200, description = "OK", body = envelope::Envelope<String>)))]
pub async fn discover_token(
    State(state): State<SharedState>,
    req: axum::extract::Request,
) -> axum::response::Response {
    // HTTP/1.1 请求行是 origin-form（无 authority），Host 以 Host 头为准
    let host = req
        .headers()
        .get(axum::http::header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or_default()
        .split(':')
        .next()
        .unwrap_or_default()
        .trim_matches(['[', ']'])
        .to_string();
    if !matches!(host.as_str(), "127.0.0.1" | "localhost" | "::1" | "[::1]") {
        return envelope::ApiError::new(
            envelope::codes::INVALID_HOST,
            StatusCode::FORBIDDEN,
            format!("host {host} is not loopback"),
        )
        .into_response();
    }
    axum::Json(envelope::Envelope::ok(state.settings.token.clone())).into_response()
}

pub fn routes() -> utoipa_axum::router::OpenApiRouter<crate::api::SharedState> {
    utoipa_axum::router::OpenApiRouter::new()
        .routes(utoipa_axum::routes!(info))
        .routes(utoipa_axum::routes!(startup))
        .routes(utoipa_axum::routes!(status))
        .routes(utoipa_axum::routes!(discover_token))
}
