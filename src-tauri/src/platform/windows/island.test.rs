use super::*;

#[test]
fn state_request_rejects_invalid_geometry_and_unknown_states()
{
    let valid = IslandStateRequest { state: "collapsed".into(), window_box: work(-100.0, 40.0, 200.0, 80.0), cards: vec![IslandCard { rect: work(0.0, 0.0, 200.0, 80.0), radius: 40.0 }] }; // 合法调用方实测区域。
    assert!(valid.validate().is_ok());
    for state in ["hover", "expanded"]
    {
        let mut request = valid.clone();
        request.state = state.into();
        assert!(request.validate().is_ok());
    }
    let mut request = valid.clone();
    request.state = "invalid".into();
    assert!(request.validate().is_err());
    request = valid.clone();
    request.window_box.left = f64::NAN;
    assert!(request.validate().is_err());
    request = valid.clone();
    request.cards[0].rect.width = 0.0;
    assert!(request.validate().is_err());
    request = valid.clone();
    request.cards[0].rect.left = 1.0;
    assert!(request.validate().is_err());
    request = valid.clone();
    request.cards[0].radius = 41.0;
    assert!(request.validate().is_err());
    request = valid.clone();
    request.cards.clear();
    assert!(request.validate().is_err());
    request = valid.clone();
    request.cards.push(valid.cards[0].clone());
    assert!(request.validate().is_err());
    request.state = "expanded".into();
    assert!(request.validate().is_ok());
    request.cards = vec![valid.cards[0].clone(); 129];
    assert!(request.validate().is_err());
}

#[test]
fn suppression_uses_lock_and_primary_monitor_coverage()
{
    use super::super::refresh_policy::{Coverage, ForegroundSnapshot};
    let mut snapshot = ForegroundSnapshot::default(); // 未知事实不伪造全屏。
    assert!(!suppressed(&snapshot, true, "main"));
    snapshot.locked = Some(true);
    assert!(suppressed(&snapshot, false, "main"));
    snapshot.locked = Some(false);
    snapshot.exclusive_fullscreen = Some(true);
    assert!(suppressed(&snapshot, true, "main"));
    assert!(!suppressed(&snapshot, false, "main"));
    snapshot.exclusive_fullscreen = Some(false);
    snapshot.coverage = Some(vec![Coverage { monitor_id: "other".into(), work: true, screen: true }]);
    assert!(!suppressed(&snapshot, true, "main"));
    snapshot.coverage = Some(vec![Coverage { monitor_id: "main".into(), work: true, screen: false }]);
    assert!(!suppressed(&snapshot, true, "main"));
    snapshot.maximized = Some(true);
    assert!(suppressed(&snapshot, true, "main"));
    assert!(!suppressed(&snapshot, false, "main"));
    snapshot.maximized = Some(false);
    snapshot.coverage.as_mut().unwrap()[0].screen = true;
    assert!(suppressed(&snapshot, true, "main"));
}

#[test]
fn window_coordinates_explicitly_convert_dip_to_pixels()
{
    assert_eq!(px(100.0, 1.5), 150);
    assert_eq!(px(-10.5, 2.0), -21);
    assert_eq!(px(11.0, 1.25), 14);
    assert_eq!(island_page_url(), "about:blank");
}

/// 表达主屏任务栏占用不同边后的 rcWork。
fn work(left: f64, top: f64, width: f64, height: f64) -> IslandRect { IslandRect { left, top, width, height } }

#[test]
fn snaps_to_each_edge_for_each_taskbar_work_area()
{
    // 上、下、左、右任务栏只改变 rcWork，不改变吸附算法。
    for area in [work(0.0, 40.0, 1920.0, 1040.0), work(0.0, 0.0, 1920.0, 1040.0), work(40.0, 0.0, 1880.0, 1080.0), work(0.0, 0.0, 1880.0, 1080.0)]
    {
        let mid_x = area.left + area.width / 2.0; // 水平中心。
        let mid_y = area.top + area.height / 2.0; // 垂直中心。
        assert_eq!(snap_edge(mid_x, area.top + 10.0, area), IslandEdge::Top);
        assert_eq!(snap_edge(mid_x, area.top + area.height - 10.0, area), IslandEdge::Bottom);
        assert_eq!(snap_edge(area.left + 10.0, mid_y, area), IslandEdge::Left);
        assert_eq!(snap_edge(area.left + area.width - 10.0, mid_y, area), IslandEdge::Right);
    }
}

#[test]
fn ties_keep_top_bottom_left_right_priority()
{
    let area = work(0.0, 0.0, 100.0, 100.0); // 正方工作区让四边等距。
    assert_eq!(snap_edge(50.0, 50.0, area), IslandEdge::Top);
    assert_eq!(snap_edge(10.0, 90.0, area), IslandEdge::Bottom);
    assert_eq!(snap_edge(90.0, 10.0, area), IslandEdge::Top);
    assert_eq!(snap_edge(50.0, 100.0, work(0.0, 0.0, 100.0, 200.0)), IslandEdge::Left);
    assert_eq!(snap_edge(-10.0, 50.0, area), IslandEdge::Left);
}

#[test]
fn clamps_offsets_at_both_ends_and_places_all_edges()
{
    let area = work(-200.0, 40.0, 400.0, 300.0); // 带负坐标的工作区。
    let size = (100.0, 50.0); // 岛的实际测量尺寸。
    for edge in [IslandEdge::Top, IslandEdge::Bottom, IslandEdge::Left, IslandEdge::Right]
    {
        let maximum = if matches!(edge, IslandEdge::Top | IslandEdge::Bottom) { 300.0 } else { 250.0 }; // 沿边最大偏移。
        assert_eq!(clamp_offset(edge, -10.0, size, area), 0.0);
        assert_eq!(clamp_offset(edge, 999.0, size, area), maximum);
        assert_eq!(clamp_offset(edge, 12.5, size, area), 12.5);
    }
    assert_eq!(place(IslandEdge::Top, -5.0, size, area), work(-200.0, 40.0, 100.0, 50.0));
    assert_eq!(place(IslandEdge::Bottom, 999.0, size, area), work(100.0, 290.0, 100.0, 50.0));
    assert_eq!(place(IslandEdge::Left, 999.0, size, area), work(-200.0, 290.0, 100.0, 50.0));
    assert_eq!(place(IslandEdge::Right, -5.0, size, area), work(100.0, 40.0, 100.0, 50.0));
}

#[test]
fn oversized_island_uses_work_origin_without_shrinking()
{
    let area = work(-100.0, -20.0, 40.0, 30.0); // 比岛小的退化工作区。
    for edge in [IslandEdge::Top, IslandEdge::Bottom, IslandEdge::Left, IslandEdge::Right]
    {
        assert_eq!(clamp_offset(edge, 999.0, (100.0, 50.0), area), 0.0);
        assert_eq!(place(edge, 999.0, (100.0, 50.0), area), work(-100.0, -20.0, 100.0, 50.0));
    }
}

#[test]
fn preferences_round_trip_and_empty_centers_at_top()
{
    for edge in [IslandEdge::Top, IslandEdge::Bottom, IslandEdge::Left, IslandEdge::Right]
    {
        assert_eq!(IslandEdge::from_preference(edge.as_preference()), Some(edge));
    }
    let area = work(-400.0, 40.0, 1000.0, 600.0); // 默认居中也以工作区起点为基准。
    assert_eq!(IslandEdge::from_preference(""), Some(IslandEdge::Top));
    assert_eq!(IslandEdge::from_preference("TOP"), None);
    assert_eq!(place_from_preference("", 999.0, (200.0, 50.0), area), Some(work(0.0, 40.0, 200.0, 50.0)));
    assert_eq!(place_from_preference("top", 0.0, (200.0, 50.0), area), Some(work(-400.0, 40.0, 200.0, 50.0)));
    assert_eq!(place_from_preference("unknown", 0.0, (200.0, 50.0), area), None);
}

#[test]
fn expansion_points_into_work_area()
{
    assert_eq!(expand_direction(IslandEdge::Top), (0.0, 1.0));
    assert_eq!(expand_direction(IslandEdge::Bottom), (0.0, -1.0));
    assert_eq!(expand_direction(IslandEdge::Left), (1.0, 0.0));
    assert_eq!(expand_direction(IslandEdge::Right), (-1.0, 0.0));
}

#[test]
fn union_covers_both_states_including_negative_coordinates()
{
    let current = work(-200.0, -50.0, 100.0, 40.0); // 当前态。
    let target = work(-160.0, -100.0, 300.0, 200.0); // 目标态跨越坐标原点。
    assert_eq!(union_box(current, target), work(-200.0, -100.0, 340.0, 200.0));
    assert_eq!(union_box(target, current), union_box(current, target));
    assert_eq!(union_box(current, current), current);
}
