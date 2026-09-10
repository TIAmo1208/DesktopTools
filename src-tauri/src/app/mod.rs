use std::sync::Mutex;

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
mod tests
{
    use super::ApplicationState;
    use crate::contracts::{ApplicationError, ApplicationSnapshot, ApplicationStatus};

    /// 验证应用状态初始快照。
    #[test]
    fn initial_snapshot_is_ready()
    {
        // 待检查的正式应用状态。
        let state = ApplicationState::default();

        assert_eq!(state.snapshot(), Ok(ApplicationSnapshot { status: ApplicationStatus::Ready, revision: 0 }),);
    }

    /// 验证成功 command 生成新的权威快照。
    #[test]
    fn accepted_status_advances_the_snapshot_revision()
    {
        // 待更新的正式应用状态。
        let state = ApplicationState::default();
        // command 对应的应用状态结果。
        let snapshot = state.set_status("active");

        assert_eq!(snapshot, Ok(ApplicationSnapshot { status: ApplicationStatus::Active, revision: 1 }),);
    }

    /// 验证失败 command 保持权威快照。
    #[test]
    fn rejected_status_preserves_the_snapshot()
    {
        // 待校验的正式应用状态。
        let state = ApplicationState::default();
        // command 输入错误结果。
        let error = state.set_status("starting");

        assert_eq!(error, Err(ApplicationError::invalid_status()));
        assert_eq!(state.snapshot().expect("应用快照应可读取").revision, 0);
    }
}
