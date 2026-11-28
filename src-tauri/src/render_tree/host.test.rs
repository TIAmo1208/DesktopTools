//! `render_tree::host` 单测：内联 ESM 插件、浏览器能力隔离、上限拒绝与错误传播。

use super::error::RenderTreeError;
use super::host::{DEFAULT_ENTRY, TileRender, TileRequest, render_tile};
use super::node::LimitReason;

/// 一个只读状态格子的最小 ESM 插件。
const STATUS_TILE: &str = r#"
export default function render(input) {
    return {
        type: "column",
        gap: 6,
        padding: 10,
        background: "--fence-surface",
        radius: 10,
        children: [
            { type: "row", gap: 6, children: [
                { type: "status_dot", tone: "--fence-active" },
                { type: "text", text: "实例 " + input.instanceId, tone: "--fence-text" },
                { type: "spacer" },
                { type: "text", text: input.tileType, tone: "--fence-muted", size: "caption" }
            ]},
            { type: "progress", value: 0.5, height: 6 },
            { type: "sparkline", values: [0.2, 0.6, 0.4], height: 24 }
        ]
    };
}
"#;

/// 内联 ESM 模块可以被加载、求值并调用入口函数。
#[test]
fn loads_inline_esm_module_and_returns_tree()
{
    let request = TileRequest::new("tile-1", "status");
    let render = render_tile(STATUS_TILE, "status-tile.js", DEFAULT_ENTRY, &request).expect("应成功");

    match render
    {
        TileRender::Tree { tree, stats } =>
        {
            assert_eq!(stats.nodes, 8);
            assert_eq!(stats.depth, 3);
            assert_eq!(tree.kind, super::node::NodeKind::Column);
            assert_eq!(tree.children.len(), 3);
        }
        other => panic!("应渲染树：{other:?}"),
    }
}

/// 宿主不向插件暴露 `fetch`、`document`、`window` 等浏览器能力。
#[test]
fn does_not_expose_browser_globals()
{
    let module = r#"
export default function render() {
    const names = ["document", "fetch", "window", "localStorage", "XMLHttpRequest", "setTimeout"];
    return { type: "text", text: names.map((name) => typeof globalThis[name]).join(",") };
}
"#;
    let request = TileRequest::new("tile-1", "status");
    let render = render_tile(module, "probe.js", DEFAULT_ENTRY, &request).expect("应成功");
    match render
    {
        TileRender::Tree { tree, .. } => assert_eq!(tree.text.as_deref(), Some("undefined,undefined,undefined,undefined,undefined,undefined")),
        other => panic!("应渲染树：{other:?}"),
    }
}

/// 自定义入口函数名。
#[test]
fn supports_custom_entry_name()
{
    let module = r#"
export function tile(input) {
    return { type: "text", text: "自定义入口 " + input.tileType };
}
"#;
    let request = TileRequest::new("tile-1", "logs");
    let render = render_tile(module, "custom.js", "tile", &request).expect("应成功");
    match render
    {
        TileRender::Tree { tree, .. } => assert_eq!(tree.text.as_deref(), Some("自定义入口 logs")),
        other => panic!("应渲染树：{other:?}"),
    }
}

/// 超过 5000 个节点：拒绝渲染，返回兜底节点与上限原因。
#[test]
fn rejects_tree_over_node_limit()
{
    let module = r#"
export default function render() {
    const children = [];
    for (let index = 0; index < 5001; index += 1) {
        children.push({ type: "text", text: "n" + index });
    }
    return { type: "column", children };
}
"#;
    let request = TileRequest::new("tile-1", "status");
    let render = render_tile(module, "wide.js", DEFAULT_ENTRY, &request).expect("上限不是错误");
    match render
    {
        TileRender::Rejected { reason, stats, fallback } =>
        {
            assert_eq!(reason, LimitReason::TooManyNodes { nodes: 5001, limit: 5000 });
            assert_eq!(stats.nodes, 5001);
            assert_eq!(fallback.key.as_deref(), Some("fallback"));
            assert!(fallback.text.expect("兜底句").contains("超出渲染上限"));
        }
        other => panic!("应拒绝渲染：{other:?}"),
    }
}

/// 深度 33：拒绝渲染。
#[test]
fn rejects_tree_over_depth_limit()
{
    let module = r#"
export default function render() {
    let node = { type: "text", text: "leaf" };
    for (let index = 0; index < 32; index += 1) {
        node = { type: "column", children: [node] };
    }
    return node;
}
"#;
    let request = TileRequest::new("tile-1", "status");
    let render = render_tile(module, "deep.js", DEFAULT_ENTRY, &request).expect("上限不是错误");
    match render
    {
        TileRender::Rejected { reason, stats, .. } =>
        {
            assert_eq!(reason, LimitReason::TooDeep { depth: 33, limit: 32 });
            assert_eq!(stats.depth, 33);
        }
        other => panic!("应拒绝渲染：{other:?}"),
    }
}

/// 插件返回裸色值：明确报错，指出字段与取值。
#[test]
fn rejects_bare_color_from_plugin()
{
    let module = r##"
export default function render() {
    return { type: "column", background: "#101010", children: [] };
}
"##;
    let request = TileRequest::new("tile-1", "status");
    match render_tile(module, "color.js", DEFAULT_ENTRY, &request).expect_err("应报错")
    {
        RenderTreeError::BareColor { field, value, .. } =>
        {
            assert_eq!(field, "background");
            assert_eq!(value, "#101010");
        }
        other => panic!("错误类型不符：{other:?}"),
    }
}

/// 插件使用界面调色板令牌：同样报错（桌面层只允许 --fence-*）。
#[test]
fn rejects_palette_token_from_plugin()
{
    let module = r#"
export default function render() {
    return { type: "text", text: "a", tone: "--color-text" };
}
"#;
    let request = TileRequest::new("tile-1", "status");
    match render_tile(module, "token.js", DEFAULT_ENTRY, &request).expect_err("应报错")
    {
        RenderTreeError::UnknownToken { field, value, .. } =>
        {
            assert_eq!(field, "tone");
            assert_eq!(value, "--color-text");
        }
        other => panic!("错误类型不符：{other:?}"),
    }
}

/// 模块抛出异常：错误信息带出原始消息。
#[test]
fn reports_script_error_message()
{
    let module = r#"
export default function render() {
    throw new Error("boom-42");
}
"#;
    let request = TileRequest::new("tile-1", "status");
    match render_tile(module, "throw.js", DEFAULT_ENTRY, &request).expect_err("应报错")
    {
        RenderTreeError::Script { detail } => assert!(detail.contains("boom-42"), "{detail}"),
        other => panic!("错误类型不符：{other:?}"),
    }
}

/// 模块没有导出入口：报 MissingEntry。
#[test]
fn reports_missing_entry()
{
    let module = "export const nothing = 1;\n";
    let request = TileRequest::new("tile-1", "status");
    assert_eq!(render_tile(module, "empty.js", DEFAULT_ENTRY, &request).expect_err("应报错"), RenderTreeError::MissingEntry { entry: DEFAULT_ENTRY.to_string() });
}

/// 入口返回值里出现函数：按不支持的类型报错，而不是静默变成 null。
#[test]
fn rejects_unsupported_value_type()
{
    let module = r#"
export default function render() {
    return { type: "text", text: "a", callback: () => 1 };
}
"#;
    let request = TileRequest::new("tile-1", "status");
    match render_tile(module, "function.js", DEFAULT_ENTRY, &request).expect_err("应报错")
    {
        RenderTreeError::Script { detail } => assert!(detail.contains("不支持的类型"), "{detail}"),
        other => panic!("错误类型不符：{other:?}"),
    }
}
