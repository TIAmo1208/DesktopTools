use super::{DesktopHostContext, DesktopHostRuntimeData, RENDERER_HEARTBEAT_TIMEOUT, RENDERER_NAVIGATION_LIMIT, RENDERER_READY_GRACE, RENDERER_RELOAD_BACKOFF_CAP, RENDERER_RELOAD_RETRY, TakeoverCoordinator, TakeoverPhase, all_hosts_ready, environment_failure_is_blocking, monitor_topology_changed, renderer_restart_due, renderer_startup_reload_delay, renderer_startup_reload_due};
use crate::contracts::{DesktopHostMonitor, DesktopHostReadiness};
use crate::platform::windows::{NativeDesktopHost, ShellDesktopState};

/// 验证 guardian 与全部逐屏门槛完成后才允许提交。
#[test]
fn takeover_waits_for_guardian_and_every_host()
{
    // 保存接管前 Explorer 可见状态的协调器。
    let mut coordinator = TakeoverCoordinator::new(true);
    assert!(!coordinator.begin_commit());

    coordinator.mark_guardian_ready();
    coordinator.mark_hosts_ready();
    assert!(coordinator.begin_commit());
    assert_eq!(coordinator.phase(), TakeoverPhase::Committing);
}

/// 创建逐屏门槛测试使用的显示器状态。
fn monitor(id: &str, scale_factor: f64) -> DesktopHostMonitor { DesktopHostMonitor { id: id.to_string(), name: id.to_string(), primary: id == "DISPLAY1", x: if id == "DISPLAY1" { 0 } else { 1920 }, y: 0, width: 1920, height: 1080, work_x: if id == "DISPLAY1" { 0 } else { 1920 }, work_y: 0, work_width: 1920, work_height: 1040, scale_factor, dpi: (96.0 * scale_factor).round() as u32 } }

/// 验证任一自动逐屏门槛缺失都会阻止统一桌面接管，输入诊断不阻塞提交。
#[test]
fn every_host_must_pass_every_automatic_gate()
{
    let mut coordinator = TakeoverCoordinator::new(true); // 当前全局接管协调器。
    coordinator.mark_guardian_ready();
    let ready = DesktopHostReadiness { guardian: true, page: true, system_images: true, pixels: true, input: true, hit_map: true }; // 当前完整逐屏门槛。
    let mut data = DesktopHostRuntimeData { coordinator, hosts: [monitor("DISPLAY1", 1.0), monitor("DISPLAY2", 1.5)].into_iter().enumerate().map(|(index, monitor)| DesktopHostContext { host: NativeDesktopHost { hwnd: index as isize + 1, monitor }, readiness: ready, hit_regions: Vec::new(), input_drag_active: false, last_renderer_heartbeat: std::time::Instant::now(), renderer_reload_requested_at: None, renderer_wait_started: std::time::Instant::now(), renderer_reload_attempts: 0, renderer_failure_reported: false }).collect(), shell: ShellDesktopState { def_view: 1, explorer_host: 1, list_view: 2, original_items_visible: true }, guardian: None, cleanup_complete: false, rebuild_in_progress: false, environment_stop_event: 0 }; // 当前双屏宿主运行状态。
    data.hosts[1].readiness.hit_map = false;
    assert!(!all_hosts_ready(&data));
    data.hosts[1].readiness.hit_map = true;
    data.hosts[1].readiness.input = false;
    data.hosts[1].readiness.pixels = false;
    assert!(all_hosts_ready(&data));
    data.hosts[1].readiness.page = false;
    assert!(!all_hosts_ready(&data));
}

/// 验证 DPI-only 变化不被分类为持久显示器拓扑变化。
#[test]
fn dpi_only_change_preserves_monitor_topology()
{
    let previous = [monitor("DISPLAY1", 1.0), monitor("DISPLAY2", 1.25)]; // 变化前显示器环境。
    let latest = [monitor("DISPLAY1", 1.0), monitor("DISPLAY2", 1.5)]; // 仅次屏 DPI 变化后的环境。
    assert!(!monitor_topology_changed(&previous, &latest));

    let disconnected = [monitor("DISPLAY1", 1.0)]; // 次屏断开后的真实拓扑。
    assert!(monitor_topology_changed(&previous, &disconnected));
}

/// 验证 Renderer 只在已接管、心跳超时且超过退避间隔时重载。
#[test]
fn renderer_restart_waits_for_committed_stale_heartbeat()
{
    assert!(!renderer_restart_due(false, RENDERER_HEARTBEAT_TIMEOUT, None, 0));
    assert!(renderer_restart_due(true, RENDERER_HEARTBEAT_TIMEOUT, None, 0));
    assert!(!renderer_restart_due(true, RENDERER_HEARTBEAT_TIMEOUT, Some(RENDERER_RELOAD_RETRY - std::time::Duration::from_millis(1)), 0));
    assert!(renderer_restart_due(true, RENDERER_HEARTBEAT_TIMEOUT, Some(RENDERER_RELOAD_RETRY), 0));
    assert!(!renderer_restart_due(true, RENDERER_HEARTBEAT_TIMEOUT, Some(RENDERER_RELOAD_RETRY), 1));
    assert!(renderer_restart_due(true, RENDERER_HEARTBEAT_TIMEOUT, Some(RENDERER_RELOAD_RETRY * 2), 1));
}

/// 验证未就绪页面只在导航完成后停滞、失败报告或导航超时后重载，连续重载按倍增间隔节流。
#[test]
fn renderer_startup_reload_waits_for_stalled_page_and_backs_off()
{
    let grace = RENDERER_READY_GRACE; // 导航完成后的就绪宽限期。
    let limit = RENDERER_NAVIGATION_LIMIT; // 导航迟迟未完成的兜底上限。
    let just_under = grace - std::time::Duration::from_millis(1); // 尚未到宽限期的等待时间。
    assert!(!renderer_startup_reload_due(true, true, limit, Some(limit), None, 0));
    assert!(!renderer_startup_reload_due(false, false, limit - std::time::Duration::from_millis(1), None, None, 0));
    assert!(renderer_startup_reload_due(false, false, limit, None, None, 0));
    assert!(!renderer_startup_reload_due(false, false, limit, Some(just_under), None, 0));
    assert!(renderer_startup_reload_due(false, false, limit, Some(grace), None, 0));
    assert!(renderer_startup_reload_due(false, true, std::time::Duration::ZERO, None, None, 0));
    assert!(!renderer_startup_reload_due(false, false, limit, Some(grace), Some(RENDERER_RELOAD_RETRY), 1));
    assert!(renderer_startup_reload_due(false, false, limit, Some(grace), Some(RENDERER_RELOAD_RETRY * 2), 1));
    assert_eq!(renderer_startup_reload_delay(0), RENDERER_RELOAD_RETRY);
    assert_eq!(renderer_startup_reload_delay(9), RENDERER_RELOAD_BACKOFF_CAP);
}

/// 验证环境瞬时发现失败不会立即撤下宿主，连续失败才进入阻塞恢复。
#[test]
fn environment_failure_uses_a_bounded_recovery_window()
{
    assert!(!environment_failure_is_blocking(1));
    assert!(!environment_failure_is_blocking(19));
    assert!(environment_failure_is_blocking(20));

    let mut coordinator = TakeoverCoordinator::new(true); // 已进入阻塞恢复的宿主协调器。
    coordinator.mark_failed();
    assert_eq!(coordinator.phase(), TakeoverPhase::Failed);
}

/// 验证正常退出和异常恢复使用最近观察到的用户图标设置。
#[test]
fn recovery_uses_the_latest_observed_user_visibility()
{
    let mut coordinator = TakeoverCoordinator::new(true); // 启动时桌面图标可见的恢复状态。
    coordinator.observe_user_visibility(false);

    assert!(!coordinator.restore());
    assert_eq!(coordinator.phase(), TakeoverPhase::Restored);
}
