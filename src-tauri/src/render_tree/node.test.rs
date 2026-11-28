//! `render_tree::node` 单测：契约白名单校验、缺字段与裸色值报错、节点与深度上限。

use serde_json::json;

use super::error::RenderTreeError;
use super::node::{Dimension, LimitReason, MAX_DEPTH, MAX_NODES, Node, NodeKind, TextSize, fallback_node, parse_tree};

/// 最小文本节点：统计为 1 节点 1 层。
#[test]
fn parses_minimal_text_node()
{
    let value = json!({ "type": "text", "text": "CPU", "size": "caption", "tone": "--fence-muted" });
    let parsed = parse_tree(&value).expect("应通过校验");
    assert!(parsed.is_accepted());
    assert_eq!(parsed.stats.nodes, 1);
    assert_eq!(parsed.stats.depth, 1);
    let node = parsed.tree.expect("有树");
    assert_eq!(node.kind, NodeKind::Text);
    assert_eq!(node.text.as_deref(), Some("CPU"));
    assert_eq!(node.style.size, Some(TextSize::Caption));
    assert_eq!(node.tone.expect("有令牌").as_str(), "--fence-muted");
}

/// 容器递归统计节点数与深度。
#[test]
fn counts_nodes_and_depth_across_containers()
{
    let value = json!({ "type": "column", "children": [ { "type": "row", "children": [ { "type": "text", "text": "a" }, { "type": "text", "text": "b" } ] } ] });
    let parsed = parse_tree(&value).expect("应通过校验");
    assert_eq!(parsed.stats.nodes, 4);
    assert_eq!(parsed.stats.depth, 3);
}

/// 背景写裸色值必须报错，并带上字段路径。
#[test]
fn rejects_bare_color_with_path()
{
    let value = json!({ "type": "column", "children": [ { "type": "text", "text": "a", "background": "#101010" } ] });
    match parse_tree(&value).expect_err("应报错")
    {
        RenderTreeError::BareColor { path, field, value } =>
        {
            assert_eq!(path, "$.children[0]");
            assert_eq!(field, "background");
            assert_eq!(value, "#101010");
        }
        other => panic!("错误类型不符：{other:?}"),
    }
}

/// 界面调色板令牌不在桌面层白名单，必须报错而不是回退。
#[test]
fn rejects_palette_token()
{
    let value = json!({ "type": "text", "text": "a", "tone": "--color-text" });
    match parse_tree(&value).expect_err("应报错")
    {
        RenderTreeError::UnknownToken { field, value, .. } =>
        {
            assert_eq!(field, "tone");
            assert_eq!(value, "--color-text");
        }
        other => panic!("错误类型不符：{other:?}"),
    }
}

/// 未知字段直接拒绝，不忽略。
#[test]
fn rejects_unknown_field()
{
    let value = json!({ "type": "text", "text": "a", "shadow": 4 });
    match parse_tree(&value).expect_err("应报错")
    {
        RenderTreeError::UnknownField { path, field } =>
        {
            assert_eq!(path, "$");
            assert_eq!(field, "shadow");
        }
        other => panic!("错误类型不符：{other:?}"),
    }
}

/// 文本节点缺 `text`：明确报缺字段。
#[test]
fn rejects_missing_text()
{
    let value = json!({ "type": "text", "size": "body" });
    assert_eq!(parse_tree(&value).expect_err("应报错"), RenderTreeError::MissingField { path: "$".to_string(), field: "text" });
}

/// 容器缺 `children`：明确报缺字段。
#[test]
fn rejects_missing_children()
{
    let value = json!({ "type": "column" });
    assert_eq!(parse_tree(&value).expect_err("应报错"), RenderTreeError::MissingField { path: "$".to_string(), field: "children" });
}

/// 文本节点写 `children`：拒绝（避免写了却不生效）。
#[test]
fn rejects_children_on_leaf()
{
    let value = json!({ "type": "text", "text": "a", "children": [] });
    match parse_tree(&value).expect_err("应报错")
    {
        RenderTreeError::InvalidValue { field, .. } => assert_eq!(field, "children"),
        other => panic!("错误类型不符：{other:?}"),
    }
}

/// 容器写 `text`：拒绝。
#[test]
fn rejects_text_on_container()
{
    let value = json!({ "type": "row", "text": "a", "children": [] });
    match parse_tree(&value).expect_err("应报错")
    {
        RenderTreeError::InvalidValue { field, .. } => assert_eq!(field, "text"),
        other => panic!("错误类型不符：{other:?}"),
    }
}

/// 进度值超出 0–1：拒绝。
#[test]
fn rejects_value_out_of_range()
{
    let value = json!({ "type": "progress", "value": 1.4 });
    match parse_tree(&value).expect_err("应报错")
    {
        RenderTreeError::InvalidValue { field, detail, .. } =>
        {
            assert_eq!(field, "value");
            assert!(detail.contains("0–1"), "{detail}");
        }
        other => panic!("错误类型不符：{other:?}"),
    }
}

/// 未知节点类型：拒绝并回显原文。
#[test]
fn rejects_unknown_node_type()
{
    let value = json!({ "type": "canvas", "text": "a" });
    match parse_tree(&value).expect_err("应报错")
    {
        RenderTreeError::UnknownNodeType { value, .. } => assert_eq!(value, "canvas"),
        other => panic!("错误类型不符：{other:?}"),
    }
}

/// 5001 个节点：拒绝渲染，返回上限原因（树为 None），统计到 5001。
#[test]
fn rejects_tree_over_node_limit()
{
    let children: Vec<serde_json::Value> = (0..MAX_NODES + 1).map(|index| json!({ "type": "text", "text": format!("n{index}") })).collect();
    let value = json!({ "type": "column", "children": children });
    let parsed = parse_tree(&value).expect("上限不是契约错误");
    assert!(!parsed.is_accepted());
    assert!(parsed.tree.is_none());
    assert_eq!(parsed.stats.nodes, MAX_NODES + 1);
    assert_eq!(parsed.limit, Some(LimitReason::TooManyNodes { nodes: MAX_NODES + 1, limit: MAX_NODES }));
}

/// 恰好 5000 个节点：允许渲染。
#[test]
fn accepts_tree_at_node_limit()
{
    let children: Vec<serde_json::Value> = (0..MAX_NODES - 1).map(|index| json!({ "type": "text", "text": format!("n{index}") })).collect();
    let value = json!({ "type": "column", "children": children });
    let parsed = parse_tree(&value).expect("应通过校验");
    assert!(parsed.is_accepted());
    assert_eq!(parsed.stats.nodes, MAX_NODES);
}

/// 深度 33：拒绝渲染。
#[test]
fn rejects_tree_over_depth_limit()
{
    let mut value = json!({ "type": "text", "text": "leaf" });
    for _ in 1..MAX_DEPTH + 1
    {
        value = json!({ "type": "column", "children": [ value ] });
    }
    let parsed = parse_tree(&value).expect("上限不是契约错误");
    assert!(!parsed.is_accepted());
    assert_eq!(parsed.stats.depth, MAX_DEPTH + 1);
    assert_eq!(parsed.limit, Some(LimitReason::TooDeep { depth: MAX_DEPTH + 1, limit: MAX_DEPTH }));
}

/// 深度 32：允许渲染。
#[test]
fn accepts_tree_at_depth_limit()
{
    let mut value = json!({ "type": "text", "text": "leaf" });
    for _ in 1..MAX_DEPTH
    {
        value = json!({ "type": "column", "children": [ value ] });
    }
    let parsed = parse_tree(&value).expect("应通过校验");
    assert!(parsed.is_accepted());
    assert_eq!(parsed.stats.depth, MAX_DEPTH);
}

/// 节点结构与样式可以完整往返 serde；JSON 里样式字段是扁平的。
#[test]
fn node_survives_json_round_trip()
{
    let mut node = Node::container(
        NodeKind::Row,
        vec![Node::text("余额"), {
            let mut value = Node::text("12.00");
            value.tone = Some(super::tokens::ThemeToken::parse("--fence-active").expect("合法令牌"));
            value.style.mono = Some(true);
            value
        }],
    );
    node.style.gap = Some(8.0);
    node.style.width = Some(Dimension::Fixed(240.0));
    node.style.padding = Some(12.0);
    node.style.truncate_lines = Some(1);

    let encoded = serde_json::to_value(&node).expect("序列化失败");
    assert_eq!(encoded["type"], "row");
    assert_eq!(encoded["gap"], 8.0);
    assert_eq!(encoded["children"][1]["tone"], "--fence-active");
    let decoded: Node = serde_json::from_value(encoded).expect("反序列化失败");
    assert_eq!(decoded, node);
}

/// 兜底节点本身必须通过契约校验，否则拒绝渲染会二次失败。
#[test]
fn fallback_node_is_valid_tree()
{
    let fallback = fallback_node(LimitReason::TooManyNodes { nodes: 5001, limit: MAX_NODES });
    let value = serde_json::to_value(&fallback).expect("序列化失败");
    let parsed = parse_tree(&value).expect("兜底节点应通过校验");
    assert!(parsed.is_accepted());
    assert_eq!(parsed.stats.nodes, 1);
    assert!(parsed.tree.expect("有树").text.expect("有文本").contains("超出渲染上限"));
}
