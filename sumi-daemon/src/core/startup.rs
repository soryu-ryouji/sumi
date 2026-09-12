//! 启动状态机：先监听端口、初始索引后台构建，完成后放行全部 API。
//! phase 与 app/startup 契约对应：sync（元数据对账）/ scan / hash / apply。

use std::sync::RwLock;

#[derive(Clone, Debug, PartialEq)]
pub enum Phase {
    /// 元数据对账 + 注水（仅配置文件模式；TOML 全量回退时上报进度）
    Sync,
    /// 遍历清单（total 恒 0，客户端显示不定态进度）
    Scan,
    /// 并行计算哈希
    Hash,
    /// 应用索引变更
    Apply,
}

#[derive(Clone, Debug)]
pub enum StartupStatus {
    Starting {
        phase: Phase,
        processed: u64,
        total: u64,
    },
    Ready,
    Error(String),
}

pub struct StartupState {
    inner: RwLock<StartupStatus>,
}

impl StartupState {
    pub fn new() -> StartupState {
        StartupState {
            inner: RwLock::new(StartupStatus::Starting {
                phase: Phase::Sync,
                processed: 0,
                total: 0,
            }),
        }
    }

    pub fn snapshot(&self) -> StartupStatus {
        self.inner.read().unwrap().clone()
    }

    pub fn is_ready(&self) -> bool {
        matches!(self.snapshot(), StartupStatus::Ready)
    }

    pub fn set_phase(&self, phase: Phase, processed: u64, total: u64) {
        *self.inner.write().unwrap() = StartupStatus::Starting {
            phase,
            processed,
            total,
        };
    }

    pub fn set_ready(&self) {
        *self.inner.write().unwrap() = StartupStatus::Ready;
    }

    pub fn set_error(&self, message: impl Into<String>) {
        *self.inner.write().unwrap() = StartupStatus::Error(message.into());
    }
}
