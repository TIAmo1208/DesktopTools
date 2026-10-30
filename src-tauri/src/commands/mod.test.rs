use super::{command_argument, created_weather_instance, weather_enabled_preferences, weather_tile_state};
use crate::contracts::{ComponentInstanceSnapshot, FrontendHostReadiness, WorkspaceRect};
use serde_json::json;

/// 创建天气格子入口把功能开关写成开启且只改这一项，原本已开启时不产生多余写入。
#[test]
fn weather_tile_entry_enables_the_feature_switch_once()
{
    let disabled = crate::contracts::settings::ApplicationPreferences::default(); // 首次启动的默认偏好。
    let enabled = weather_enabled_preferences(&disabled).expect("关闭状态下创建入口必须写入开启");
    assert!(enabled.weather_enabled);
    assert_eq!(enabled, crate::contracts::settings::ApplicationPreferences { weather_enabled: true, ..disabled }, "除天气开关外不得改动其他偏好");
    assert!(weather_enabled_preferences(&enabled).is_none(), "原本开启时不得重复写入");
}

/// 新天气格子只保存来源与城市两个标识，未绑定入口使用空对象。
#[test]
fn weather_tile_state_contains_only_the_binding_identifiers()
{
    let qweather = weather_tile_state(Some((&crate::weather::Source::Qweather, "101010100"))); // 绑定和风城市。
    let fields = qweather.as_object().expect("绑定状态必须是对象");
    assert_eq!(fields.len(), 2, "初始状态只允许两个标识字段：{qweather}");
    assert_eq!(fields["weatherSource"], "qweather");
    assert_eq!(fields["weatherCityId"], "101010100");
    let foreca = weather_tile_state(Some((&crate::weather::Source::Foreca, "100184745"))); // 绑定 Foreca 城市。
    assert_eq!(foreca["weatherSource"], "foreca");
    assert_eq!(foreca["weatherCityId"], "100184745");
    let unbound = weather_tile_state(None); // 未绑定入口的初始状态。
    assert_eq!(unbound, json!({}));
    assert_eq!(serde_json::to_string(&unbound).unwrap().len(), 2, "空对象序列化字节数为 2");
}

/// 新实例选取创建前不存在的第一个天气实例，其他组件身份不参与匹配。
#[test]
fn created_weather_instance_is_the_first_new_weather_component()
{
    // 与工作区快照字段一致的最小天气实例工厂。
    let weather = |instance_id: &str| ComponentInstanceSnapshot { instance_id: instance_id.into(), component_id: "official.weather".into(), monitor_id: "DISPLAY1".into(), projection_monitor_id: None, projection_bounds: None, bounds: WorkspaceRect { left: 24.0, top: 24.0, width: 360.0, height: 280.0 }, state_version: 1, state: json!({}) };
    assert_eq!(created_weather_instance(&[], &[weather("first")]), Some("first".into()), "existing 为空时取第一个天气实例");
    assert_eq!(created_weather_instance(&["first".into()], &[weather("first"), weather("second")]), Some("second".into()), "跳过创建前已存在的实例");
    assert_eq!(created_weather_instance(&["first".into(), "second".into()], &[weather("second"), weather("first")]), None, "全部已存在时没有新实例");
    let collect = ComponentInstanceSnapshot { component_id: "official.collect-box".into(), ..weather("collect") }; // 其他组件的身份不参与匹配。
    assert_eq!(created_weather_instance(&[], &[collect, weather("weather")]), Some("weather".into()), "只有 official.weather 身份参与匹配");
}

/// 原生天气命令在进入业务前拒绝缺失、非法或类型错误的参数。
#[test]
fn native_weather_arguments_are_rejected_before_business()
{
    // AppHandle 无法在单元测试中构造，这里直接覆盖 dispatch_native_command 各分支使用的同一参数边界。
    // 校验失败在读取业务状态之前返回，因此断言错误编码即可确认不会进入业务。
    let invalid = |error: crate::contracts::DesktopHostError| {
        let encoded = super::serialize_error(error); // transport 实际返回的错误对象。
        assert_eq!(encoded["code"], "invalid-desktop-host-readiness");
        assert!(encoded["message"].as_str().is_some_and(|message| !message.is_empty()));
    };
    invalid(command_argument::<crate::weather::Source>(&json!({"cityId": "101010100"}), "source", "天气来源参数无效").unwrap_err());
    invalid(command_argument::<crate::weather::Source>(&json!({"source": "unknown", "cityId": "101010100"}), "source", "天气来源参数无效").unwrap_err());
    invalid(command_argument::<String>(&json!({"source": "qweather"}), "cityId", "天气城市标识无效").unwrap_err());
    // create_weather_tile 的 monitorId 读取表达式：类型错误必须在进入业务前失败，并与其他参数使用同一错误编码。
    let monitor: Result<Option<String>, _> = json!({"source": "qweather", "cityId": "101010100", "monitorId": 7}).get("monitorId").cloned().map(serde_json::from_value).transpose();
    assert!(monitor.expect_err("monitorId 非字符串必须在进入业务前被拒绝").to_string().contains("invalid type"), "错误必须说明参数类型无效");
    assert!(include_str!("mod.rs").contains("map(serde_json::from_value).transpose().map_err(|error| serialize_error(DesktopHostError::invalid_readiness(error.to_string())))?"), "monitorId 类型错误必须编码为 invalid-readiness 对象，不能返回裸字符串");
    assert_eq!(command_argument::<crate::weather::Source>(&json!({"source": "foreca"}), "source", "天气来源参数无效").unwrap().key(), "foreca", "合法来源仍可解析");
    assert_eq!(command_argument::<String>(&json!({"cityId": "101010100"}), "cityId", "天气城市标识无效").unwrap(), "101010100", "合法城市标识仍可解析");
}

/// 打开设置窗口只经托盘既有显隐入口，不得另建窗口或承载设置语义。
#[test]
fn settings_window_is_opened_through_the_existing_entry()
{
    // 命令分发表源码，覆盖原生 transport 与 Tauri command 两处登记。
    let table = include_str!("mod.rs");
    assert!(table.contains(r#""open_settings_window" =>"#), "原生通道必须登记打开设置窗口");
    assert!(table.contains("crate::app::tray::set_settings_visible(&app, true)"), "必须复用既有设置窗口显隐入口");
    assert!(!table.contains("WebviewWindowBuilder"), "命令层不得自建窗口");
}

/// 桌面格子只读插件设置。没有登记时保存后的重读会被宿主拒绝。
#[test]
fn native_host_can_read_plugin_settings()
{
    let table = include_str!("mod.rs"); // 逐屏命令分发表。
    assert!(table.contains(r#""get_plugin_settings" =>"#), "原生通道必须登记读取插件设置");
    assert!(table.contains(r#"command_argument(&arguments, "pluginId""#), "逐屏参数名是 pluginId");
    assert!(table.contains("read_plugin_settings"), "必须走与设置窗口相同的读取");
    assert!(!table.contains(r#""save_plugin_settings" =>"#), "桌面格子不写插件设置");
}

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
