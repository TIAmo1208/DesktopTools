use std::sync::Mutex;

pub(crate) mod desktop_drop;
pub mod desktop_menu;
pub mod settings;
pub mod tray;

use crate::contracts::{ApplicationError, ApplicationSnapshot, ApplicationStatus, parse_application_status};

/// 持有正式应用的权威状态快照。
pub struct ApplicationState
{
    /// command 共享的当前应用快照。
    snapshot: Mutex<ApplicationSnapshot>,
}

/// 创建正式应用的初始状态。
impl Default for ApplicationState
{
    /// 创建就绪状态及初始修订号。
    fn default() -> Self { Self { snapshot: Mutex::new(ApplicationSnapshot { status: ApplicationStatus::Ready, revision: 0 }) } }
}

/// 提供 command 使用的应用状态操作。
impl ApplicationState
{
    /// 返回当前权威应用快照。
    pub fn snapshot(&self) -> Result<ApplicationSnapshot, ApplicationError>
    {
        // 当前受锁保护的应用快照。
        let snapshot = self.snapshot.lock().map_err(|_| ApplicationError::state_unavailable())?;

        Ok(snapshot.clone())
    }

    /// 校验并提交下一应用状态。
    pub fn set_status(&self, value: &str) -> Result<ApplicationSnapshot, ApplicationError>
    {
        // 已校验的下一应用状态。
        let status = parse_application_status(value)?;
        // 等待更新的权威应用快照。
        let mut snapshot = self.snapshot.lock().map_err(|_| ApplicationError::state_unavailable())?;
        snapshot.status = status;
        snapshot.revision += 1;

        Ok(snapshot.clone())
    }
}

#[cfg(test)]
#[path = "mod.test.rs"]
mod tests;
