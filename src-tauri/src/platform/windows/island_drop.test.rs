use super::*;

/// 收集箱收入不能向源端报告移动，只接受源允许的复制效果。
#[test]
fn collect_effect_only_reports_copy()
{
    assert_eq!(collect_effect(DROPEFFECT_COPY), DROPEFFECT_COPY);
    assert_eq!(collect_effect(DROPEFFECT_COPY | DROPEFFECT_MOVE), DROPEFFECT_COPY);
    assert_eq!(collect_effect(DROPEFFECT_MOVE), DROPEFFECT_NONE);
    assert_eq!(collect_effect(DROPEFFECT_NONE), DROPEFFECT_NONE);
}

/// 不可接收的拖入也必须先拿到有效效果，页面才能收到 reject；可接收收集箱仍进入收入路径。
#[test]
fn enter_reports_typed_fact_and_never_returns_none()
{
    let collect = island_enter_decision(false, true, true, true, DROPEFFECT_COPY);
    assert_eq!(collect.effect, DROPEFFECT_COPY);
    assert_eq!(collect.fact, Some(IslandDropFact { active: true, kind: IslandDropKind::Collect }));

    // 收集箱未启用、格式不含收集箱、源不允许复制都归入 reject，但 Enter 的效果必须非 NONE。
    for (accepting, offer_has_collect, allowed) in [(false, true, DROPEFFECT_COPY), (true, false, DROPEFFECT_COPY), (true, true, DROPEFFECT_MOVE), (true, true, DROPEFFECT_NONE), (false, false, DROPEFFECT_MOVE | DROPEFFECT_LINK)]
    {
        let decision = island_enter_decision(false, accepting, offer_has_collect, true, allowed);
        assert_ne!(decision.effect, DROPEFFECT_NONE, "Enter 必须返回有效效果：{allowed:?}");
        assert_eq!(decision.fact, Some(IslandDropFact { active: true, kind: IslandDropKind::Reject }));
    }
}

/// 显示区域之外（展开态透明列）不进入接收路径、不发事实，效果仍非 NONE 以维持 OS 会话。
#[test]
fn outside_display_never_starts_receiving()
{
    for (accepting, offer_has_collect, allowed) in [(true, true, DROPEFFECT_COPY), (true, false, DROPEFFECT_COPY), (false, true, DROPEFFECT_MOVE)]
    {
        let decision = island_enter_decision(false, accepting, offer_has_collect, false, allowed);
        assert_ne!(decision.effect, DROPEFFECT_NONE, "透明列也要保持 OS 会话：{allowed:?}");
        assert_eq!(decision.fact, None, "透明列不发接收事实");
    }
}

/// reject 的进入效果优先用源允许的复制，其次链接、移动；源没声明任何效果时仍返回复制。
#[test]
fn reject_enter_effect_follows_source_allowed_effects()
{
    assert_eq!(reject_effect(DROPEFFECT_COPY | DROPEFFECT_MOVE), DROPEFFECT_COPY);
    assert_eq!(reject_effect(DROPEFFECT_LINK | DROPEFFECT_MOVE), DROPEFFECT_LINK);
    assert_eq!(reject_effect(DROPEFFECT_MOVE), DROPEFFECT_MOVE);
    assert_eq!(reject_effect(DROPEFFECT_NONE), DROPEFFECT_COPY);
}

/// 右键拖入直接拒绝，不进入接收路径也不发事件。
#[test]
fn right_button_drag_is_rejected_without_fact()
{
    for (accepting, offer_has_collect, allowed) in [(true, true, DROPEFFECT_COPY), (false, true, DROPEFFECT_COPY), (true, false, DROPEFFECT_MOVE)]
    {
        let decision = island_enter_decision(true, accepting, offer_has_collect, true, allowed);
        assert_eq!(decision.effect, DROPEFFECT_NONE);
        assert_eq!(decision.fact, None);
    }
}

/// 只有收集箱会话在移动阶段报复制；reject 与未进入接收路径都立即报禁止。
#[test]
fn over_reports_copy_only_for_collect_sessions()
{
    assert_eq!(island_over_effect(Some(IslandDropKind::Collect), DROPEFFECT_COPY), DROPEFFECT_COPY);
    assert_eq!(island_over_effect(Some(IslandDropKind::Collect), DROPEFFECT_COPY | DROPEFFECT_MOVE), DROPEFFECT_COPY);
    assert_eq!(island_over_effect(Some(IslandDropKind::Collect), DROPEFFECT_MOVE), DROPEFFECT_NONE);
    assert_eq!(island_over_effect(Some(IslandDropKind::Reject), DROPEFFECT_COPY), DROPEFFECT_NONE);
    assert_eq!(island_over_effect(None, DROPEFFECT_COPY), DROPEFFECT_NONE);
}

/// reject 放下不进入收入路径；可接收会话缺少条件时同样只回退。
#[test]
fn reject_drop_never_collects()
{
    assert_eq!(island_drop_decision(Some(IslandDropKind::Reject), true, true, DROPEFFECT_COPY), IslandDropDecision::Ignore);
    assert_eq!(island_drop_decision(None, true, true, DROPEFFECT_COPY), IslandDropDecision::Ignore);
    assert_eq!(island_drop_decision(Some(IslandDropKind::Collect), false, true, DROPEFFECT_COPY), IslandDropDecision::Ignore);
    assert_eq!(island_drop_decision(Some(IslandDropKind::Collect), true, false, DROPEFFECT_COPY), IslandDropDecision::Ignore);
    assert_eq!(island_drop_decision(Some(IslandDropKind::Collect), true, true, DROPEFFECT_MOVE), IslandDropDecision::Ignore);
    assert_eq!(island_drop_decision(Some(IslandDropKind::Collect), true, true, DROPEFFECT_COPY), IslandDropDecision::Income);
}

/// 离开与放下补发 inactive 并沿用进入类型；右键路径（未进入接收路径）不发事件。
#[test]
fn leave_reports_inactive_only_after_engagement()
{
    assert_eq!(island_leave_decision(Some(IslandDropKind::Collect)), Some(IslandDropFact { active: false, kind: IslandDropKind::Collect }));
    assert_eq!(island_leave_decision(Some(IslandDropKind::Reject)), Some(IslandDropFact { active: false, kind: IslandDropKind::Reject }));
    assert_eq!(island_leave_decision(None), None);
}

/// 事件载荷是带类型事实：active 为布尔，kind 只有 collect 与 reject。
#[test]
fn drop_fact_payload_is_typed()
{
    assert_eq!(serde_json::to_value(IslandDropFact { active: true, kind: IslandDropKind::Collect }).unwrap(), serde_json::json!({ "active": true, "kind": "collect" }));
    assert_eq!(serde_json::to_value(IslandDropFact { active: false, kind: IslandDropKind::Reject }).unwrap(), serde_json::json!({ "active": false, "kind": "reject" }));
    let kinds = serde_json::to_value([IslandDropKind::Collect, IslandDropKind::Reject]).unwrap();
    assert_eq!(kinds, serde_json::json!(["collect", "reject"]));
}

/// 音乐投放本轮不实现：投放目标与端口都没有音乐类型或音乐导入路径。
#[test]
fn music_drop_is_not_implemented()
{
    let target = include_str!("island_drop.rs"); // 核对实际编译源码，不只凭测试意图。
    assert!(!target.contains("Music"));
    assert!(!target.to_lowercase().contains("music"));
    assert!(!include_str!("../../drop_bridge.rs").to_lowercase().contains("music"));
}
