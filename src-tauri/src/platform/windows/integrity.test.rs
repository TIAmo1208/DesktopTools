//! 覆盖启动完整性门槛的纯逻辑判定。

use super::{MEDIUM_INTEGRITY_RID, low_integrity_message};
use std::path::Path;

#[test]
fn low_integrity_is_rejected_with_recovery_hint()
{
    // 低完整性进程的启动提示。
    let message = low_integrity_message(0x1000, Path::new("C:\\apps\\desktop-tools.exe")).expect("低完整性必须拒绝启动");
    assert!(message.contains("低完整性"));
    assert!(message.contains("icacls"));
    assert!(message.contains("C:\\apps"));
}

#[test]
fn medium_and_higher_integrity_is_allowed()
{
    assert!(low_integrity_message(MEDIUM_INTEGRITY_RID, Path::new("C:\\apps\\desktop-tools.exe")).is_none());
    assert!(low_integrity_message(0x3000, Path::new("C:\\apps\\desktop-tools.exe")).is_none());
}

#[test]
fn untrusted_integrity_is_rejected()
{
    assert!(low_integrity_message(0, Path::new("C:\\apps\\desktop-tools.exe")).is_some());
}
