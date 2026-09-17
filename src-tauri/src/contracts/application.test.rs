use super::{ApplicationError, ApplicationSnapshot, ApplicationStatus, parse_application_status};

/// 验证 command 接受公开状态值。
#[test]
fn command_accepts_a_known_application_status()
{
    // command 输入解析结果。
    let status = parse_application_status("active");

    assert_eq!(status, Ok(ApplicationStatus::Active));
}

/// 验证 command 拒绝未知状态并返回稳定错误。
#[test]
fn command_rejects_an_unknown_application_status()
{
    // command 输入错误结果。
    let error = parse_application_status("starting");

    assert_eq!(error, Err(ApplicationError { code: "invalid-application-status".to_string(), message: "应用状态参数无效".to_string() }),);
}

/// 验证 event 载荷与前端契约字段一致。
#[test]
fn event_payload_uses_the_public_contract_shape()
{
    // 等待序列化的应用快照。
    let snapshot = ApplicationSnapshot { status: ApplicationStatus::Active, revision: 7 };
    // event JSON 载荷。
    let payload = serde_json::to_value(snapshot).expect("应用快照应可序列化");

    assert_eq!(payload, serde_json::json!({ "status": "active", "revision": 7 }));
}

/// 验证 command 错误使用公开序列化契约。
#[test]
fn command_error_uses_the_public_contract_shape()
{
    // command JSON 错误载荷。
    let payload = serde_json::to_value(ApplicationError::invalid_status()).expect("应用错误应可序列化");

    assert_eq!(payload, serde_json::json!({ "code": "invalid-application-status", "message": "应用状态参数无效" }));
}
