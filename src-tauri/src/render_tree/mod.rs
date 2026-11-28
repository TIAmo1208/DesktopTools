//! 插件渲染树宿主基础：节点模型、主题令牌校验、布局到绘制列表、QuickJS 执行。
//!
//! 模块边界：**纯逻辑**——不建窗口、不碰 Direct2D、不读写偏好、不改默认启动路径。
//! 输入是插件模块源码与容器尺寸，输出是带绝对坐标的 [`DrawList`]；与 [`crate::render`] 的
//! 窗口/绘制代码没有依赖关系：`adapter` 把 [`Primitive`] 翻成 `render::draw::DrawCommand`
//! （只读引用其类型），颜色经 [`TokenPalette`] 由令牌名解析成具体值。
//!
//! 对应 ADR-0053「待验证假设」的落地情况：
//! - 已落地：`key` 稳定槽位、`visible` 隐藏语义、节点数与深度上限（含兜底句）、令牌白名单。
//! - 未落地（阶段 3 后续）：交互命中、文本测量回读、节点动画、剪贴板、宿主定时器/rAF/可见性上报。

pub mod adapter;
pub mod draw;
pub mod error;
pub mod host;
pub mod layout;
pub mod loader;
pub mod manifest;
pub mod node;
pub mod palette;
pub mod slot;
pub mod tokens;

pub use adapter::{Adapted, adapt_tree, adapt_value, convert};
pub use draw::{DrawList, Primitive, PrimitiveStroke, Rect, StackError, StackKind};
pub use error::RenderTreeError;
pub use host::{DEFAULT_ENTRY, TileRender, TileRequest, render_tile};
pub use layout::{TextMetrics, layout, layout_in};
pub use loader::PluginSession;
pub use manifest::{Entries, ManifestError, ManifestMode, ManifestWarning, PluginManifest};
pub use node::{Align, Dimension, FlexWeight, FontWeight, Justify, LimitReason, MAX_DEPTH, MAX_NODES, Node, NodeKind, ParsedTree, Style, TextSize, TreeStats, fallback_node, parse_tree};
pub use palette::TokenPalette;
pub use slot::DisplaySlot;
pub use tokens::{FENCE_TOKENS, ThemeToken, TokenError};

#[cfg(test)]
#[path = "node.test.rs"]
mod node_tests;

#[cfg(test)]
#[path = "layout.test.rs"]
mod layout_tests;

#[cfg(test)]
#[path = "tokens.test.rs"]
mod tokens_tests;

#[cfg(test)]
#[path = "host.test.rs"]
mod host_tests;

#[cfg(test)]
#[path = "manifest.test.rs"]
mod manifest_tests;

#[cfg(test)]
#[path = "loader.test.rs"]
mod loader_tests;

#[cfg(test)]
#[path = "adapter.test.rs"]
mod adapter_tests;
