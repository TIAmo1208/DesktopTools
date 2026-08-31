mod application;
mod desktop_host;
mod desktop_item;

use serde::Serialize;
use serde_json::{Value, json};
use tauri::{AppHandle, Manager};

use crate::contracts::{DesktopHostError, FrontendHostReadiness, HitRegion};
use crate::host::DesktopHostRuntime;
use crate::shell::DesktopItemState;

pub use application::{get_application_snapshot, set_application_status};
pub use desktop_host::{
    get_desktop_host_snapshot, publish_desktop_host_readiness, record_desktop_host_input_probe,
    update_desktop_host_hit_map,
};
pub use desktop_item::get_desktop_item_snapshot;

/// 路由独立 Windowed WebView2 transport 到正式应用命令边界。
pub(crate) fn dispatch_native_command(
    app: &AppHandle,
    runtime: &DesktopHostRuntime,
    command: &str,
    arguments: Value,
) -> Result<Value, Value> {
    // 当前本地 transport 命令的统一 JSON 结果。
    let result: Result<Value, Value> = match command {
        "get_desktop_item_snapshot" => {
            desktop_item::refresh_desktop_item_snapshot(&app.state::<DesktopItemState>())
                .map_err(serialize_error)
                .and_then(serialize_result)
        }
        "get_current_desktop_item_snapshot" => {
            desktop_item::read_desktop_item_snapshot(&app.state::<DesktopItemState>())
                .map_err(serialize_error)
                .and_then(serialize_result)
        }
        "get_desktop_host_snapshot" => desktop_host::read_desktop_host_snapshot(runtime)
            .map_err(serialize_error)
            .and_then(serialize_result),
        "publish_desktop_host_readiness" => {
            command_argument(&arguments, "report", "页面准备参数无效")
                .and_then(|report: FrontendHostReadiness| {
                    desktop_host::publish_readiness(runtime, report)
                })
                .map_err(serialize_error)
                .and_then(serialize_result)
        }
        "update_desktop_host_hit_map" => {
            command_argument(&arguments, "hitRegions", "命中图参数无效")
                .and_then(|regions: Vec<HitRegion>| desktop_host::replace_hit_map(runtime, regions))
                .map_err(serialize_error)
                .and_then(serialize_result)
        }
        "record_desktop_host_input_probe" => arguments
            .get("itemId")
            .and_then(Value::as_str)
            .ok_or_else(|| DesktopHostError::invalid_readiness("缺少 itemId"))
            .and_then(|item_id| desktop_host::record_input_probe(runtime, item_id))
            .map_err(serialize_error)
            .and_then(serialize_result),
        _ => Err(serialize_error(DesktopHostError::invalid_readiness(
            format!("正式宿主命令无效: {command}"),
        ))),
    };
    result
}

/// 从本地 transport 请求中读取并反序列化一个命名参数。
fn command_argument<T: serde::de::DeserializeOwned>(
    arguments: &Value,
    name: &str,
    invalid_message: &str,
) -> Result<T, DesktopHostError> {
    // 等待类型校验的命名参数值。
    let value = arguments
        .get(name)
        .cloned()
        .ok_or_else(|| DesktopHostError::invalid_readiness(format!("缺少 {name}")))?;
    serde_json::from_value(value)
        .map_err(|error| DesktopHostError::invalid_readiness(format!("{invalid_message}: {error}")))
}

/// 将类型化命令结果转换为本地 transport JSON。
fn serialize_result<T: Serialize>(value: T) -> Result<Value, Value> {
    serde_json::to_value(value).map_err(|_| serialize_error(DesktopHostError::state_unavailable()))
}

/// 将类型化命令错误转换为本地 transport JSON。
fn serialize_error(error: impl Serialize) -> Value {
    serde_json::to_value(error)
        .unwrap_or_else(|_| json!({ "code": "desktop-host-error", "message": "桌面宿主命令失败" }))
}

#[cfg(test)]
mod tests {
    use super::command_argument;
    use crate::contracts::FrontendHostReadiness;
    use serde_json::json;

    /// 验证本地 transport 与 Tauri command 使用相同的类型化准备证据。
    #[test]
    fn native_transport_decodes_typed_readiness_arguments() {
        // 完整且字段类型有效的页面准备请求。
        let arguments = json!({
            "report": {
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
        let report: FrontendHostReadiness =
            command_argument(&arguments, "report", "页面准备参数无效").unwrap();

        assert_eq!(report.hit_regions.len(), 1);
        assert!(
            command_argument::<FrontendHostReadiness>(&json!({}), "report", "页面准备参数无效")
                .is_err()
        );
    }
}
