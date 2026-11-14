use super::*;

/// 正式窗口路径固定透明矩形，防止重新混入裁切、材质或实验开关。
#[test]
fn transparent_rectangle_window_contract_has_no_shape_hit_test_scaffolding()
{
    let source = include_str!("island.rs"); // 核对实际编译源码，不依赖真机壁纸截图。
    assert!(source.contains("DwmEnableBlurBehindWindow(hwnd, &blur)"));
    assert!(source.contains("DwmExtendFrameIntoClientArea(hwnd"));
    assert!(source.contains("hbrBackground: Default::default()"));
    assert!(!source.contains("EXPERIMENT"));
    assert!(!source.contains("SetWindowRgn("));
    assert!(!source.contains("WM_NCHITTEST"));
    assert!(!source.contains("DWMSBT"));
}

/// 释放时返回矩形客户区事实，圆角透明角也属于矩形内，不复用原生区域命中判断。
#[test]
fn drag_pointer_result_uses_client_rectangle_and_camel_case()
{
    let rect = RECT { left: 0, top: 0, right: 200, bottom: 72 };
    for (x, y) in [(0, 0), (199, 0), (0, 71), (199, 71), (100, 36)]
    {
        assert!(point_inside_client(POINT { x, y }, rect));
    }
    for (x, y) in [(-1, 0), (0, -1), (200, 0), (0, 72), (200, 72), (1000, 1000)]
    {
        assert!(!point_inside_client(POINT { x, y }, rect));
    }
    assert!(!point_inside_client(POINT { x: 0, y: 0 }, RECT::default()));
    assert_eq!(serde_json::to_value(IslandDragResult { pointer_inside: true, offset: Some(-12.5) }).unwrap(), serde_json::json!({"pointerInside": true, "offset": -12.5}));
    assert_eq!(serde_json::to_value(IslandDragResult::default()).unwrap(), serde_json::json!({"pointerInside": false, "offset": null}));
    let (done, saved) = std::sync::mpsc::sync_channel(1);
    drop(done);
    assert!(!wait_for_landing(saved, std::time::Duration::ZERO).pointer_inside); // 取消/失效路径不能误报 true。
}

/// 命令保存屏障返回前，权威偏好已可见；旧序号正常丢弃也会释放屏障。
#[test]
fn landing_wait_exposes_saved_preferences_and_stale_sequence_is_success()
{
    let directory = std::env::temp_dir().join(format!("desktop-tools-island-wait-{}", std::process::id())); // 本测试独占配置目录。
    let state = Arc::new(SettingsState::load(directory.join("settings.json")).unwrap());
    state
        .update(|preferences| {
            preferences.island_enabled = true;
            Ok(())
        })
        .unwrap();
    let mut order = LandingOrder::default();
    let sequence = order.begin().unwrap();
    let (done, saved) = std::sync::mpsc::sync_channel(1);
    let worker_state = Arc::clone(&state);
    let worker = std::thread::spawn(move || {
        assert!(order.save(&worker_state, sequence, IslandEdge::Bottom, 1547.0).unwrap().is_some());
        done.send(IslandDragResult { pointer_inside: true, offset: Some(7.0) }).unwrap(); // 与生产一致：先发布完成信号，后台再保存偏好。
        order
    });
    let landed = wait_for_landing(saved, std::time::Duration::from_secs(2));
    assert!(landed.pointer_inside);
    assert_eq!(landed.offset, Some(7.0));
    assert_eq!(state.preferences().unwrap().island_edge, "bottom");
    assert_eq!(state.preferences().unwrap().island_offset, 1547.0);
    let mut order = worker.join().unwrap();
    let stale = order.begin().unwrap();
    order.begin().unwrap(); // 模拟更新的拖动序号已开始。
    let (done, saved) = std::sync::mpsc::sync_channel(1);
    assert!(order.save(&state, stale, IslandEdge::Left, 1.0).unwrap().is_none());
    done.send(IslandDragResult::default()).unwrap(); // 丢弃不是错误，依然确认完成，不走超时或失败分支。
    assert!(!wait_for_landing(saved, std::time::Duration::ZERO).pointer_inside);
    assert_eq!(state.preferences().unwrap().island_offset, 1547.0);
    std::fs::remove_dir_all(directory).unwrap();
}

/// 未调度保存的取消会断开通道；仍在后台的保存等待有界，不影响拖动结果。
#[test]
fn landing_cancel_and_timeout_are_bounded()
{
    let (done, saved) = std::sync::mpsc::sync_channel(1);
    let mut drag = DragProgress { landing_done: Some(done), ..Default::default() };
    drag.invalidate();
    assert_eq!(saved.recv_timeout(std::time::Duration::ZERO), Err(std::sync::mpsc::RecvTimeoutError::Disconnected));
    let (done, saved) = std::sync::mpsc::sync_channel(1);
    assert!(!wait_for_landing(saved, std::time::Duration::ZERO).pointer_inside); // 不等后台完成也不把超时升级为拖动失败。
    assert!(done.send(IslandDragResult::default()).is_err()); // 超时后后台任务仍可自行结束，发送失败可以忽略。
}

/// 动画中间帧和最终态复用同一校验；前置参数错误与原生失败都只清理一次。
#[test]
fn animation_submission_failure_clears_intermediate_and_final_state_can_recover()
{
    let final_state = IslandStateRequest { state: "collapsed".into(), window_box: work(0.0, 0.0, 200.0, 44.0), viewport: work(-12.0, -12.0, 224.0, 68.0), cards: vec![IslandCard { rect: work(0.0, 0.0, 200.0, 44.0), radius: 22.0 }] }; // 合法最终常态可重新恢复。
    let mut invalid = final_state.clone();
    invalid.cards[0].rect.width = 201.0; // 模拟 overshoot 使卡片越出本帧窗口盒。
    let mut geometry = Some(final_state.clone()); // 上一合法帧不能在下一帧错误后残留。
    let mut drag = DragProgress::default();
    let mut hidden = 0;
    for result in [invalid.validate(), Err("原生区域提交失败".into())]
    {
        assert!(
            finish_state_submission(result, || {
                geometry = None;
                drag.invalidate();
                hidden += 1;
            })
            .is_err()
        );
        assert!(geometry.is_none());
        assert!(!can_drag(geometry.as_ref()));
    }
    assert_eq!(hidden, 2);
    assert!(finish_state_submission(final_state.validate(), || hidden += 1).is_ok());
    run_transition(
        final_state.clone(),
        |_| Ok(()),
        |_, update| {
            update(&mut geometry, &mut drag);
            Ok(())
        },
        || hidden += 1,
    )
    .unwrap(); // 与生产成功提交共用收口。
    assert_eq!(geometry.unwrap().window_box, final_state.window_box);
    assert_eq!(hidden, 2);
}

/// 常态与 hover 共享窗口拖动门禁；展开、未知态及失败后空几何仍拒绝。
#[test]
fn window_drag_accepts_collapsed_and_hover_only()
{
    for (state, allowed) in [("collapsed", true), ("hover", true), ("expanded", false), ("unknown", false)]
    {
        let geometry = IslandStateRequest { state: state.into(), window_box: work(0.0, 0.0, 200.0, 72.0), viewport: work(-12.0, -12.0, 224.0, 96.0), cards: vec![] };
        assert_eq!(can_drag(Some(&geometry)), allowed, "{state}");
    }
    assert!(!can_drag(None));
}

/// 点击前台补偿只能消费一次；过期前台、无效目标、原生拖动都必须放弃。
#[test]
fn click_foreground_is_one_shot_and_never_overrides_new_focus_or_drag()
{
    let mut state = ClickForeground { target: Some(100) };
    assert_eq!(state.take(200, 200, true, false), Some(100));
    assert_eq!(state.take(200, 200, true, false), None);
    for (foreground, valid, dragging) in [(200, false, false), (200, true, true), (300, true, false)]
    {
        state.target = Some(100);
        assert_eq!(state.take(200, foreground, valid, dragging), None);
        assert!(state.target.is_none());
    }
    for target in [None, Some(0), Some(200)]
    {
        state.target = target;
        assert_eq!(state.take(200, 200, true, false), None);
    }
}

/// 不激活位只做幂等合并，WebView2 已有扩展样式必须逐位保留。
#[test]
fn child_noactivate_style_preserves_bits_and_is_idempotent()
{
    for style in [0, 0x00200000, 0x00000020, 0x00280024, 0x08000088, -1]
    {
        let merged = noactivate_style(style);
        assert_eq!(merged & WS_EX_NOACTIVATE.0 as isize, WS_EX_NOACTIVATE.0 as isize);
        assert_eq!(merged & !(WS_EX_NOACTIVATE.0 as isize), style & !(WS_EX_NOACTIVATE.0 as isize));
        assert_eq!(noactivate_style(merged), merged);
    }
}

#[test]
fn state_request_rejects_invalid_geometry_and_unknown_states()
{
    let valid = IslandStateRequest { state: "collapsed".into(), window_box: work(-100.0, 40.0, 200.0, 80.0), viewport: work(-112.0, 28.0, 224.0, 104.0), cards: vec![IslandCard { rect: work(0.0, 0.0, 200.0, 80.0), radius: 40.0 }] }; // 合法调用方实测区域。
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
    request = valid.clone();
    request.viewport.left = valid.window_box.left + 1.0;
    assert!(request.validate().is_err()); // 窗口超出固定视口。
    request = valid.clone();
    request.viewport.width = 20_000.0;
    assert!(request.validate().is_err()); // 固定视口上限。
    request = valid.clone();
    request.state = "expanded".into();
    assert!(request.validate().is_ok());
    request.cards = vec![valid.cards[0].clone(); 129];
    assert!(request.validate().is_err());
}

/// 夹紧后仍在视口内但原点被移动，同样会破坏页面 margin 与父窗口裁剪的对齐，必须拒绝。
#[test]
fn normalization_cannot_move_the_page_origin_even_inside_viewport()
{
    let viewport = IslandRect { left: 0.0, top: 0.0, width: 420.0, height: 800.0 };
    let requested = IslandRect { left: 150.0, top: 750.0, width: 120.0, height: 44.0 };
    let clamped = clamp_window(requested, IslandRect { left: 0.0, top: 0.0, width: 2000.0, height: 760.0 });
    assert_eq!(clamped.top, 716.0);
    assert!(window_in_viewport(clamped, viewport)); // 仍在固定视口内。
    assert!(accept_normalized_window(requested, clamped, viewport).is_err()); // 但原点变化必须拒绝。
    assert!(accept_normalized_window(requested, requested, viewport).is_ok());
    assert!(accept_normalized_window(requested, IslandRect { left: 150.0, top: 0.0, width: 120.0, height: 44.0 }, viewport).is_err());
}

/// 工作区变化后夹紧会把窗口挪出页面提交的固定视口；必须由同一不变量拒绝。
#[test]
fn clamp_after_validate_can_break_viewport_and_must_be_rejected()
{
    let viewport = IslandRect { left: 1580.0, top: 0.0, width: 420.0, height: 800.0 };
    let window = IslandRect { left: 1880.0, top: 0.0, width: 120.0, height: 44.0 };
    assert!(window_in_viewport(window, viewport)); // 页面旧工作区下合法。
    let clamped = clamp_window(window, IslandRect { left: 0.0, top: 0.0, width: 1000.0, height: 800.0 });
    assert_eq!(clamped.left, 880.0);
    assert!(!window_in_viewport(clamped, viewport)); // 新工作区夹紧后必须由后置校验拦下。
}

/// 窗口落在视口内才允许提交；夹紧后越界必须被同一判定拦下。
#[test]
fn window_must_stay_inside_fixed_viewport()
{
    let viewport = IslandRect { left: 80.0, top: 20.0, width: 420.0, height: 600.0 };
    assert!(window_in_viewport(IslandRect { left: 80.0, top: 20.0, width: 420.0, height: 600.0 }, viewport));
    assert!(window_in_viewport(IslandRect { left: 80.4, top: 20.4, width: 419.2, height: 599.2 }, viewport)); // 半像素容差内。
    assert!(!window_in_viewport(IslandRect { left: 79.0, top: 20.0, width: 420.0, height: 600.0 }, viewport));
    assert!(!window_in_viewport(IslandRect { left: 80.0, top: 20.0, width: 421.0, height: 600.0 }, viewport));
    assert!(!window_in_viewport(IslandRect { left: 80.0, top: 20.0, width: 420.0, height: 601.0 }, viewport));
}

/// 固定视口下，无论窗口怎么移动，子窗口相对父窗口的偏移都让内容保持同一屏幕位置。
#[test]
fn fixed_viewport_keeps_child_screen_origin_across_window_moves()
{
    let viewport = IslandRect { left: 80.0, top: 20.0, width: 420.0, height: 600.0 };
    let first = child_bounds(IslandRect { left: 100.0, top: 50.0, width: 200.0, height: 72.0 }, viewport, 1.0);
    assert_eq!(first, (-20, -30, 420, 600));
    let moved = child_bounds(IslandRect { left: 150.0, top: 80.0, width: 148.0, height: 44.0 }, viewport, 1.0);
    assert_eq!(moved, (-70, -60, 420, 600)); // 窗口右下移动时偏移更负，屏幕原点不变。
    // 1.5 倍缩放下子窗口位置按物理像素四舍五入，尺寸仍覆盖视口。
    let scaled = child_bounds(IslandRect { left: 101.0, top: 51.0, width: 200.0, height: 72.0 }, viewport, 1.5);
    assert_eq!(scaled, (px(80.0, 1.5) - px(101.0, 1.5), px(20.0, 1.5) - px(51.0, 1.5), px(420.0, 1.5), px(600.0, 1.5)));
    // 负坐标与部分越界也保持同一换算。
    let negative = child_bounds(IslandRect { left: -100.0, top: 0.0, width: 200.0, height: 72.0 }, viewport, 1.0);
    assert_eq!(negative.0, px(80.0, 1.0) - px(-100.0, 1.0));
}

#[test]
fn screenshot_hotkey_timeout_turns_pending_into_recovery()
{
    assert!(!screenshot_hotkey_timeout(0, 9_999, false)); // 未触发。
    assert!(!screenshot_hotkey_timeout(1_000, 1_500, false)); // 500ms 内等待截图工具。
    assert!(screenshot_hotkey_timeout(1_000, 2_201, false)); // 超过 1200ms 未出现。
    assert!(screenshot_hotkey_timeout(1_000, 1_100, true)); // 工具已出现后离开前台。
}

#[test]
fn screenshot_tool_process_names_are_recognized()
{
    for name in ["screenclipper.exe", "snippingtool.exe", "screensketch.exe"]
    // 系统截图浮层的可执行文件名。
    {
        assert!(is_screenshot_process(name), "应识别截图工具：{name}");
    }
    for name in ["explorer.exe", "screenclip.exe", "snippingtools.exe", ""]
    // 相近名称不能误伤普通程序。
    {
        assert!(!is_screenshot_process(name), "不应把普通程序当截图工具：{name}");
    }
}

#[test]
fn suppression_only_uses_lock_and_exclusive_fullscreen()
{
    use super::super::refresh_policy::{Coverage, ForegroundSnapshot};
    let mut snapshot = ForegroundSnapshot::default(); // 未知事实不伪造全屏。
    assert!(!suppressed(&snapshot, true));
    snapshot.locked = Some(true);
    assert!(suppressed(&snapshot, false));
    snapshot.locked = Some(false);
    snapshot.exclusive_fullscreen = Some(true);
    assert!(suppressed(&snapshot, true));
    assert!(!suppressed(&snapshot, false));
    snapshot.exclusive_fullscreen = Some(false);
    snapshot.coverage = Some(vec![Coverage { monitor_id: "other".into(), work: true, screen: true }]);
    assert!(!suppressed(&snapshot, true));
    snapshot.coverage = Some(vec![Coverage { monitor_id: "main".into(), work: true, screen: false }]);
    assert!(!suppressed(&snapshot, true));
    snapshot.maximized = Some(true);
    assert!(!suppressed(&snapshot, true));
    assert!(!suppressed(&snapshot, false));
    snapshot.maximized = Some(false);
    snapshot.coverage.as_mut().unwrap()[0].screen = true;
    assert!(!suppressed(&snapshot, true)); // 即使覆盖整屏，也不能把普通应用最大化当独占全屏。
    snapshot.locked = Some(true);
    assert!(suppressed(&snapshot, false)); // 锁屏不受全屏隐藏开关控制。
}

#[test]
fn window_coordinates_explicitly_convert_dip_to_pixels()
{
    assert_eq!(px(100.0, 1.5), 150);
    assert_eq!(px(-10.5, 2.0), -21);
    assert_eq!(px(11.0, 1.25), 14);
    assert_eq!(island_page_url(7, None).as_str(), "http://desktop-tools-island.localhost/index.html?island=1&boot=7");
    let stylesheet = "/plugins/主题 + 空格/theme.css?v=a&x=%20"; // 查询转义不能把主题地址拆成岛参数。
    let url = island_page_url(9, Some(stylesheet));
    assert_eq!(url.query_pairs().find(|(key, _)| key == "theme-css").unwrap().1, stylesheet);
    assert_eq!(url.query_pairs().count(), 3);
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
fn capsule_keeps_six_dip_margin_and_notch_touches_edge()
{
    let area = work(0.0, 0.0, 1000.0, 800.0); // 常规工作区。
    let size = (100.0, 50.0); // 岛的实际测量尺寸。
    assert_eq!(island_margin("capsule"), 6.0);
    assert_eq!(island_margin("notch"), 0.0);
    assert_eq!(place(IslandEdge::Top, 0.0, size, area, island_margin("capsule")).top, 6.0);
    assert_eq!(place(IslandEdge::Bottom, 0.0, size, area, island_margin("capsule")).top, 800.0 - 50.0 - 6.0);
    assert_eq!(place(IslandEdge::Left, 0.0, size, area, island_margin("capsule")).left, 6.0);
    assert_eq!(place(IslandEdge::Right, 0.0, size, area, island_margin("capsule")).left, 1000.0 - 100.0 - 6.0);
    assert_eq!(place(IslandEdge::Top, 0.0, size, area, island_margin("notch")).top, 0.0);
    assert_eq!(place(IslandEdge::Bottom, 0.0, size, area, island_margin("notch")).top, 750.0);
    assert_eq!(place(IslandEdge::Left, 0.0, size, area, island_margin("notch")).left, 0.0);
    assert_eq!(place(IslandEdge::Right, 0.0, size, area, island_margin("notch")).left, 900.0);
}

#[test]
fn clamps_offsets_at_both_ends_and_places_all_edges()
{
    let area = work(-200.0, 40.0, 400.0, 300.0); // 带负坐标的工作区。
    let size = (100.0, 50.0); // 岛的实际测量尺寸。
    for edge in [IslandEdge::Top, IslandEdge::Bottom, IslandEdge::Left, IslandEdge::Right]
    {
        let maximum = if matches!(edge, IslandEdge::Top | IslandEdge::Bottom) { 150.0 } else { 125.0 }; // 相对居中的半幅偏移。
        assert_eq!(clamp_offset(edge, -999.0, size, area), -maximum);
        assert_eq!(clamp_offset(edge, 999.0, size, area), maximum);
        assert_eq!(clamp_offset(edge, 12.5, size, area), 12.5);
    }
    assert_eq!(place(IslandEdge::Top, 0.0, size, area, 0.0), work(-50.0, 40.0, 100.0, 50.0)); // 沿水平方向居中。
    assert_eq!(place(IslandEdge::Top, -5.0, size, area, 0.0), work(-55.0, 40.0, 100.0, 50.0));
    assert_eq!(place(IslandEdge::Bottom, 999.0, size, area, 0.0), work(100.0, 290.0, 100.0, 50.0));
    assert_eq!(place(IslandEdge::Left, 999.0, size, area, 0.0), work(-200.0, 290.0, 100.0, 50.0));
    assert_eq!(place(IslandEdge::Right, -5.0, size, area, 0.0), work(100.0, 160.0, 100.0, 50.0));
}

#[test]
fn oversized_island_uses_work_origin_without_shrinking()
{
    let area = work(-100.0, -20.0, 40.0, 30.0); // 比岛小的退化工作区。
    for edge in [IslandEdge::Top, IslandEdge::Bottom, IslandEdge::Left, IslandEdge::Right]
    {
        assert_eq!(clamp_offset(edge, 999.0, (100.0, 50.0), area), 0.0);
        assert_eq!(place(edge, 999.0, (100.0, 50.0), area, 0.0), work(-100.0, -20.0, 100.0, 50.0));
    }
}

#[test]
fn preferences_round_trip_and_empty_uses_top_with_offset()
{
    for edge in [IslandEdge::Top, IslandEdge::Bottom, IslandEdge::Left, IslandEdge::Right]
    {
        assert_eq!(IslandEdge::from_preference(edge.as_preference()), Some(edge));
    }
    let area = work(-400.0, 40.0, 1000.0, 600.0); // 默认居中也以工作区起点为基准。
    assert_eq!(IslandEdge::from_preference(""), Some(IslandEdge::Top));
    assert_eq!(IslandEdge::from_preference("TOP"), None);
    assert_eq!(place_from_preference("", 999.0, (200.0, 50.0), area, 0.0), Some(work(400.0, 40.0, 200.0, 50.0))); // 空边按顶部夹紧偏移。
    assert_eq!(place_from_preference("", 0.0, (200.0, 50.0), area, 0.0), Some(work(0.0, 40.0, 200.0, 50.0))); // 空边偏移 0 居中。
    assert_eq!(place_from_preference("top", 0.0, (200.0, 50.0), area, 0.0), Some(work(0.0, 40.0, 200.0, 50.0))); // 显式 top 也水平居中。
    assert_eq!(place_from_preference("unknown", 0.0, (200.0, 50.0), area, 0.0), None);
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

/// 同一窗口盒的动画帧不能重复改原生尺寸；固定视口让子窗口随父窗口反向偏移。
#[test]
fn surface_plan_skips_repeated_frames_and_keeps_fixed_child_origin()
{
    let first = AppliedSurface::default().plan((10, 20, 300, 72), (-10, -20, 420, 1040));
    assert!(first.1 && first.2, "首帧必须同时下发窗口与子窗口");
    assert_eq!(first.0.window, Some((10, 20, 300, 72)));
    assert_eq!(first.0.child, Some((-10, -20, 420, 1040)));
    let again = first.0.plan((10, 20, 300, 72), (-10, -20, 420, 1040));
    assert!(!again.1 && !again.2, "同尺寸动画帧不能再次 SetWindowPos/SetBounds");
    // 窗口原点移动时，子窗口偏移反向变化以保持内容屏幕位置不变，尺寸不变。
    let moved = first.0.plan((40, 20, 300, 72), (-40, -20, 420, 1040));
    assert!(moved.1 && moved.2);
    assert_eq!(moved.0.child, Some((-40, -20, 420, 1040)));
    // 窗口收缩也同时改窗口与子窗口偏移。
    let shrunk = moved.0.plan((60, 20, 148, 44), (-60, -20, 420, 1040));
    assert!(shrunk.1 && shrunk.2);
    assert_eq!(shrunk.0.child, Some((-60, -20, 420, 1040)));
    // 视口变化只改子窗口，窗口不动。
    let viewport_changed = shrunk.0.plan((60, 20, 148, 44), (-100, -20, 420, 1040));
    assert!(!viewport_changed.1 && viewport_changed.2);
    // 拖动只失效窗口缓存；下一份提交即使窗口相同也要重新定位。
    let mut dragged = viewport_changed.0;
    dragged.forget_window();
    let relocated = dragged.plan((60, 20, 148, 44), (-100, -20, 420, 1040));
    assert!(relocated.1);
    // 失败收口清空缓存后必须重新下发。
    let reset = AppliedSurface::default().plan((60, 20, 148, 44), (-100, -20, 420, 1040));
    assert!(reset.1 && reset.2);
}

/// 岛没有用户关闭入口：外部关闭请求不得销毁窗口，窗口被外部销毁后事件投递要清理并重建。
#[test]
fn island_window_rejects_external_close_and_recovers_disposed_webview()
{
    let source = include_str!("island.rs"); // 核对实际编译源码，不依赖真机复现。
    assert!(source.contains("if message == WM_CLOSE"));
    assert!(source.contains("岛：忽略外部关闭请求"));
    assert!(source.contains("if !unsafe { IsWindow(Some(hwnd)) }.as_bool()"));
    assert!(source.contains("forget_disposed_window()"));
    assert!(source.contains("reconcile(&app)")); // 清理后按当前偏好重建，不靠下一次偏好保存。
}
