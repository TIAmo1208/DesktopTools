use super::{SHELL_HIDE_ICONS_BIT, component_host_event_needs_time_consumers, host_event_kind, is_pixel_probe_region, match_desktop_item_monitor_assignments, shell_flag_state_icons_visible};
use crate::contracts::{DesktopHostMonitor, DesktopItem, DesktopItemAvailability, HitRegion};
use std::collections::HashMap;
use windows::Win32::Foundation::POINT;

/// 验证透明桌面背景保留输入能力且不参与渲染像素门槛。
#[test]
fn pixel_probe_excludes_the_workspace_background()
{
    let region = HitRegion { id: "workspace-background".to_string(), kind: "workspace-background".to_string(), left: 0.0, top: 0.0, width: 1920.0, height: 1080.0, priority: 1, target_path: None };

    assert!(!is_pixel_probe_region(&region));
}

/// 验证 Shell `fHideIcons` 用户设置与恢复可见性语义相反。
#[test]
fn shell_hide_icons_setting_maps_to_restore_visibility()
{
    assert!(shell_flag_state_icons_visible(0));
    assert!(!shell_flag_state_icons_visible(SHELL_HIDE_ICONS_BIT));
}

/// 验证 Explorer 物理图标坐标按显示器边界匹配稳定项目引用。
#[test]
fn explorer_positions_assign_items_to_monitors()
{
    let monitors = [DesktopHostMonitor { id: "DISPLAY1".to_string(), name: "DISPLAY1".to_string(), primary: true, x: 0, y: 0, width: 1920, height: 1080, work_x: 0, work_y: 0, work_width: 1920, work_height: 1040, scale_factor: 1.0, dpi: 96 }, DesktopHostMonitor { id: "DISPLAY2".to_string(), name: "DISPLAY2".to_string(), primary: false, x: -1280, y: 0, width: 1280, height: 1024, work_x: -1280, work_y: 0, work_width: 1280, work_height: 984, scale_factor: 1.25, dpi: 120 }]; // 当前测试的负坐标混合 DPI 显示器集合。
    let items = [DesktopItem { stable_reference: r"C:\Users\User\Desktop\Project".to_string(), display_name: "Project".to_string(), system_image: None, availability: DesktopItemAvailability::Available }]; // 当前等待 Explorer 坐标匹配的项目。
    let positions = HashMap::from([(r"c:\users\user\desktop\project".to_string(), POINT { x: -400, y: 120 })]); // Explorer 返回的绝对解析名称和物理坐标。

    assert_eq!(match_desktop_item_monitor_assignments(&items, &monitors, &positions), [(items[0].stable_reference.clone(), "DISPLAY2".to_string(), crate::contracts::WorkspacePoint { left: 704.0, top: 96.0 })]);
}

/// 验证 loopback 数据通知不依赖时间消费者，时间滴答仍按时间消费者过滤。
#[test]
fn loopback_host_events_do_not_require_time_consumers()
{
    let loopback = r#"{"sequence":3,"kind":"loopback-data:astrbot-push","unixMillis":1700000000000}"#;
    let tick = r#"{"sequence":4,"kind":"time-tick","unixMillis":1700000000000}"#;
    assert_eq!(host_event_kind(loopback).as_deref(), Some("loopback-data:astrbot-push"));
    assert!(!component_host_event_needs_time_consumers(loopback));
    assert!(component_host_event_needs_time_consumers(tick));
    assert!(component_host_event_needs_time_consumers("{"));
}

/// 浮岛消息事件必须同时进入 Rust 转发白名单与注入桥的事件映射，否则岛侧静默收不到。
#[test]
fn island_message_event_is_forwarded_and_mapped_to_the_read_command()
{
    assert!(super::FORWARDED_EVENTS.contains(&"island-messages-changed"), "事件必须由宿主转发到原生 WebView");
    let script = super::native_bridge_script("http://desktop-tools-island.localhost");
    assert!(script.contains(r#""island-messages-changed": "get_island_messages""#), "注入桥必须把事件映射到只读投影命令");
}
