//! SSE 事件总线：书库变更的推送通道。
//! 事件名与负载即持久契约（实现方必须逐字兼容），以常量集中定义，客户端不许凭代码反推。
//! 订阅者消费跟不上（积压超过容量）时 broadcast 通道返回 Lagged，订阅端点据此断开重连，
//! 计数进 app/status 的 sse_lagged。

use serde_json::Value;
use tokio::sync::broadcast;

/// SSE 事件名（REST 契约常量）
pub mod names {
    pub const ITEM_ADDED: &str = "item.added";
    pub const ITEMS_ADDED: &str = "items.added";
    pub const ITEM_UPDATED: &str = "item.updated";
    pub const ITEMS_UPDATED: &str = "items.updated";
    pub const ITEM_TRASHED: &str = "item.trashed";
    pub const ITEM_RESTORED: &str = "item.restored";
    pub const ITEM_REMOVED: &str = "item.removed";
    pub const FOLDER_CHANGED: &str = "folder.changed";
    pub const LIBRARY_UPDATED: &str = "library.updated";
    pub const GLOBAL_FILTER_CHANGED: &str = "global_filter.changed";
    pub const LOCKS_CHANGED: &str = "locks.changed";
    pub const TASK_PROGRESS: &str = "task.progress";
}

/// 一条 SSE 事件：事件名 + JSON 负载
#[derive(Clone, Debug)]
pub struct Event {
    pub name: &'static str,
    pub payload: Value,
}

/// broadcast 容量：API 文档约定积压 1024 条即断开订阅
pub const CHANNEL_CAPACITY: usize = 1024;

pub struct EventBus {
    sender: broadcast::Sender<Event>,
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl EventBus {
    pub fn new() -> EventBus {
        let (sender, _) = broadcast::channel(CHANNEL_CAPACITY);
        EventBus { sender }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.sender.subscribe()
    }

    /// 广播事件；无订阅者时静默丢弃（事件不保证送达，客户端重连后须全量对齐）
    pub fn emit(&self, name: &'static str, payload: Value) {
        let _ = self.sender.send(Event { name, payload });
    }

    pub fn emit_json<T: serde::Serialize>(&self, name: &'static str, payload: &T) {
        let value = serde_json::to_value(payload).unwrap_or(Value::Null);
        self.emit(name, value);
    }

    /// `task.progress` 专用：服务端 500ms 节流由调用方（task_progress 聚合器）保证
    pub fn emit_task(&self, task: &str, pending: u64, active: u64) {
        self.emit(
            names::TASK_PROGRESS,
            serde_json::json!({ "task": task, "pending": pending, "active": active }),
        );
    }
}
