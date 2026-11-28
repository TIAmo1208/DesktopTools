//! `render_tree::layout` 单测：Column/Row 嵌套、固定与弹性尺寸、gap、padding、截断行数。

use super::draw::{DrawList, Primitive};
use super::layout::{TextMetrics, layout, layout_detailed};
use super::node::{Dimension, FlexWeight, Node, NodeKind};
use super::tokens::ThemeToken;

/// 构造带固定尺寸的文本节点（测试里用固定宽高隔开文本度量误差）。
fn fixed_text(text: &str, width: Option<f32>, height: Option<f32>) -> Node
{
    let mut node = Node::text(text);
    node.style.width = width.map(Dimension::Fixed);
    node.style.height = height.map(Dimension::Fixed);
    node
}

/// 取指定文本的图元。
fn text_primitive<'a>(list: &'a DrawList, text: &str) -> &'a Primitive { list.primitives().iter().find(|primitive| matches!(primitive, Primitive::Text { text: value, .. } if value == text)).unwrap_or_else(|| panic!("找不到文本 {text}")) }

/// 断言矩形，容差 0.01 DIP。
fn assert_rect(primitive: &Primitive, x: f32, y: f32, width: f32, height: f32)
{
    let rect = match primitive
    {
        Primitive::Text { rect, .. } | Primitive::RoundRect { rect, .. } | Primitive::Progress { rect, .. } | Primitive::Ring { rect, .. } | Primitive::StatusDot { rect, .. } | Primitive::Divider { rect, .. } | Primitive::Sparkline { rect, .. } | Primitive::Bars { rect, .. } | Primitive::Placeholder { rect, .. } | Primitive::PushClip { rect } => *rect,
        other => panic!("该图元没有矩形：{other:?}"),
    };
    let close = |left: f32, right: f32| (left - right).abs() < 0.01;
    assert!(close(rect.x, x) && close(rect.y, y) && close(rect.width, width) && close(rect.height, height), "期望 ({x},{y},{width},{height})，实际 ({},{},{},{})", rect.x, rect.y, rect.width, rect.height);
}

/// Column 内嵌 Row：行内固定列 + 弹性列，行下方再放一个文本。
#[test]
fn places_column_nested_row()
{
    let mut row = Node::container(
        NodeKind::Row,
        vec![fixed_text("a", Some(60.0), Some(20.0)), {
            let mut node = Node::text("b");
            node.style.width = Some(Dimension::Flex(FlexWeight { flex: 1.0 }));
            node.style.height = Some(Dimension::Fixed(20.0));
            node
        }],
    );
    row.style.height = Some(Dimension::Fixed(40.0));
    let column = Node::container(NodeKind::Column, vec![row, fixed_text("c", None, Some(30.0))]);

    let list = layout(&column, 200.0, 200.0);

    assert_rect(text_primitive(&list, "a"), 0.0, 0.0, 60.0, 20.0);
    assert_rect(text_primitive(&list, "b"), 60.0, 0.0, 140.0, 20.0);
    assert_rect(text_primitive(&list, "c"), 0.0, 40.0, 200.0, 30.0);
}

/// 行内固定宽度与弹性宽度按剩余空间比例分配。
#[test]
fn distributes_fixed_and_flexible_widths()
{
    let row = Node::container(
        NodeKind::Row,
        vec![
            fixed_text("fixed", Some(60.0), Some(10.0)),
            {
                let mut node = Node::text("flex1");
                node.style.width = Some(Dimension::Flex(FlexWeight { flex: 1.0 }));
                node.style.height = Some(Dimension::Fixed(10.0));
                node
            },
            {
                let mut node = Node::text("flex3");
                node.style.width = Some(Dimension::Flex(FlexWeight { flex: 3.0 }));
                node.style.height = Some(Dimension::Fixed(10.0));
                node
            },
        ],
    );

    let list = layout(&row, 200.0, 60.0);

    assert_rect(text_primitive(&list, "fixed"), 0.0, 0.0, 60.0, 10.0);
    assert_rect(text_primitive(&list, "flex1"), 60.0, 0.0, 35.0, 10.0);
    assert_rect(text_primitive(&list, "flex3"), 95.0, 0.0, 105.0, 10.0);
}

/// 列内 gap 让相邻子节点留出间距。
#[test]
fn gap_offsets_siblings()
{
    let mut column = Node::container(NodeKind::Column, vec![fixed_text("first", None, Some(20.0)), fixed_text("second", None, Some(20.0))]);
    column.style.gap = Some(8.0);

    let list = layout(&column, 120.0, 100.0);

    assert_rect(text_primitive(&list, "first"), 0.0, 0.0, 120.0, 20.0);
    assert_rect(text_primitive(&list, "second"), 0.0, 28.0, 120.0, 20.0);
}

/// padding 把子节点整体内缩，并缩小子节点可用宽度。
#[test]
fn padding_insets_children()
{
    let mut column = Node::container(NodeKind::Column, vec![fixed_text("inside", None, Some(20.0))]);
    column.style.padding = Some(12.0);

    let list = layout(&column, 200.0, 100.0);

    assert_rect(text_primitive(&list, "inside"), 12.0, 12.0, 176.0, 20.0);
}

/// truncateLines 决定文本高度与图元的 max_lines。
#[test]
fn truncate_lines_limits_text_height()
{
    let mut text = Node::text("一段会被截断的说明文字");
    text.style.truncate_lines = Some(2);
    let column = Node::container(NodeKind::Column, vec![text]);

    let metrics = TextMetrics::default();
    let list = layout(&column, 100.0, 100.0);

    let line_px = metrics.line_px(super::node::TextSize::Body);
    match text_primitive(&list, "一段会被截断的说明文字")
    {
        Primitive::Text { rect, max_lines, .. } =>
        {
            assert_eq!(*max_lines, 2);
            assert!((rect.height - 2.0 * line_px).abs() < 0.01, "高度应为两行：{}", rect.height);
        }
        other => panic!("应为文本图元：{other:?}"),
    }
}

/// `visible: false` 的子节点不参与布局，也不产出图元。
#[test]
fn hidden_child_is_skipped()
{
    let mut hidden = fixed_text("hidden", None, Some(20.0));
    hidden.style.visible = Some(false);
    let column = Node::container(NodeKind::Column, vec![hidden, fixed_text("shown", None, Some(20.0))]);

    let list = layout(&column, 100.0, 100.0);

    assert_eq!(list.len(), 1);
    assert_rect(text_primitive(&list, "shown"), 0.0, 0.0, 100.0, 20.0);
}

/// 网格按列数排队，行高取该行最高子节点。
#[test]
fn grid_places_cells_row_by_row()
{
    let mut grid = Node::container(NodeKind::Grid, vec![fixed_text("g1", None, Some(10.0)), fixed_text("g2", None, Some(14.0)), fixed_text("g3", None, Some(10.0))]);
    grid.style.columns = Some(2);
    grid.style.gap = Some(4.0);

    let list = layout(&grid, 200.0, 100.0);

    assert_rect(text_primitive(&list, "g1"), 0.0, 0.0, 98.0, 14.0);
    assert_rect(text_primitive(&list, "g2"), 102.0, 0.0, 98.0, 14.0);
    assert_rect(text_primitive(&list, "g3"), 0.0, 18.0, 98.0, 10.0);
}

/// 容器背景产出圆角矩形图元，颜色是令牌名而不是解析后的色值。
#[test]
fn container_background_emits_round_rect()
{
    let mut column = Node::container(NodeKind::Column, vec![fixed_text("x", None, Some(10.0))]);
    column.style.background = Some(ThemeToken::parse("--fence-surface").expect("合法令牌"));
    column.style.radius = Some(10.0);

    let list = layout(&column, 120.0, 60.0);

    match &list.primitives()[0]
    {
        Primitive::RoundRect { rect, radius, fill, .. } =>
        {
            assert_eq!((rect.x, rect.y, rect.width, rect.height), (0.0, 0.0, 120.0, 60.0));
            assert_eq!(*radius, 10.0);
            assert_eq!(fill.as_ref().expect("有填充令牌").as_str(), "--fence-surface");
        }
        other => panic!("首个图元应为圆角矩形：{other:?}"),
    }
}

/// 透明度与裁剪栈严格配对。
#[test]
fn opacity_and_clip_stacks_are_balanced()
{
    let mut column = Node::container(NodeKind::Column, vec![Node::container(NodeKind::Scroll, vec![fixed_text("s", None, Some(10.0))])]);
    column.style.opacity = Some(0.5);

    let list = layout(&column, 100.0, 100.0);

    assert!(list.is_balanced(), "栈应配对：{:?}", list.validate());
    assert!(matches!(list.primitives().first(), Some(Primitive::PushOpacity { .. })));
    assert!(matches!(list.primitives().last(), Some(Primitive::PopOpacity)));
}

/// 交互命中：声明 `action` 的可见节点按**同一次布局**产出命中矩形；隐藏与零尺寸节点不产出。
#[test]
fn action_nodes_produce_hit_rects()
{
    let mut button = fixed_text("刷新", Some(120.0), Some(24.0));
    button.action = Some("refresh".to_string());
    let mut hidden = fixed_text("隐藏", Some(120.0), Some(24.0));
    hidden.action = Some("ghost".to_string());
    hidden.style.visible = Some(false);
    let mut empty = fixed_text("零高", Some(120.0), Some(0.0));
    empty.action = Some("empty".to_string());
    let root = Node::container(NodeKind::Column, vec![button, hidden, empty]);
    let result = layout_detailed(&root, super::draw::Rect::new(0.0, 0.0, 120.0, 200.0), TextMetrics::default());
    assert_eq!(result.actions.len(), 1, "只有可见且有高度的节点成为命中区");
    let hit = &result.actions[0];
    assert_eq!(hit.action, "refresh", "动作标识原样保留");
    assert_eq!(hit.path, "$.children[0]", "命中区带节点路径");
    assert!((hit.rect.width - 120.0).abs() < 0.01 && (hit.rect.height - 24.0).abs() < 0.01, "命中矩形与该节点绘制矩形一致：{:?}", hit.rect);
    // 命中区与绘制同源：文本图元与命中区矩形重合。
    let text = text_primitive(&result.draw, "刷新");
    assert_rect(text, hit.rect.x, hit.rect.y, hit.rect.width, hit.rect.height);
}

/// 容器同样可以整体作为命中区（列表项整行点击），命中区覆盖容器内容盒。
#[test]
fn container_action_covers_its_box()
{
    let mut row = Node::container(NodeKind::Row, vec![fixed_text("甲", Some(60.0), Some(20.0)), fixed_text("乙", Some(60.0), Some(20.0))]);
    row.action = Some("open-item".to_string());
    let result = layout_detailed(&row, super::draw::Rect::new(10.0, 20.0, 200.0, 40.0), TextMetrics::default());
    assert_eq!(result.actions.len(), 1);
    let hit = &result.actions[0];
    assert_eq!(hit.path, "$", "根节点路径为 $");
    assert!((hit.rect.x - 10.0).abs() < 0.01 && (hit.rect.y - 20.0).abs() < 0.01 && (hit.rect.width - 200.0).abs() < 0.01 && (hit.rect.height - 40.0).abs() < 0.01, "容器命中区是其盒子：{:?}", hit.rect);
}