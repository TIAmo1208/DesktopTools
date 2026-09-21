use super::*;
use std::time::Duration;

/// 构造正式协议消息，不绕过真实队列路径。
fn message(content: &str) -> ToastRequest { ToastRequest { r#type: "info".into(), content: content.into(), source: "test".into(), dedupe_key: None, loading: None, duration_ms: 4000 } }

/// 新消息独立淡入，旧层从当前画面缩小上移，连发也不跳回起点。
#[test]
fn incoming_toast_does_not_fade_existing_items()
{
    let now = Instant::now(); // 可重复的测试时钟。
    let mut queue = ToastState::default(); // 与正式窗口相同的队列。
    queue.enqueue(message("A"), 64.0, now, true);
    let start = queue.items[0].motion.sample(now, false, true); // 透明的下方起点。
    assert_eq!((start.top, start.alpha), (-48.0, 0.0));
    let settled = now + Duration::from_millis(250); // 第一条已完成进场。
    queue.enqueue(message("B"), 64.0, settled, true);
    let old = queue.items[0].motion.sample(settled, false, true); // 新消息不影响旧消息透明度。
    assert_eq!((old.top, old.scale, old.alpha), (-64.0, 1.0, 1.0));
    let middle = settled + Duration::from_millis(100); // 尚未完成让位。
    let before = queue.items[0].motion.sample(middle, false, true);
    queue.enqueue(message("C"), 64.0, middle, true);
    assert_eq!(queue.items[0].motion.sample(middle, false, true), before);
    let end = queue.items[0].motion.sample(middle + Duration::from_millis(250), false, true);
    assert_eq!((end.top, end.scale, end.alpha, end.content), (-92.0, 0.88, 1.0, 0.0));
}

/// 手动和自动关闭均向下淡出，旧层同时补位，动画完成后才移除。
#[test]
fn exit_is_per_item_and_preserves_remaining_opacity()
{
    let now = Instant::now(); // 可控时钟。
    let mut queue = ToastState::default(); // 两条消息的堆叠。
    queue.enqueue(message("A"), 64.0, now, true);
    queue.enqueue(message("B"), 64.0, now + Duration::from_millis(250), true);
    let close_time = now + Duration::from_millis(500); // 两条已稳定。
    let closing_id = queue.items[1].id; // 新前景的独立身份。
    queue.close(closing_id, close_time, true);
    assert_eq!(queue.items.len(), 2);
    assert_eq!(queue.items[1].motion.to.top, -48.0);
    assert_eq!(queue.items[1].motion.to.alpha, 0.0);
    assert_eq!(queue.items[0].motion.to.scale, 1.0);
    assert_eq!(queue.items[0].motion.from.alpha, 1.0);
    queue.tick(close_time + Duration::from_millis(250), true);
    assert_eq!(queue.items.len(), 1);
    queue.tick(now + Duration::from_secs(5), true);
    assert!(queue.items[0].closing);
    assert_eq!(queue.tick(now + Duration::from_millis(5250), true), None);
    assert!(queue.items.is_empty());
}

/// 悬停无空闲计时器，关闭动画直接落位，去重及容量规则保持。
#[test]
fn hover_and_disabled_animation_have_no_idle_frames()
{
    let now = Instant::now(); // 单调测试起点。
    let mut queue = ToastState::default(); // 有界队列。
    for index in 0..6
    {
        queue.enqueue(message(&index.to_string()), 64.0, now, false);
    }
    assert_eq!(queue.items.len(), 5);
    let duplicate = ToastRequest { dedupe_key: Some("same".into()), ..message("去重") }; // 同来源去重。
    assert!(queue.enqueue(duplicate.clone(), 64.0, now, false));
    assert!(!queue.enqueue(duplicate, 64.0, now, false));
    queue.hover(true, now + Duration::from_secs(1), false);
    assert_eq!(queue.tick(now + Duration::from_secs(10), false), None);
    queue.hover(false, now + Duration::from_secs(10), false);
    assert_eq!(queue.tick(now + Duration::from_secs(10), false), Some(3000));
    assert_eq!(queue.items.back().unwrap().motion.sample(now, false, false).alpha, 1.0);
}

/// 长旧消息折叠后只露出顶部，展开后恢复自己的测量高度。
#[test]
fn mixed_heights_do_not_leak_below_the_front_toast()
{
    let now = Instant::now(); // 可控时钟。
    let mut queue = ToastState::default(); // 一长一短的消息。
    queue.enqueue(message("长消息"), 160.0, now, false);
    queue.enqueue(message("短消息"), 64.0, now, false);
    let old = queue.items[0].motion.to; // 稳定折叠几何。
    assert!(old.top + old.height * old.scale <= 0.0);
    assert_eq!(old.height, 64.0);
    queue.hover(true, now, false);
    assert_eq!(queue.items[0].motion.to.height, 160.0);
}

/// 界面主题独立于桌面主题，动画遵守系统跟随优先级。
#[test]
fn independent_durations_and_visible_loading_control_wakeups()
{
    let now = Instant::now(); // 使用固定时间检查不同显示期限。
    let mut queue = ToastState::default();
    queue.enqueue(ToastRequest { duration_ms: 8000, loading: Some(true), ..message("长加载") }, 64.0, now, true);
    assert_eq!(queue.tick(now + Duration::from_millis(250), true), Some(33), "可见 loading 持续旋转");
    queue.enqueue(ToastRequest { duration_ms: 1000, ..message("短前景") }, 64.0, now + Duration::from_millis(250), true);
    assert_eq!(queue.tick(now + Duration::from_millis(500), true), Some(750), "折叠后层 loading 不浪费帧");
    queue.tick(now + Duration::from_millis(1250), true);
    assert!(!queue.items[0].closing);
    assert!(queue.items[1].closing, "后进入的短消息先退出");
    queue.tick(now + Duration::from_millis(1500), true);
    assert_eq!(queue.items.len(), 1);
    assert_eq!(queue.tick(now + Duration::from_millis(1500), true), Some(33));
    queue.hover(true, now + Duration::from_secs(2), false);
    assert_eq!(queue.tick(now + Duration::from_secs(3), false), None, "减少动态效果时保留静态 loading");
}

/// 界面主题独立于桌面主题，动画遵守系统跟随优先级。
#[test]
fn appearance_uses_interface_preferences_only()
{
    let mut preferences = ApplicationPreferences::default(); // 产品默认偏好。
    preferences.theme = ApplicationTheme::Light;
    preferences.desktop_appearance.theme = ApplicationTheme::Dark;
    preferences.interface_animations = true;
    let appearance = Appearance::resolve(&preferences, true, false); // 系统深色并关闭动画。
    assert!(!appearance.dark);
    assert!(!appearance.animated);
    preferences.theme = ApplicationTheme::System;
    preferences.follow_system_reduced_motion = false;
    let appearance = Appearance::resolve(&preferences, true, false);
    assert!(appearance.dark && appearance.animated);
}

/// 真 HWND 验证长正文测量、关闭命中、主题切换和重复销毁。
#[test]
fn native_surface_theme_and_close_follow_rendered_frames()
{
    let preferences = ApplicationPreferences { theme: ApplicationTheme::Light, follow_system_reduced_motion: false, interface_animations: false, ..Default::default() }; // 确定性原生测试。
    let manager = ToastManager::install(preferences).unwrap(); // 真实分层窗口。
    {
        let mut runtime = manager.runtime.lock().unwrap(); // 原生主线程入队。
        runtime.queue.enqueue(message(&"长正文换行测试。".repeat(15)), 64.0, Instant::now(), false);
    }
    manager.paint().unwrap();
    let frame = manager.runtime.lock().unwrap().frames[0].clone(); // 实际绘制的区域。
    assert!(frame.rect.bottom - frame.rect.top > 64.0);
    assert!(!frame.hit_close(frame.rect.left + 40.0, frame.rect.top + 32.0));
    assert!(frame.hit_close(frame.rect.right - 28.0, frame.rect.top + 32.0));
    let started = manager.runtime.lock().unwrap().queue.items[0].motion.started; // 换色前进场时间。
    manager.runtime.lock().unwrap().preferences.theme = ApplicationTheme::Dark;
    let preferences = manager.runtime.lock().unwrap().preferences.clone(); // 模拟程序现有主题解析入口。
    manager.sync_appearance(preferences, true, false);
    manager.sync_appearance(ApplicationPreferences::default(), false, true); // 迟到页面的旧偏好不能覆写当前主题。
    assert!(manager.runtime.lock().unwrap().appearance.dark);
    manager.paint().unwrap();
    assert_eq!(manager.runtime.lock().unwrap().queue.items[0].motion.started, started);
    let scale = manager.runtime.lock().unwrap().scale; // 转回实际输入坐标。
    manager.pointer(Some(POINT { x: ((frame.rect.right - 28.0) * scale) as i32, y: ((frame.rect.top + 32.0) * scale) as i32 }), true);
    assert!(manager.runtime.lock().unwrap().queue.items.is_empty());
    manager.cleanup();
    manager.cleanup();
    assert!(manager.hwnd().is_invalid());
}
