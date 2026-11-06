use serde::Serialize;
use tauri::{AppHandle, Emitter};

/// 页面里的插件订阅宿主事件的通道名。
pub const PLUGIN_EVENT_CHANNEL: &str = "plugin-event";

/// 发给插件的宿主事件；`type` 决定语义，`payload` 由各事件自己定义。
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginHostEvent
{
    pub r#type: String,
    pub at: u64,
    pub payload: serde_json::Value,
}

/// 把一条宿主事件发给所有窗口；插件在桌面宿主页面用 `onPluginEvent` 收到。
pub fn emit_plugin_event(app: &AppHandle, kind: &str, payload: serde_json::Value)
{
    let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|elapsed| elapsed.as_millis() as u64).unwrap_or(0); // 事件时刻用毫秒，插件自己决定要不要用。
    if let Err(error) = app.emit(PLUGIN_EVENT_CHANNEL, PluginHostEvent { r#type: kind.to_string(), at, payload })
    {
        crate::log_warn!("插件事件：发送失败，类型={kind}，错误={error}"); // 没人订阅不算失败，只留日志。
    }
}

#[cfg(test)]
mod tests
{
    use super::PluginHostEvent;

    #[test]
    fn plugin_event_keeps_the_documented_shape()
    {
        let event = PluginHostEvent { r#type: "toast-shown".to_string(), at: 7, payload: serde_json::json!({ "kind": "info" }) };
        let value = serde_json::to_value(&event).unwrap();
        assert_eq!(value["type"], "toast-shown");
        assert_eq!(value["at"], 7);
        assert_eq!(value["payload"]["kind"], "info"); // 字段用驼峰，前端按同名字段读取。
    }
}
