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
