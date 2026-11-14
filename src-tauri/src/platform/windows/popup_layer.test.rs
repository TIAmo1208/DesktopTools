use super::*;

/// 构造一个只用于等待判定的弹层状态；窗口与焦点字段不参与判定。
fn state_of(request_id: Option<u64>, shown: bool, generation: u64) -> PopupLayerState { PopupLayerState { window: None, request: request_id.map(|request_id| PendingRequest { view: PopupLayerView { request_id, kind: PopupLayerKind::Menu, available: PopupLayerSpace { left: 0.0, right: 0.0, top: 0.0, bottom: 0.0 }, payload: Value::Null }, monitor: 0, anchor_x: 0.0, anchor_y: 0.0, foreground_before: 0 }), shown, foreground_before: 0, generation, needs_repaint: false } }

/// 同一个请求还没显示时继续等。
#[test]
fn measure_wait_holds_while_the_request_is_unshown()
{
    assert!(wait_is_pending(&state_of(Some(7), false, 3), 7, 3));
}

/// 显示之后不再等，避免把已经出现的浮层当成超时。
#[test]
fn measure_wait_ends_after_the_popup_is_shown()
{
    assert!(!wait_is_pending(&state_of(Some(7), true, 3), 7, 3));
}

/// 换了请求或收了请求都说明这次等待已经过期。
#[test]
fn measure_wait_ends_after_the_request_changed_or_ended()
{
    assert!(!wait_is_pending(&state_of(Some(8), false, 3), 7, 3), "换了请求不得再收束");
    assert!(!wait_is_pending(&state_of(None, false, 3), 7, 3), "请求已结束不得再收束");
}

/// 代次推进说明定位与显示流程已经换代，旧等待作废。
#[test]
fn measure_wait_ends_after_the_generation_advanced()
{
    assert!(!wait_is_pending(&state_of(Some(7), false, 4), 7, 3));
}

/// 超时收束必须带上同一身份：只有身份与代次都对得上才会被认领。
#[test]
fn measure_wait_never_claims_a_request_with_another_identity()
{
    let state = state_of(Some(9), false, 5); // 正在等 9 号请求。
    assert!(!wait_is_pending(&state, 8, 5), "别的请求不得被 9 号的计时收束");
    assert!(!wait_is_pending(&state, 9, 4), "代次不符不得收束");
    assert!(wait_is_pending(&state, 9, 5));
}

/// 带身份的取消只能结束同一请求；过期取消停在原地，不清请求也不关窗口。
#[test]
fn dismiss_applies_only_to_the_same_request()
{
    assert!(dismiss_applies_to(Some(7), Some(7)), "同一请求的取消要生效");
    assert!(!dismiss_applies_to(Some(7), Some(8)), "过期取消不得结束新请求");
    assert!(dismiss_applies_to(Some(7), None), "没有进行中的请求时只隐藏窗口");
    assert!(dismiss_applies_to(None, Some(7)), "不带身份的取消结束当前请求");
    assert!(dismiss_applies_to(None, None), "没有请求时取消是空操作");
}

/// 显示流程在失败时不得提前把 shown 置位，否则上报超时看门狗不再兜底。
///
/// 判定的是源码契约：`shown` 只在最终定位成功后赋值，之前的失败分支都不能写入它。
#[test]
fn shown_is_set_only_after_the_window_is_positioned()
{
    let source = include_str!("popup_layer.rs"); // 核对实际编译源码，不依赖真机显示结果。
    let positioned = source.find("window.set_position(tauri::PhysicalPosition::new(left, top))").expect("最终定位调用必须存在");
    let shown = source.find("state.shown = true").expect("显示成功必须记录已显示");
    assert!(shown > positioned, "shown 必须在最终定位之后才置位");
    assert_eq!(source.matches("state.shown = true").count(), 1, "只允许显示成功路径写入 shown");
}
