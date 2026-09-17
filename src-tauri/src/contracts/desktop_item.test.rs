use super::{DesktopItem, DesktopItemAvailability, DesktopItemError, DesktopItemSnapshot, ShellOperationItemResult, ShellOperationKind, ShellOperationResult};

/// 验证桌面项目快照使用公开序列化契约。
#[test]
fn desktop_item_snapshot_uses_the_public_contract_shape()
{
    // 等待序列化的桌面项目快照。
    let snapshot = DesktopItemSnapshot { revision: 3, items: vec![DesktopItem { stable_reference: r"C:\Users\User\Desktop\Project".to_string(), display_name: "Project".to_string(), system_image: Some("data:image/png;base64,AA==".to_string()), availability: DesktopItemAvailability::Available }] };
    // command 与 event 共用的 JSON 载荷。
    let payload = serde_json::to_value(snapshot).expect("桌面项目快照应可序列化");

    assert_eq!(
        payload,
        serde_json::json!({
            "revision": 3,
            "items": [{
                "stableReference": r"C:\Users\User\Desktop\Project",
                "displayName": "Project",
                "systemImage": "data:image/png;base64,AA==",
                "availability": "available"
            }]
        })
    );
}

/// 验证桌面项目错误使用独立公开契约。
#[test]
fn desktop_item_error_uses_the_public_contract_shape()
{
    // command JSON 错误载荷。
    let payload = serde_json::to_value(DesktopItemError::snapshot_unavailable()).expect("桌面项目错误应可序列化");

    assert_eq!(
        payload,
        serde_json::json!({
            "code": "desktop-item-snapshot-unavailable",
            "message": "桌面项目快照暂时不可用"
        })
    );
}

/// 验证永久删除结果明确序列化不可逆属性和原生状态。
#[test]
fn shell_operation_result_exposes_actual_and_permanent_state()
{
    let payload = serde_json::to_value(ShellOperationResult {
        operation_id: 8,
        completed_at_unix_micros: 1_800_000_000_000_000,
        kind: ShellOperationKind::PermanentDelete,
        aborted: false,
        operation_error: None, // 当前操作没有整体原生错误。
        reversible: false,
        permanent: true,
        effect: None,
        items: vec![ShellOperationItemResult { proof: None, compensation_blocked: None, source_reference: r"C:\Users\User\Desktop\obsolete.txt".to_string(), result_reference: None, succeeded: true, result_code: 0, source_parent_reference: None, source_name: None, result_parent_reference: None, result_name: None }],
        message: "永久删除完成：1 项".to_string(),
    })
    .expect("Shell 操作结果应可序列化"); // command 返回的公开 JSON 载荷。

    assert_eq!(payload["kind"], "permanent-delete");
    assert_eq!(payload["permanent"], true);
    assert_eq!(payload["items"][0]["succeeded"], true);
}
