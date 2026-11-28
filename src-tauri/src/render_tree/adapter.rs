//! 渲染树布局结果 → 渲染层绘制命令。
//!
//! 这一层是插件内容接入原生绘制的最后一步：把 [`crate::render_tree::layout`] 产出的
//! 「绝对坐标 + 图元 + 主题令牌名」翻成 [`crate::render::draw::DrawCommand`]。只**只读引用**
//! `render::draw` 的类型，不修改 `render/` 下的任何文件，也不建窗口。
//!
//! 颜色只走令牌表（[`TokenPalette`]）：令牌查不到就报错并带节点路径，绝不猜默认色；
//! 裸色值在解析阶段已被拒绝（[`crate::render_tree::error::RenderTreeError::BareColor`]）。
//!
//! 近似说明（绘制指令没有弧线与折线图元）：
//! - 环（`Ring`）用 24 段等距小圆点近似，先铺轨道色再铺已完成部分；
//! - 迷你折线（`Sparkline`）用采样点的小圆点近似；
//! - 进度（`Progress`）用「轨道矩形 + 按比例截宽的填充矩形」表达。
//! 这些都在记录里标为近似，等绘制层支持弧线/折线后替换，不影响契约。

use crate::render::draw::{Color, DrawCommand, DrawList as CommandList, Rect as CommandRect, TextAlign as CommandTextAlign};
use crate::render_tree::draw::{DrawList, Primitive, Rect};
use crate::render_tree::error::RenderTreeError;
use crate::render_tree::layout::{TextMetrics, layout_in};
use crate::render_tree::node::{Align, LimitReason, MAX_DEPTH, MAX_NODES, Node, NodeKind, TextSize, TreeStats, fallback_node, parse_tree};
use crate::render_tree::palette::TokenPalette;
use crate::render_tree::tokens::{FENCE_TOKENS, ThemeToken};

/// 图元没有显式颜色时的缺省令牌（语义取主题变量契约里最近的一项）。
const DEFAULT_TEXT: &str = "--fence-text";
const DEFAULT_TRACK: &str = "--fence-muted";
const DEFAULT_FILL: &str = "--fence-active";
const DEFAULT_DOT: &str = "--fence-active";
const DEFAULT_DIVIDER: &str = "--fence-border";
const DEFAULT_SURFACE: &str = "--fence-surface";
const DEFAULT_BORDER: &str = "--fence-border";
/// 环形进度的近似分段数。
const RING_SEGMENTS: usize = 24;

/// 一次适配的结果。
#[derive(Clone, Debug, PartialEq)]
pub struct Adapted
{
    /// 渲染层绘制命令；超限时为空。
    pub commands: CommandList,
    /// 树的规模统计。
    pub stats: TreeStats,
    /// 超限原因；非空表示未产出任何命令。
    pub rejection: Option<LimitReason>,
    /// 超限时建议显示的兜底节点，调用方可直接渲染它。
    pub fallback: Option<Node>,
}

impl Adapted
{
    /// 是否因为超限而没有产出命令。
    pub fn is_rejected(&self) -> bool { self.rejection.is_some() }

    /// 命令条数。
    pub fn command_count(&self) -> usize { self.commands.len() }
}

/// 已校验的树 → 布局 → 绘制命令。
pub fn adapt_tree(tree: &Node, bounds: Rect, palette: &TokenPalette) -> Result<Adapted, RenderTreeError>
{
    let stats = tree.measure_stats(); // 限额在适配层再判一次，避免调用方绕过解析直接构造大树。
    if let Some(reason) = limit_reason(&stats)
    {
        return Ok(rejected(reason, stats));
    }
    finish(tree, stats, bounds, palette)
}

/// 插件返回的 JSON 树 → 契约校验 → 限额 → 布局 → 绘制命令。
pub fn adapt_value(value: &serde_json::Value, bounds: Rect, palette: &TokenPalette) -> Result<Adapted, RenderTreeError>
{
    let parsed = parse_tree(value)?;
    if let Some(reason) = parsed.limit
    {
        return Ok(rejected(reason, parsed.stats));
    }
    let tree = parsed.tree.expect("未触发上限时解析结果必含节点树");
    finish(&tree, parsed.stats, bounds, palette)
}

/// 布局结果 → 绘制命令（不判限额，供已经拿到布局结果的调用方使用）。
pub fn convert(draw: &DrawList, palette: &TokenPalette) -> Result<CommandList, RenderTreeError>
{
    let mut commands = CommandList::new(); // 输出的渲染层指令。
    for (index, primitive) in draw.primitives().iter().enumerate()
    {
        let path = draw.path_of(index).to_string(); // 该图元来自哪个节点。
        emit(&mut commands, primitive, palette, &path)?;
    }
    Ok(commands)
}

/// 共用出口：布局 + 转换。
fn finish(tree: &Node, stats: TreeStats, bounds: Rect, palette: &TokenPalette) -> Result<Adapted, RenderTreeError>
{
    let draw = layout_in(tree, bounds, TextMetrics::default());
    let commands = convert(&draw, palette)?;
    Ok(Adapted { commands, stats, rejection: None, fallback: None })
}

/// 构造「超限、不产出命令」的结果，并记 Warn 日志。
fn rejected(reason: LimitReason, stats: TreeStats) -> Adapted
{
    crate::log_warn!("渲染树适配：{}（节点={} 深度={}），不产出绘制命令", reason.describe(), stats.nodes, stats.depth);
    Adapted { commands: CommandList::new(), stats, rejection: Some(reason), fallback: Some(fallback_node(reason)) }
}

/// 把树规模判成限额原因（与解析层同一判定顺序：先节点数、再深度）。
fn limit_reason(stats: &TreeStats) -> Option<LimitReason>
{
    if stats.nodes > MAX_NODES
    {
        return Some(LimitReason::TooManyNodes { nodes: stats.nodes, limit: MAX_NODES });
    }
    if stats.depth > MAX_DEPTH
    {
        return Some(LimitReason::TooDeep { depth: stats.depth, limit: MAX_DEPTH });
    }
    None
}

/// 单条图元的转换。
fn emit(commands: &mut CommandList, primitive: &Primitive, palette: &TokenPalette, path: &str) -> Result<(), RenderTreeError>
{
    match primitive
    {
        Primitive::RoundRect { rect, radius, fill, stroke } =>
        {
            let rect = to_command_rect(*rect);
            if let Some(fill) = fill
            {
                commands.push(DrawCommand::FillRoundRect { rect, radius: clamp_radius(*radius, rect), color: color_of(palette, Some(fill), DEFAULT_SURFACE, path)? });
            }
            if let Some(stroke) = stroke
            {
                commands.push(DrawCommand::StrokeRoundRect { rect, radius: clamp_radius(*radius, rect), width: stroke.width.max(0.0), color: color_of(palette, Some(&stroke.color), DEFAULT_BORDER, path)? });
            }
            Ok(())
        }
        Primitive::Text { rect, text, size, weight, mono, color, align, max_lines, underline } =>
        {
            let rect = to_command_rect(*rect);
            let color = color_of(palette, color.as_ref(), DEFAULT_TEXT, path)?;
            // 等宽数字由字体族表达，`DrawCommand::Text` 没有对应字段（记录里列为绘制层缺口）。
            let _ = mono;
            commands.push(DrawCommand::Text { rect, text: text.clone(), size: font_px(*size), weight: weight.numeric(), color, align: to_command_align(*align), max_lines: *max_lines });
            if *underline
            {
                // 链接下划线：底边 1 DIP 的细条。
                let line = CommandRect::new(rect.x, rect.y + rect.height - 1.0, rect.width, 1.0);
                commands.push(DrawCommand::FillRoundRect { rect: line, radius: 0.0, color });
            }
            Ok(())
        }
        Primitive::Progress { rect, radius, value, track, fill } =>
        {
            let rect = to_command_rect(*rect);
            let track_color = color_of(palette, track.as_ref(), DEFAULT_TRACK, path)?;
            commands.push(DrawCommand::FillRoundRect { rect, radius: clamp_radius(*radius, rect), color: track_color });
            let value = value.clamp(0.0, 1.0);
            if value > 0.0
            {
                let fill_rect = CommandRect::new(rect.x, rect.y, (rect.width * value).max(1.0), rect.height);
                commands.push(DrawCommand::FillRoundRect { rect: fill_rect, radius: clamp_radius(*radius, fill_rect), color: color_of(palette, fill.as_ref(), DEFAULT_FILL, path)? });
            }
            Ok(())
        }
        Primitive::Ring { rect, thickness, value, track, fill } =>
        {
            let rect = to_command_rect(*rect);
            let track_color = color_of(palette, track.as_ref(), DEFAULT_TRACK, path)?;
            let fill_color = color_of(palette, fill.as_ref(), DEFAULT_FILL, path)?;
            let value = value.clamp(0.0, 1.0);
            let filled = (value * RING_SEGMENTS as f32).round() as usize; // 已完成的分段数。
            let side = rect.width.min(rect.height);
            let radius = ((side - thickness) * 0.5).max(0.0);
            let center_x = rect.x + rect.width * 0.5;
            let center_y = rect.y + rect.height * 0.5;
            let dot = thickness.max(1.0);
            for segment in 0..RING_SEGMENTS
            {
                let angle = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * segment as f32 / RING_SEGMENTS as f32; // 从正上方顺时针。
                let x = center_x + radius * angle.cos() - dot * 0.5;
                let y = center_y + radius * angle.sin() - dot * 0.5;
                let color = if segment < filled { fill_color } else { track_color };
                commands.push(DrawCommand::FillRoundRect { rect: CommandRect::new(x, y, dot, dot), radius: dot * 0.5, color });
            }
            Ok(())
        }
        Primitive::StatusDot { rect, color } =>
        {
            let rect = to_command_rect(*rect);
            commands.push(DrawCommand::FillRoundRect { rect, radius: rect.width.min(rect.height) * 0.5, color: color_of(palette, color.as_ref(), DEFAULT_DOT, path)? });
            Ok(())
        }
        Primitive::Divider { rect, color } =>
        {
            commands.push(DrawCommand::FillRoundRect { rect: to_command_rect(*rect), radius: 0.0, color: color_of(palette, color.as_ref(), DEFAULT_DIVIDER, path)? });
            Ok(())
        }
        Primitive::Sparkline { rect, values, color, width } =>
        {
            let rect = to_command_rect(*rect);
            let color = color_of(palette, color.as_ref(), DEFAULT_FILL, path)?;
            let dot = ((*width).max(1.0) * 2.0).clamp(1.5, 6.0); // 采样点直径。
            let step = rect.width / values.len().max(1) as f32;
            for (index, value) in values.iter().enumerate()
            {
                let x = rect.x + step * (index as f32 + 0.5) - dot * 0.5;
                let y = rect.y + (1.0 - value.clamp(0.0, 1.0)) * rect.height - dot * 0.5;
                commands.push(DrawCommand::FillRoundRect { rect: CommandRect::new(x, y.clamp(rect.y, rect.y + rect.height), dot, dot), radius: dot * 0.5, color });
            }
            Ok(())
        }
        Primitive::Bars { rect, values, color, radius, gap } =>
        {
            let rect = to_command_rect(*rect);
            let color = color_of(palette, color.as_ref(), DEFAULT_FILL, path)?;
            let slot = rect.width / values.len().max(1) as f32;
            for (index, value) in values.iter().enumerate()
            {
                let bar_width = (slot - gap).max(0.5);
                let height = value.clamp(0.0, 1.0) * rect.height;
                if height <= 0.0
                {
                    continue;
                }
                let bar = CommandRect::new(rect.x + slot * index as f32 + gap * 0.5, rect.y + rect.height - height, bar_width, height);
                commands.push(DrawCommand::FillRoundRect { rect: bar, radius: clamp_radius(*radius, bar), color });
            }
            Ok(())
        }
        Primitive::Placeholder { rect, radius, kind } =>
        {
            if !matches!(kind, NodeKind::Icon | NodeKind::Image | NodeKind::Empty)
            {
                return Err(RenderTreeError::UnsupportedPrimitive { path: path.to_string(), detail: format!("占位图元只支持 icon / image / empty，收到 {}", kind.as_str()) });
            }
            let rect = to_command_rect(*rect);
            commands.push(DrawCommand::FillRoundRect { rect, radius: clamp_radius(*radius, rect), color: color_of(palette, None, DEFAULT_SURFACE, path)? });
            commands.push(DrawCommand::StrokeRoundRect { rect, radius: clamp_radius(*radius, rect), width: 1.0, color: color_of(palette, None, DEFAULT_BORDER, path)? });
            Ok(())
        }
        Primitive::PushClip { rect } =>
        {
            commands.push(DrawCommand::PushClip { rect: to_command_rect(*rect) });
            Ok(())
        }
        Primitive::PopClip =>
        {
            commands.push(DrawCommand::PopClip);
            Ok(())
        }
        Primitive::PushOpacity { opacity } =>
        {
            commands.push(DrawCommand::PushOpacity { opacity: opacity.clamp(0.0, 1.0) });
            Ok(())
        }
        Primitive::PopOpacity =>
        {
            commands.push(DrawCommand::PopOpacity);
            Ok(())
        }
    }
}

/// 取令牌颜色；显式令牌优先，缺省令牌兜底，表里没有就报错（带节点路径）。
fn color_of(palette: &TokenPalette, token: Option<&ThemeToken>, fallback: &str, path: &str) -> Result<Color, RenderTreeError>
{
    let name = token.map(ThemeToken::as_str).unwrap_or(fallback);
    palette.resolve_name(name).ok_or_else(|| RenderTreeError::PaletteMiss { path: path.to_string(), token: name.to_string() })
}

/// 内部矩形 → 渲染层矩形。
fn to_command_rect(rect: Rect) -> CommandRect { CommandRect::new(rect.x, rect.y, rect.width, rect.height) }

/// 圆角半径不能超过短边的一半，否则绘制层会得到非法圆角。
fn clamp_radius(radius: f32, rect: CommandRect) -> f32 { radius.max(0.0).min(rect.width.min(rect.height) * 0.5) }

/// 字号档位 → 像素值（与布局的估算度量一致）。
fn font_px(size: TextSize) -> f32 { TextMetrics::default().size(size) }

/// 渲染树的对齐 → 渲染层的文本对齐。
fn to_command_align(align: Align) -> CommandTextAlign
{
    match align
    {
        Align::Center => CommandTextAlign::Center,
        Align::End => CommandTextAlign::End,
        _ => CommandTextAlign::Start,
    }
}

/// 令牌白名单（供调用方核对颜色表的完整性）。
pub fn required_tokens() -> &'static [&'static str] { FENCE_TOKENS }
