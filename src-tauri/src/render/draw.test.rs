//! `render::draw` 的指令模型单测：序列化往返、栈配对、颜色换算。

use super::draw::{Color, DrawCommand, DrawList, Rect, TextAlign};

/// 混合指令序列经 JSON 往返后必须完全相等（跨进程/跨语言传绘制列表的前提）。
#[test]
fn draw_list_survives_json_round_trip()
{
    let mut list = DrawList::new();
    list.push(DrawCommand::FillRoundRect { rect: Rect::new(8.0, 8.0, 240.0, 120.0), radius: 12.0, color: Color::rgba(0x1E, 0x1E, 0x24, 184) });
    list.push(DrawCommand::StrokeRoundRect { rect: Rect::new(8.0, 8.0, 240.0, 120.0), radius: 12.0, width: 1.5, color: Color::rgba(0, 0, 0, 46) });
    list.push(DrawCommand::PushClip { rect: Rect::new(12.0, 12.0, 200.0, 60.0) });
    list.push(DrawCommand::Text { rect: Rect::new(16.0, 16.0, 180.0, 24.0), text: "原生宿主首测".to_string(), size: 15.0, weight: 600, color: Color::rgba(0xFF, 0xFF, 0xFF, 255), align: TextAlign::Center, max_lines: 1 });
    list.push(DrawCommand::PopClip);
    list.push(DrawCommand::PushOpacity { opacity: 0.5 });
    list.push(DrawCommand::PopOpacity);

    let encoded = serde_json::to_string(&list).expect("序列化失败");
    let decoded: DrawList = serde_json::from_str(&encoded).expect("反序列化失败");
    assert_eq!(decoded, list);
    assert_eq!(decoded.len(), 7);
    assert!(decoded.is_balanced());
}

/// 指令以 `op` 判别字段区分类型，便于后续跨进程协议演进。
#[test]
fn draw_command_serializes_with_op_tag()
{
    let command = DrawCommand::FillRoundRect { rect: Rect::new(0.0, 0.0, 10.0, 10.0), radius: 4.0, color: Color::rgba(1, 2, 3, 4) };
    let value = serde_json::to_value(&command).expect("序列化失败");
    assert_eq!(value["op"], "fill_round_rect");

    let text = DrawCommand::Text { rect: Rect::new(0.0, 0.0, 10.0, 10.0), text: "中文".to_string(), size: 12.0, weight: 400, color: Color::TRANSPARENT, align: TextAlign::End, max_lines: 2 };
    assert_eq!(serde_json::to_value(&text).expect("序列化失败")["align"], "end");
}

/// 裁剪/透明度必须严格配对：多一个 Pop 或漏一个 Pop 都要判定为不成立。
#[test]
fn unbalanced_stacks_are_rejected()
{
    let mut extra_pop = DrawList::new();
    extra_pop.push(DrawCommand::PopClip);
    assert!(!extra_pop.is_balanced());

    let mut missing_pop = DrawList::new();
    missing_pop.push(DrawCommand::PushOpacity { opacity: 0.3 });
    assert!(!missing_pop.is_balanced());

    let mut nested = DrawList::new();
    nested.push(DrawCommand::PushClip { rect: Rect::new(0.0, 0.0, 1.0, 1.0) });
    nested.push(DrawCommand::PushOpacity { opacity: 1.0 });
    nested.push(DrawCommand::PopOpacity);
    nested.push(DrawCommand::PopClip);
    assert!(nested.is_balanced());
}

/// 颜色按 `0xRRGGBBAA` 拆分量，透明度可单独替换。
#[test]
fn color_splits_into_unit_components()
{
    let color = Color::rgba(0x33, 0x66, 0x99, 0xCC);
    assert_eq!(color.0, 0x3366_99CC);
    let unit = color.to_unit();
    assert!((unit[0] - 0.2).abs() < 0.002);
    assert!((unit[1] - 0.4).abs() < 0.002);
    assert!((unit[2] - 0.6).abs() < 0.002);
    assert!((unit[3] - 0.8).abs() < 0.002);
    assert_eq!(color.with_alpha(0.5), Color::rgba(0x33, 0x66, 0x99, 0x80));
    assert_eq!(Color::TRANSPARENT.to_unit()[3], 0.0);
}

/// 矩形包含同样是左闭右开，且负尺寸先归一化。
#[test]
fn rect_contains_matches_half_open_convention()
{
    let rect = Rect::new(10.0, 20.0, 100.0, 50.0);
    assert!(rect.contains(10.0, 20.0));
    assert!(rect.contains(109.9, 69.9));
    assert!(!rect.contains(110.0, 40.0));
    assert!(!rect.contains(50.0, 70.0));
    assert_eq!(Rect::new(0.0, 0.0, -5.0, 10.0).normalized(), Rect::new(0.0, 0.0, 0.0, 10.0));
    assert_eq!(rect.right(), 110.0);
    assert_eq!(rect.bottom(), 70.0);
}
