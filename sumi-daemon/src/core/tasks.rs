//! 后台任务积压追踪：app/status 与 SSE task.progress 的同一数据源。
//! - cover：解析 worker 队列（封面/书目元数据/全文索引）
//! - index：索引管道（排队 job + 写入防抖路径；扫描进行中 active=1 并携带阶段进度）
//! task.progress 的 500ms 节流由 SSE 侧聚合器保证，本结构只维护原子计数。

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::RwLock;

/// 最近一轮全库扫描统计（app/status 的 last_scan；未跑过为 None）
#[derive(Clone, Debug, Default)]
pub struct ScanStats {
    pub started_unix_ms: i64,
    pub duration_ms: u64,
    pub files: u64,
    pub dirty_dirs: u64,
    pub applied: u64,
}

/// 扫描进行中的阶段进度（与 startup phase 同名，但运行期扫描也走这里）
#[derive(Clone, Debug)]
pub struct IndexPhaseProgress {
    pub phase: &'static str,
    pub processed: u64,
    pub total: u64,
}

#[derive(Default)]
pub struct TaskTracker {
    pub cover_pending: AtomicU64,
    pub cover_active: AtomicU64,
    pub index_pending: AtomicU64,
    pub index_active: AtomicU64,
    /// 索引队列曾溢出（已触发兜底扫描）；下一轮扫描收尾后复位
    pub queue_overflow: AtomicBool,
    phase: RwLock<Option<IndexPhaseProgress>>,
    last_scan: RwLock<Option<ScanStats>>,
}

impl TaskTracker {
    pub fn new() -> TaskTracker {
        TaskTracker::default()
    }

    pub fn cover_incr_pending(&self) {
        self.cover_pending.fetch_add(1, Ordering::Relaxed);
    }

    pub fn cover_decr_pending(&self) {
        self.cover_pending.fetch_sub(1, Ordering::Relaxed);
    }

    pub fn cover_incr_active(&self) {
        self.cover_active.fetch_add(1, Ordering::Relaxed);
    }

    pub fn cover_decr_active(&self) {
        self.cover_active.fetch_sub(1, Ordering::Relaxed);
    }

    pub fn index_incr_pending(&self) {
        self.index_pending.fetch_add(1, Ordering::Relaxed);
    }

    pub fn index_decr_pending(&self) {
        self.index_pending.fetch_sub(1, Ordering::Relaxed);
    }

    pub fn index_set_active(&self, active: u64) {
        self.index_active.store(active, Ordering::Relaxed);
    }

    pub fn set_phase(&self, phase: Option<IndexPhaseProgress>) {
        *self.phase.write().unwrap() = phase;
    }

    pub fn phase_snapshot(&self) -> Option<IndexPhaseProgress> {
        self.phase.read().unwrap().clone()
    }

    pub fn mark_queue_overflow(&self) {
        self.queue_overflow.store(true, Ordering::Relaxed);
    }

    pub fn clear_queue_overflow(&self) {
        self.queue_overflow.store(false, Ordering::Relaxed);
    }

    pub fn finish_scan(&self, stats: ScanStats) {
        self.clear_queue_overflow();
        self.set_phase(None);
        *self.last_scan.write().unwrap() = Some(stats);
    }

    pub fn last_scan(&self) -> Option<ScanStats> {
        self.last_scan.read().unwrap().clone()
    }

    /// (cover_pending, cover_active, index_pending, index_active)
    pub fn counters(&self) -> (u64, u64, u64, u64) {
        (
            self.cover_pending.load(Ordering::Relaxed),
            self.cover_active.load(Ordering::Relaxed),
            self.index_pending.load(Ordering::Relaxed),
            self.index_active.load(Ordering::Relaxed),
        )
    }
}
