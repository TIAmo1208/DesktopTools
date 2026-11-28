//! `render_tree::adapter` 单测：每类节点的命令映射、令牌表、超限兜底与端到端。

use serde_json::json;

use super::adapter::{Adapted, adapt_tree, adapt_value, convert};
use super::draw::{DrawList, Primitive, Rect};
use super::error::RenderTreeError;
use super::host::{DEFAULT_ENTRY, TileRender, TileRequest, render_tile};
use super::node::{LimitReason, Node, NodeKind};
use super::palette::TokenPalette;
use super::tokens::FENCE_TOKENS;
use crate::render::draw::{Color, DrawCommand, Rect as CommandRect, TextAlign as CommandTextAlign};

/// 默认画布。
fn bounds() -> Rect { Rect::new(0.0, 0.0, 320.0, 200.0) }

/// 深色内置令牌表。
fn palette() -> TokenPalette { TokenPalette::dark() }

/// 适配一棵 JSON 树。
fn adapt(value: &serde_json::Value) -> Adapted { adapt_value(value, bounds(), &palette()).expect("适配应成功") }

/// 取命令序列。
fn commands(value: &serde_json::Value) -> Vec<DrawCommand> { adapt(value).commands.commands().to_vec() }

/// 内置颜色：深色 surface。
fn surface() -> Color { Color::rgba(32, 38, 40, 148) }

/// 内置颜色：深色 border。
fn border() -> Color { Color::rgba(210, 224, 227, 97) }

/// 内置颜色：深色 active。
fn active() -> Color { Color::rgba(69, 201, 140, 255) }

/// 内置颜色：深色 muted。
fn muted() -> Color { Color::rgba(168, 180, 182, 255) }

/// 内置颜色：深色 warning。
fn warning() -> Color { Color::rgba(232, 163, 63, 255) }

/// 内置颜色：深色 text。
fn text() -> Color { Color::rgba(232, 239, 240, 255) }

/// 容器表面：填充 + 描边。
#[test]
fn column_surface_emits_fill_and_stroke()
{
    let value = json!({ "type": "column", "background": "--fence-surface", "border": "--fence-border", "borderWidth": 2, "radius": 10, "children": [ { "type": "text", "text": "内容" } ] });
    let commands = commands(&value);
    assert_eq!(commands[0], DrawCommand::FillRoundRect { rect: CommandRect::new(0.0, 0.0, 320.0, 200.0), radius: 10.0, color: surface() });
    assert_eq!(commands[1], DrawCommand::StrokeRoundRect { rect: CommandRect::new(0.0, 0.0, 320.0, 200.0), radius: 10.0, width: 2.0, color: border() });
    assert!(matches!(commands[2], DrawCommand::Text { .. }), "{:?}", commands[2]);
}

/// 文本：字号档位、字重、对齐、截断行数与内容。
#[test]
fn text_emits_size_weight_align_and_max_lines()
{
    let value = json!({ "type": "column", "children": [ { "type": "text", "text": "两行截断的说明文字", "size": "title", "weight": "semibold", "textAlign": "center", "truncateLines": 2, "tone": "--fence-text" } ] });
    match &commands(&value)[0]
    {
        DrawCommand::Text { text: content, size, weight, color, align, max_lines, .. } =>
        {
            assert_eq!(content, "两行截断的说明文字");
            assert_eq!(*size, 18.0);
            assert_eq!(*weight, 600);
            assert_eq!(*color, text());
            assert_eq!(*align, CommandTextAlign::Center);
            assert_eq!(*max_lines, 2);
        }
        other => panic!("应为文本命令：{other:?}"),
    }
}

/// 分隔线：1 DIP 细条、直角、边框色。
#[test]
fn divider_emits_thin_rect()
{
    let value = json!({ "type": "column", "children": [ { "type": "divider", "tone": "--fence-border" } ] });
    assert_eq!(commands(&value)[0], DrawCommand::FillRoundRect { rect: CommandRect::new(0.0, 0.0, 320.0, 1.0), radius: 0.0, color: border() });
}

/// 状态点：圆角半径取短边一半、颜色取令牌。
#[test]
fn status_dot_emits_circle()
{
    let value = json!({ "type": "column", "children": [ { "type": "status_dot", "tone": "--fence-warning", "width": 8, "height": 8 } ] });
    assert_eq!(commands(&value)[0], DrawCommand::FillRoundRect { rect: CommandRect::new(0.0, 0.0, 8.0, 8.0), radius: 4.0, color: warning() });
}

/// 进度：轨道 + 按比例截宽的填充。
#[test]
fn progress_emits_track_and_fill()
{
    let value = json!({ "type": "column", "children": [ { "type": "progress", "value": 0.5, "height": 6, "tone": "--fence-active", "background": "--fence-muted" } ] });
    let commands = commands(&value);
    assert_eq!(commands.len(), 2, "{commands:?}");
    assert_eq!(commands[0], DrawCommand::FillRoundRect { rect: CommandRect::new(0.0, 0.0, 320.0, 6.0), radius: 3.0, color: muted() });
    assert_eq!(commands[1], DrawCommand::FillRoundRect { rect: CommandRect::new(0.0, 0.0, 160.0, 6.0), radius: 3.0, color: active() });
}

/// 环：24 段近似，前一半用填充色、其余用轨道色。
#[test]
fn ring_approximates_with_segments()
{
    let value = json!({ "type": "column", "children": [ { "type": "ring", "value": 0.5, "width": 44, "height": 44, "tone": "--fence-active", "background": "--fence-muted" } ] });
    let commands = commands(&value);
    assert_eq!(commands.len(), 24, "{commands:?}");
    let colors: Vec<Color> = commands
        .iter()
        .map(|command| match command
        {
            DrawCommand::FillRoundRect { color, .. } => *color,
            other => panic!("环应只发填充圆点：{other:?}"),
        })
        .collect();
    assert_eq!(colors.iter().filter(|color| **color == active()).count(), 12);
    assert_eq!(colors.iter().filter(|color| **color == muted()).count(), 12);
    // 圆点都落在节点矩形内。
    for command in &commands
    {
        if let DrawCommand::FillRoundRect { rect, .. } = command
        {
            assert!(rect.x >= -0.01 && rect.x <= 44.0 && rect.y >= -0.01 && rect.y <= 44.0, "{rect:?}");
        }
    }
}

/// 迷你折线：每个采样点一个小圆点，数值越大越靠上。
#[test]
fn sparkline_emits_sample_dots()
{
    let value = json!({ "type": "column", "children": [ { "type": "sparkline", "values": [0.0, 0.5, 1.0], "height": 28, "tone": "--fence-active" } ] });
    let commands = commands(&value);
    assert_eq!(commands.len(), 3, "{commands:?}");
    let rects: Vec<CommandRect> = commands
        .iter()
        .map(|command| match command
        {
            DrawCommand::FillRoundRect { rect, .. } => *rect,
            other => panic!("折线采样点应为填充圆点：{other:?}"),
        })
        .collect();
    assert!(rects[0].y > rects[1].y && rects[1].y > rects[2].y, "{rects:?}");
}

/// 条带：每个数值一根柱，高度与数值成正比。
#[test]
fn bars_emit_one_rect_per_value()
{
    let value = json!({ "type": "column", "children": [ { "type": "bars", "values": [0.5, 1.0], "height": 40, "tone": "--fence-active" } ] });
    let commands = commands(&value);
    assert_eq!(commands.len(), 2, "{commands:?}");
    let rects: Vec<CommandRect> = commands
        .iter()
        .map(|command| match command
        {
            DrawCommand::FillRoundRect { rect, .. } => *rect,
            other => panic!("条带应为填充矩形：{other:?}"),
        })
        .collect();
    assert!((rects[0].height - 20.0).abs() < 0.01 && (rects[0].y - 20.0).abs() < 0.01, "{rects:?}");
    assert!((rects[1].height - 40.0).abs() < 0.01 && rects[1].y.abs() < 0.01, "{rects:?}");
}

/// 图标与空态：占位框（填充 + 描边）。
#[test]
fn icon_and_empty_emit_placeholder_box()
{
    for kind in ["icon", "image", "empty"]
    {
        let value = json!({ "type": "column", "children": [ { "type": kind, "width": 20, "height": 20, "radius": 4 } ] });
        let commands = commands(&value);
        assert_eq!(commands.len(), 2, "{kind}: {commands:?}");
        assert_eq!(commands[0], DrawCommand::FillRoundRect { rect: CommandRect::new(0.0, 0.0, 20.0, 20.0), radius: 4.0, color: surface() });
        assert_eq!(commands[1], DrawCommand::StrokeRoundRect { rect: CommandRect::new(0.0, 0.0, 20.0, 20.0), radius: 4.0, width: 1.0, color: border() });
    }
}

/// 徽标：底盒 + 居中单行文字。
#[test]
fn badge_emits_box_and_centered_text()
{
    let value = json!({ "type": "column", "children": [ { "type": "badge", "text": "在线", "background": "--fence-surface", "radius": 6, "tone": "--fence-text" } ] });
    let commands = commands(&value);
    assert!(matches!(commands[0], DrawCommand::FillRoundRect { radius: 6.0, .. }), "{:?}", commands[0]);
    match &commands[1]
    {
        DrawCommand::Text { text: content, align, max_lines, .. } =>
        {
            assert_eq!(content, "在线");
            assert_eq!(*align, CommandTextAlign::Center);
            assert_eq!(*max_lines, 1);
        }
        other => panic!("徽标文字应为文本命令：{other:?}"),
    }
}

/// 链接：文本 + 底边 1 DIP 下划线。
#[test]
fn link_emits_text_with_underline()
{
    let value = json!({ "type": "column", "children": [ { "type": "link", "text": "打开设置", "tone": "--fence-text" } ] });
    let commands = commands(&value);
    assert!(matches!(&commands[0], DrawCommand::Text { text: content, .. } if content == "打开设置"), "{:?}", commands[0]);
    match &commands[1]
    {
        DrawCommand::FillRoundRect { rect, radius, color } =>
        {
            assert!((rect.height - 1.0).abs() < 0.01);
            assert_eq!(*radius, 0.0);
            assert_eq!(*color, text());
        }
        other => panic!("下划线应为细填充矩形：{other:?}"),
    }
}

/// 滚动容器：裁剪栈成对出现。
#[test]
fn scroll_emits_clip_pair()
{
    let value = json!({ "type": "scroll", "height": 60, "children": [ { "type": "text", "text": "第一行" } ] });
    let adapted = adapt(&value);
    let commands = adapted.commands.commands();
    assert!(matches!(commands.first(), Some(DrawCommand::PushClip { .. })), "{commands:?}");
    assert!(matches!(commands.last(), Some(DrawCommand::PopClip)), "{commands:?}");
    assert!(adapted.commands.is_balanced());
}

/// 空白占位不产生命令。
#[test]
fn spacer_emits_nothing()
{
    let value = json!({ "type": "column", "children": [ { "type": "spacer" } ] });
    assert_eq!(adapt(&value).command_count(), 0);
}

/// 不支持的占位图元：明确报错并带路径，不静默跳过。
#[test]
fn unsupported_placeholder_is_rejected()
{
    let mut draw = DrawList::new();
    draw.push(Primitive::Placeholder { rect: Rect::new(0.0, 0.0, 16.0, 16.0), radius: 0.0, kind: NodeKind::Bars });
    match convert(&draw, &palette()).expect_err("应报不支持")
    {
        RenderTreeError::UnsupportedPrimitive { path, detail } =>
        {
            assert_eq!(path, "$");
            assert!(detail.contains("占位图元只支持"), "{detail}");
        }
        other => panic!("错误类型不符：{other:?}"),
    }
}

/// 令牌表缺令牌：报错并指出节点路径。
#[test]
fn unknown_token_reports_node_path()
{
    let value = json!({ "type": "column", "children": [ { "type": "status_dot", "tone": "--fence-warning", "width": 8, "height": 8 } ] });
    match adapt_value(&value, bounds(), &TokenPalette::empty()).expect_err("缺令牌应报错")
    {
        RenderTreeError::PaletteMiss { path, token } =>
        {
            assert_eq!(path, "$.children[0]");
            assert_eq!(token, "--fence-warning");
        }
        other => panic!("错误类型不符：{other:?}"),
    }
}

/// 裸色值继续在解析层被拒绝（适配层入口也走同一校验）。
#[test]
fn bare_color_is_rejected()
{
    let value = json!({ "type": "column", "background": "#101010", "children": [] });
    match adapt_value(&value, bounds(), &palette()).expect_err("裸色值应报错")
    {
        RenderTreeError::BareColor { path, field, value } =>
        {
            assert_eq!(path, "$");
            assert_eq!(field, "background");
            assert_eq!(value, "#101010");
        }
        other => panic!("错误类型不符：{other:?}"),
    }
}

/// 超过 5000 节点：不产出任何命令，返回兜底标记。
#[test]
fn over_limit_tree_emits_no_commands()
{
    let children: Vec<serde_json::Value> = (0..5001).map(|index| json!({ "type": "text", "text": format!("n{index}") })).collect();
    let value = json!({ "type": "column", "children": children });
    let adapted = adapt(&value);
    assert_eq!(adapted.command_count(), 0);
    assert!(adapted.is_rejected());
    assert!(matches!(adapted.rejection, Some(LimitReason::TooManyNodes { nodes: 5001, .. })));
    assert!(adapted.fallback.expect("应有兜底节点").text.expect("兜底句").contains("超出渲染上限"));
}

/// 深度 33：同样不产出命令。
#[test]
fn over_depth_tree_emits_no_commands()
{
    let mut value = json!({ "type": "text", "text": "leaf" });
    for _ in 1..33
    {
        value = json!({ "type": "column", "children": [ value ] });
    }
    let adapted = adapt(&value);
    assert_eq!(adapted.command_count(), 0);
    assert!(matches!(adapted.rejection, Some(LimitReason::TooDeep { depth: 33, .. })));
}

/// 已构造的树走 `adapt_tree` 时也执行限额判定。
#[test]
fn adapt_tree_also_rejects_over_limit()
{
    let mut tree = Node::container(NodeKind::Column, vec![Node::text("x")]);
    tree.children = (0..5000).map(|_| Node::text("x")).collect(); // 合计 5001 个节点。
    let adapted = adapt_tree(&tree, bounds(), &palette()).expect("超限不是契约错误");
    assert_eq!(adapted.command_count(), 0);
    assert!(adapted.is_rejected());
}

/// 端到端：内联 JS 插件 → 树 → 命令列表。
#[test]
fn end_to_end_inline_plugin_to_commands()
{
    let module = r#"
export default function render() {
    return {
        type: "column",
        padding: 8,
        gap: 4,
        background: "--fence-surface",
        radius: 8,
        children: [
            { type: "text", text: "DSH 接入", tone: "--fence-text" },
            { type: "progress", value: 0.5, height: 6, tone: "--fence-active" }
        ]
    };
}
"#;
    let request = TileRequest::new("tile-1", "status");
    let render = render_tile(module, "status-tile.js", DEFAULT_ENTRY, &request).expect("插件应成功");
    let tree = match render
    {
        TileRender::Tree { tree, .. } => tree,
        other => panic!("应渲染出树：{other:?}"),
    };
    let adapted = adapt_tree(&tree, bounds(), &palette()).expect("适配应成功");
    let commands = adapted.commands.commands();
    assert_eq!(commands.len(), 4, "{commands:?}");
    assert_eq!(commands[0], DrawCommand::FillRoundRect { rect: CommandRect::new(0.0, 0.0, 320.0, 200.0), radius: 8.0, color: surface() });
    match &commands[1]
    {
        DrawCommand::Text { text: content, rect, .. } =>
        {
            assert_eq!(content, "DSH 接入");
            assert!((rect.x - 8.0).abs() < 0.01 && (rect.y - 8.0).abs() < 0.01, "{rect:?}");
        }
        other => panic!("第二条应为文本：{other:?}"),
    }
    let progress_y = commands[2].rect_of().y; // 进度条的纵向位置由布局给出，与文本高度相关。
    assert_eq!(commands[2], DrawCommand::FillRoundRect { rect: CommandRect::new(8.0, progress_y, 304.0, 6.0), radius: 3.0, color: muted() });
    assert_eq!(commands[3], DrawCommand::FillRoundRect { rect: CommandRect::new(8.0, progress_y, 152.0, 6.0), radius: 3.0, color: active() });
}

/// 内置浅/深两套颜色表都覆盖契约里的全部桌面层令牌。
#[test]
fn builtin_palettes_cover_all_tokens()
{
    for palette in [TokenPalette::dark(), TokenPalette::light()]
    {
        for token in FENCE_TOKENS
        {
            assert!(palette.resolve_name(token).is_some(), "缺少令牌 {token}");
        }
    }
}

/// 便于测试取命令矩形的小工具。
trait CommandRectExt
{
    fn rect_of(&self) -> CommandRect;
}

impl CommandRectExt for DrawCommand
{
    /// 取命令的矩形；无矩形的命令直接失败。
    fn rect_of(&self) -> CommandRect
    {
        match self
        {
            DrawCommand::FillRoundRect { rect, .. } | DrawCommand::StrokeRoundRect { rect, .. } | DrawCommand::Text { rect, .. } | DrawCommand::Caption { rect, .. } | DrawCommand::PushClip { rect } => *rect,
            other => panic!("该命令没有矩形：{other:?}"),
        }
    }
}
