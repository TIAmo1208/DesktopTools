use super::command_argument;
use crate::contracts::FrontendHostReadiness;
use serde_json::json;

/// 缺失和类型错误参数不得进入业务执行，错误保留公开编码。
#[test]
fn native_arguments_reject_invalid_mutations()
{
    for value in [json!({}), json!({"permanent": "true"}), json!({"permanent": null})]
    {
        let error = command_argument::<bool>(&value, "permanent", "命令参数无效").unwrap_err(); // 入口校验失败。
        let encoded = super::serialize_error(error); // transport 实际返回的错误对象。
        assert!(encoded["code"].is_string());
        assert!(encoded["message"].as_str().is_some_and(|message| !message.is_empty()));
    }
    assert!(!command_argument::<bool>(&json!({"permanent": false}), "permanent", "命令参数无效").unwrap());
    assert!(command_argument::<crate::contracts::WorkspaceCommand>(&json!({"command": {"type": "unknown"}}), "command", "命令参数无效").is_err());
    assert_eq!(super::serialize_result(()).unwrap(), serde_json::Value::Null);
}

/// 验证本地 transport 与 Tauri command 使用相同的类型化准备证据。
#[test]
fn native_transport_decodes_typed_readiness_arguments()
{
    // 完整且字段类型有效的页面准备请求。
    let arguments = json!({
        "report": {
            "monitorId": "\\\\.\\DISPLAY1",
            "viewportWidth": 1920.0,
            "viewportHeight": 1040.0,
            "devicePixelRatio": 1.0,
            "totalSystemImages": 1,
            "loadedSystemImages": 1,
            "hitRegions": [{
                "id": "desktop-item",
                "kind": "desktop-item",
                "left": 20.0,
                "top": 20.0,
                "width": 92.0,
                "height": 135.0,
                "priority": 1
            }]
        }
    });
    // 解析后的类型化页面准备证据。
    let report: FrontendHostReadiness = command_argument(&arguments, "report", "页面准备参数无效").unwrap();

    assert_eq!(report.hit_regions.len(), 1);
    assert!(command_argument::<FrontendHostReadiness>(&json!({}), "report", "页面准备参数无效").is_err());
}
