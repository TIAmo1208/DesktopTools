use super::{ShellHistoryEntry, ShellHistoryState, publish_observation, reconcile_desktop_items, shell_compensation_actions, shell_history_items, wait_for_shell_activity};
use crate::contracts::{DesktopItem, DesktopItemAvailability, DesktopItemSnapshot, ShellOperationItemResult, ShellOperationKind, ShellOperationResult};
use crate::platform::windows::ShellCompensationAction;

/// 空闲联合等待不会自行返回，请求事件设置后立即唤醒。
#[test]
fn shell_wait_blocks_until_request_event()
{
    use std::ffi::c_void;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::System::Threading::{CreateEventW, SetEvent};

    let event = unsafe { CreateEventW(None, false, false, None) }.unwrap(); // 本测试独占的自动复位请求事件。
    let raw = event.0 as isize; // 等待线程借用，主线程负责最终关闭。
    let (started, waiting) = mpsc::sync_channel(0); // 确认线程即将进入生产等待。
    let (completed, result) = mpsc::sync_channel(0); // 观察等待是否返回。
    let worker = std::thread::spawn(move || {
        started.send(()).unwrap();
        completed.send(wait_for_shell_activity(HANDLE(raw as *mut c_void))).unwrap();
    });
    waiting.recv().unwrap();
    assert!(matches!(result.recv_timeout(Duration::from_millis(100)), Err(mpsc::RecvTimeoutError::Timeout)));

    let wake_started = Instant::now();
    unsafe { SetEvent(event) }.unwrap();
    result.recv_timeout(Duration::from_secs(1)).unwrap().unwrap();
    assert!(wake_started.elapsed() < Duration::from_millis(500));
    worker.join().unwrap();
    unsafe { CloseHandle(event) }.unwrap();
}

/// 验证暂时解析失败的项目保留最近一次有效元数据。
#[test]
fn unavailable_item_preserves_last_valid_metadata()
{
    // 最近一次有效项目集合。
    let previous = vec![DesktopItem { stable_reference: "::{project}".to_string(), display_name: "Project".to_string(), system_image: Some("data:image/png;base64,AA==".to_string()), availability: DesktopItemAvailability::Available }];
    // 本次 Shell 枚举观察到的暂时不可用项目。
    let observed = vec![DesktopItem { stable_reference: "::{project}".to_string(), display_name: "::{project}".to_string(), system_image: None, availability: DesktopItemAvailability::Unavailable }];

    // 按稳定引用协调后的项目集合。
    let reconciled = reconcile_desktop_items(observed, &previous);

    assert_eq!(reconciled[0].display_name, "Project");
    assert_eq!(reconciled[0].system_image.as_deref(), Some("data:image/png;base64,AA=="));
    assert_eq!(reconciled[0].availability, DesktopItemAvailability::Unavailable);
}

/// 验证枚举失败保留最近一次有效快照。
#[test]
fn enumeration_failure_preserves_last_valid_snapshot()
{
    // 最近一次有效桌面项目快照。
    let mut snapshot = DesktopItemSnapshot { revision: 8, items: vec![DesktopItem { stable_reference: "::{project}".to_string(), display_name: "Project".to_string(), system_image: None, availability: DesktopItemAvailability::Available }] };
    // Shell 枚举失败结果。
    let result = publish_observation(&mut snapshot, Err("Explorer 暂时不可用".to_string()));

    assert_eq!(result.expect_err("Shell 枚举失败应返回错误").code, "shell-enumeration-failed");
    assert_eq!(snapshot.revision, 8);
    assert_eq!(snapshot.items[0].display_name, "Project");
}

/// 验证补偿日志只接纳同时具有实际结果引用和所需父级名称的项目。
#[test]
fn reversible_shell_history_requires_complete_native_metadata()
{
    let result = ShellOperationResult {
        operation_id: 3,
        completed_at_unix_micros: 10,
        kind: ShellOperationKind::Recycle,
        aborted: false,
        operation_error: None, // 当前操作没有整体原生错误。
        reversible: true,
        permanent: false,
        effect: None,
        items: vec![ShellOperationItemResult { proof: None, compensation_blocked: None, source_reference: "desktop-old".to_string(), result_reference: Some("recycle-item".to_string()), succeeded: true, result_code: 0, source_parent_reference: Some("desktop-parent".to_string()), source_name: Some("old.txt".to_string()), result_parent_reference: Some("recycle-parent".to_string()), result_name: Some("old.txt".to_string()) }, ShellOperationItemResult { proof: None, compensation_blocked: None, source_reference: "missing-result".to_string(), result_reference: None, succeeded: true, result_code: 0, source_parent_reference: Some("desktop-parent".to_string()), source_name: Some("missing.txt".to_string()), result_parent_reference: None, result_name: None }],
        message: "移入回收站完成：2 项".to_string(),
    }; // 一个可明确恢复和一个缺少结果引用的 Shell 实际结果。

    let items = shell_history_items(&result);

    assert_eq!(items.len(), 1);
    assert_eq!(items[0].current_reference, "recycle-item");
}

/// 验证回收站撤销和重做分别生成移动恢复与再次回收动作。
#[test]
fn recycle_history_builds_inverse_native_actions()
{
    let result = ShellOperationResult {
        operation_id: 4,
        completed_at_unix_micros: 20,
        kind: ShellOperationKind::Recycle,
        aborted: false,
        operation_error: None, // 当前操作没有整体原生错误。
        reversible: true,
        permanent: false,
        effect: None,
        items: vec![ShellOperationItemResult { proof: None, compensation_blocked: None, source_reference: "desktop-old".to_string(), result_reference: Some("recycle-item".to_string()), succeeded: true, result_code: 0, source_parent_reference: Some("desktop-parent".to_string()), source_name: Some("old.txt".to_string()), result_parent_reference: Some("recycle-parent".to_string()), result_name: Some("old.txt".to_string()) }],
        message: "移入回收站完成：1 项".to_string(),
    }; // 已具备完整补偿元数据的回收结果。
    let mut entry = ShellHistoryEntry { busy: false, items: shell_history_items(&result), result, state: ShellHistoryState::Applied, timeline_at_unix_micros: 20, layout: None, layout_states: std::collections::HashMap::new(), pending_layout: Vec::new() };
    entry.items[0].proof = Some(crate::platform::windows::file_proof::FileProof::capture(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")).unwrap()); // 只读证据用于动作构建，本例不操作该文件。
    entry.items[0].state = ShellHistoryState::Applied;

    let undo = shell_compensation_actions(&entry, false);
    entry.items[0].state = ShellHistoryState::Undone;
    let redo = shell_compensation_actions(&entry, true);

    assert!(matches!(
        &undo[0],
        ShellCompensationAction::Move {
            reference,
            destination_parent_reference,
            new_name
        } if reference == "recycle-item"
            && destination_parent_reference == "desktop-parent"
            && new_name == "old.txt"
    ));
    assert!(matches!(
        &redo[0],
        ShellCompensationAction::Recycle { reference } if reference == "recycle-item"
    ));
    let mut retry = entry.items[0].clone(); // 同一轮部分失败仍停留在应用方向。
    retry.state = ShellHistoryState::Applied;
    retry.current_reference = "retry-item".into();
    entry.items.push(retry);
    assert!(super::shell_history_has_direction(&entry, false));
    assert!(super::shell_history_has_direction(&entry, true));
    super::invalidate_shell_redo(&mut entry);
    assert_eq!(shell_compensation_actions(&entry, false).len(), 1); // 新分支不能丢弃明确失败项。
    assert!(shell_compensation_actions(&entry, true).is_empty());
    entry.pending_layout.push(("result".into(), "result".into(), "recycled".into(), false));
    assert!(!super::shell_history_has_direction(&entry, true)); // 文件已撤销而布局未保存时先恢复同一方向。
}
