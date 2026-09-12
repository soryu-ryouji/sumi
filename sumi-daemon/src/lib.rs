//! sumi-daemon：书籍管理后端。
//! 行为对齐基准 = OpenAPI schema + `.sumi/` 存储格式 + SSE 事件契约（见 docs/）。
//! 分层：`api/`（HTTP 端点、信封、鉴权中间件）→ `core/`（索引流水线、存储、解析器），
//! 依赖单向；进程入口在 main.rs，组件组装在 bootstrap.rs。

pub mod api;
pub mod bootstrap;
pub mod core;
pub mod settings;
