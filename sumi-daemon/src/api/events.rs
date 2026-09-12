//! SSE 订阅书库变更。EventSource 无法设置请求头，token 经查询参数传递（鉴权中间件放行）。
//! 消费跟不上（积压 1024 条）时服务端直接断开该订阅——客户端重连后必须以
//! item/skeleton + folder/list 全量对齐。

use crate::api::SharedState;
use axum::extract::State;
use axum::response::Response;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

pub fn routes() -> OpenApiRouter<SharedState> {
    OpenApiRouter::new().routes(routes!(events))
}

/// SSE 事件订阅：书库变更推送。帧格式 `event: <事件名>` + `data: <JSON 载荷>`；
/// 全部事件名与载荷契约见 core::events（事件名以常量集中定义，即持久契约）。
/// 消费跟不上（lagged）或总线关闭即断开；重连后须以 item/skeleton + folder/list 全量对齐
#[utoipa::path(
    get,
    path = "/api/v1/events",
    tags = ["events"],
    params(("token" = String, Query, description = "访问 token（EventSource 无法设置请求头）")),
    responses((status = 200, description = "text/event-stream 长连接", content_type = "text/event-stream", body = String))
)]
async fn events(State(state): State<SharedState>) -> Response {
    let mut rx = state.bus.subscribe();
    // lagged（消费跟不上）/总线关闭 → 结束流（断开订阅，客户端重连全量对齐）；
    // lagged 次数累计进 app/status（观测订阅端积压）
    let state = state.clone();
    let stream = async_stream::stream! {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let frame = axum::body::Bytes::from(format!(
                        "event: {}\ndata: {}\n\n",
                        event.name,
                        serde_json::to_string(&event.payload).unwrap_or_else(|_| "null".into())
                    ));
                    yield Ok::<axum::body::Bytes, std::convert::Infallible>(frame);
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    state.sse_lagged.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    break;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    };
    Response::builder()
        .status(axum::http::StatusCode::OK)
        .header("content-type", "text/event-stream")
        .header("cache-control", "no-cache")
        .body(axum::body::Body::from_stream(stream))
        .unwrap()
}
