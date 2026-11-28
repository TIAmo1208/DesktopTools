//! 布局引擎：把节点树按容器可用尺寸算成带绝对坐标的绘制列表。
//!
//! 纯函数：输入树、容器尺寸与文本度量，输出 [`DrawList`]；不建窗口、不读偏好、不做命中。
//! 文本度量是估算级（字符数 × 系数），真实字形成形由 DirectWrite 在绘制阶段完成；
//! [`TextMetrics`] 是这一层唯一可替换的近似，后续可用真实测量结果校准。

use crate::render_tree::draw::{DrawList, Primitive, PrimitiveStroke, Rect};
use crate::render_tree::node::{Align, Dimension, FontWeight, Justify, Node, NodeKind, TextSize};

/// 估算级文本度量（DIP）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextMetrics
{
    /// 字号档位 caption。
    pub caption: f32,
    /// 字号档位 body。
    pub body: f32,
    /// 字号档位 title。
    pub title: f32,
    /// 行高倍率。
    pub line_height: f32,
    /// 比例字体的平均字宽倍率（相对字号）。
    pub char_width: f32,
    /// 等宽字体的字宽倍率。
    pub mono_width: f32,
    /// 徽标文字左右内距。
    pub badge_padding: f32,
}

impl Default for TextMetrics
{
    /// 与阶段 0 原型的估算参数一致，便于对照读数。
    fn default() -> Self { Self { caption: 11.0, body: 13.0, title: 18.0, line_height: 1.45, char_width: 0.56, mono_width: 0.62, badge_padding: 6.0 } }
}

impl TextMetrics
{
    /// 字号档位对应的像素值。
    pub fn size(&self, size: TextSize) -> f32
    {
        match size
        {
            TextSize::Caption => self.caption,
            TextSize::Body => self.body,
            TextSize::Title => self.title,
        }
    }

    /// 单行高度（像素）。
    pub fn line_px(&self, size: TextSize) -> f32 { self.size(size) * self.line_height }

    /// 估算文本尺寸，返回（宽，单行高）。
    pub fn measure(&self, text: &str, size: TextSize, weight: FontWeight, mono: bool) -> (f32, f32)
    {
        let px = self.size(size);
        let factor = if mono { self.mono_width } else { self.char_width }; // 等宽与比例字体取不同字宽倍率。
        let weight_factor = match weight
        {
            FontWeight::Normal => 1.0,
            FontWeight::Medium => 1.02,
            FontWeight::Semibold => 1.04,
            FontWeight::Bold => 1.07,
        };
        (text.chars().count() as f32 * px * factor * weight_factor, self.line_px(size))
    }
}

/// 按容器尺寸布局整棵树。
pub fn layout(tree: &Node, width: f32, height: f32) -> DrawList
{
    let bounds = Rect::new(0.0, 0.0, width.max(0.0), height.max(0.0)); // 根容器固定为可用尺寸。
    layout_in(tree, bounds, TextMetrics::default())
}

/// 按给定矩形与文本度量布局整棵树。
pub fn layout_in(tree: &Node, bounds: Rect, metrics: TextMetrics) -> DrawList
{
    let mut engine = LayoutEngine { metrics, primitives: Vec::new(), paths: Vec::new() }; // 单次布局的累计状态。
    engine.place(tree, bounds, "$");
    DrawList::from_entries(engine.primitives, engine.paths)
}

struct LayoutEngine
{
    metrics: TextMetrics,
    primitives: Vec<Primitive>,
    /// 与 `primitives` 一一对应的节点路径（根为 `$`），供适配层报错时定位到节点。
    paths: Vec<String>,
}

/// 单个子节点在父容器里的尺寸与弹性权重。
struct Slot
{
    width: f32,
    height: f32,
    flex: Option<f32>,
}

impl LayoutEngine
{
    /// 节点本身是否参与布局（`visible: false` 的子节点不占位）。
    fn visible(node: &Node) -> bool { node.style.visible.unwrap_or(true) }

    /// 节点在给定宽度下的固有高度；固定高度直接返回。
    fn measure_height(&self, node: &Node, width: f32) -> f32
    {
        if !Self::visible(node)
        {
            return 0.0;
        }
        if let Some(Dimension::Fixed(height)) = node.style.height
        {
            return height.max(0.0);
        }
        let padding = node.style.padding.unwrap_or(0.0);
        let inner_width = (width - 2.0 * padding).max(0.0);
        match node.kind
        {
            NodeKind::Column | NodeKind::Scroll =>
            {
                let gap = node.style.gap.unwrap_or(0.0);
                let children: Vec<&Node> = node.children.iter().filter(|child| Self::visible(child)).collect(); // 只统计可见子节点。
                if children.is_empty()
                {
                    return 2.0 * padding;
                }
                let widths = self.child_widths(&children, inner_width, node.style.align.unwrap_or_default());
                let mut content = 0.0; // 子节点高度与间距之和。
                content += gap * (children.len() - 1) as f32;
                for (index, child) in children.iter().enumerate()
                {
                    content += match child.style.height
                    {
                        Some(Dimension::Fixed(height)) => height.max(0.0),
                        Some(Dimension::Flex(_)) => self.measure_height(child, widths[index]),
                        None if child.kind == NodeKind::Spacer => 0.0,
                        None => self.measure_height(child, widths[index]),
                    };
                }
                content + 2.0 * padding
            }
            NodeKind::Row =>
            {
                let children: Vec<&Node> = node.children.iter().filter(|child| Self::visible(child)).collect();
                let widths = self.child_widths(&children, inner_width, Align::Stretch);
                let mut tallest: f32 = 0.0; // 行内最高子节点决定行高。
                for (index, child) in children.iter().enumerate()
                {
                    tallest = tallest.max(self.measure_height(child, widths[index]));
                }
                tallest + 2.0 * padding
            }
            NodeKind::Grid =>
            {
                let columns = node.style.columns.unwrap_or(2).max(1) as usize;
                let gap = node.style.gap.unwrap_or(0.0);
                let cell_width = ((inner_width - gap * (columns - 1) as f32) / columns as f32).max(0.0);
                let mut total = 0.0; // 各行高度之和。
                let mut row_height: f32 = 0.0;
                let mut index = 0;
                for child in &node.children
                {
                    row_height = row_height.max(self.measure_height(child, cell_width));
                    index += 1;
                    if index % columns == 0
                    {
                        total += row_height + gap;
                        row_height = 0.0;
                    }
                }
                if index % columns != 0
                {
                    total += row_height + gap;
                }
                (total - gap).max(0.0) + 2.0 * padding
            }
            NodeKind::Text | NodeKind::Link => self.metrics.line_px(node.style.size.unwrap_or_default()) * node.style.truncate_lines.unwrap_or(1).max(1) as f32,
            NodeKind::Badge =>
            {
                let size = node.style.size.unwrap_or_default();
                self.metrics.line_px(size) + self.metrics.badge_padding
            }
            NodeKind::Progress => node.style.height.and_then(fixed).unwrap_or(6.0),
            NodeKind::Ring => node.style.height.and_then(fixed).unwrap_or(44.0),
            NodeKind::StatusDot => node.style.height.and_then(fixed).unwrap_or(8.0),
            NodeKind::Sparkline => node.style.height.and_then(fixed).unwrap_or(28.0),
            NodeKind::Bars => node.style.height.and_then(fixed).unwrap_or(40.0),
            NodeKind::Divider => node.style.height.and_then(fixed).unwrap_or(1.0),
            NodeKind::Icon | NodeKind::Image => node.style.height.and_then(fixed).unwrap_or(16.0),
            NodeKind::Empty => node.style.height.and_then(fixed).unwrap_or(24.0),
            NodeKind::Spacer => 0.0,
        }
    }

    /// 节点在不受父容器拉伸时的固有宽度。
    fn intrinsic_width(&self, node: &Node) -> f32
    {
        let width = match node.style.width
        {
            Some(Dimension::Fixed(width)) => width.max(0.0),
            _ => match node.kind
            {
                NodeKind::Text | NodeKind::Link => self.metrics.measure(node.text.as_deref().unwrap_or_default(), node.style.size.unwrap_or_default(), node.style.weight.unwrap_or_default(), node.style.mono.unwrap_or(false)).0,
                NodeKind::Badge => self.metrics.measure(node.text.as_deref().unwrap_or_default(), node.style.size.unwrap_or_default(), node.style.weight.unwrap_or_default(), false).0 + 2.0 * self.metrics.badge_padding,
                NodeKind::Ring | NodeKind::StatusDot | NodeKind::Icon | NodeKind::Image => node.style.height.and_then(fixed).unwrap_or(match node.kind
                {
                    NodeKind::Ring => 44.0,
                    NodeKind::StatusDot => 8.0,
                    _ => 16.0,
                }),
                NodeKind::Divider => 1.0,
                _ => 0.0,
            },
        };
        node.style.min_width.map(|min| width.max(min)).unwrap_or(width)
    }

    /// 按父容器可用宽度分配每个子节点的宽度。
    fn child_widths(&self, children: &[&Node], available: f32, align: Align) -> Vec<f32>
    {
        let mut widths: Vec<Option<f32>> = vec![None; children.len()]; // 尚未确定的宽度。
        let mut flex_total: f32 = 0.0; // 弹性权重之和。
        let mut fixed_total: f32 = 0.0; // 固定宽度之和。
        for (index, child) in children.iter().enumerate()
        {
            match child.style.width
            {
                Some(Dimension::Fixed(width)) =>
                {
                    widths[index] = Some(width.max(0.0));
                    fixed_total += width.max(0.0);
                }
                Some(Dimension::Flex(weight)) => flex_total += weight.flex,
                None =>
                {}
            }
        }
        let free = (available - fixed_total).max(0.0); // 固定宽度之外的空间。
        for (index, child) in children.iter().enumerate()
        {
            if widths[index].is_some()
            {
                continue;
            }
            widths[index] = Some(match child.style.width
            {
                Some(Dimension::Flex(weight)) if flex_total > 0.0 => free * weight.flex / flex_total,
                _ if flex_total > 0.0 => self.intrinsic_width(child).min(free),
                _ if align == Align::Stretch => available,
                _ => self.intrinsic_width(child).min(available),
            });
        }
        widths.into_iter().map(|width| width.unwrap_or(0.0)).collect()
    }

    /// 记录一条图元与它来自哪个节点路径。
    fn push(&mut self, primitive: Primitive, path: &str)
    {
        self.primitives.push(primitive);
        self.paths.push(path.to_string());
    }

    /// 放置一个节点及其子树。
    fn place(&mut self, node: &Node, rect: Rect, path: &str)
    {
        if !Self::visible(node)
        {
            return;
        }
        let opacity = node.style.opacity.unwrap_or(1.0); // 1 以外需要包一层透明度栈。
        if opacity < 1.0
        {
            self.push(Primitive::PushOpacity { opacity }, path);
        }

        // 盒子类节点先画自身表面；进度、环、条带、状态点与分隔线把 background 当作轨道/线条色，
        // 由各自图元消费，不在这里再铺一层底色。
        if draws_surface(node.kind) && (node.style.background.is_some() || node.style.border.is_some())
        {
            self.push(Primitive::RoundRect { rect, radius: node.style.radius.unwrap_or(0.0), fill: node.style.background.clone(), stroke: node.style.border.clone().map(|color| PrimitiveStroke { color, width: node.style.border_width.unwrap_or(1.0) }) }, path);
        }

        let padding = node.style.padding.unwrap_or(0.0);
        let inner = rect.inset(padding);
        match node.kind
        {
            NodeKind::Column => self.place_column(node, inner, path),
            NodeKind::Scroll =>
            {
                self.push(Primitive::PushClip { rect }, path);
                self.place_column(node, inner, path);
                self.push(Primitive::PopClip, path);
            }
            NodeKind::Row => self.place_row(node, inner, path),
            NodeKind::Grid => self.place_grid(node, inner, path),
            NodeKind::Text | NodeKind::Link => self.place_text(node, inner, path),
            NodeKind::Badge => self.place_badge(node, inner, path),
            NodeKind::Progress =>
            {
                let value = node.value.unwrap_or(0.0);
                self.push(Primitive::Progress { rect, radius: node.style.radius.unwrap_or(rect.height * 0.5), value, track: node.style.background.clone(), fill: node.tone.clone() }, path);
            }
            NodeKind::Ring =>
            {
                let side = rect.width.min(rect.height);
                let thickness = node.style.border_width.unwrap_or((side * 0.12).max(2.0));
                let value = node.value.unwrap_or(0.0);
                self.push(Primitive::Ring { rect, thickness, value, track: node.style.background.clone(), fill: node.tone.clone() }, path);
            }
            NodeKind::StatusDot => self.push(Primitive::StatusDot { rect, color: node.tone.clone() }, path),
            NodeKind::Divider => self.push(Primitive::Divider { rect, color: node.tone.clone().or_else(|| node.style.background.clone()) }, path),
            NodeKind::Sparkline =>
            {
                let width = node.style.border_width.unwrap_or(1.5);
                self.push(Primitive::Sparkline { rect, values: node.values.clone(), color: node.tone.clone(), width }, path);
            }
            NodeKind::Bars =>
            {
                let radius = node.style.radius.unwrap_or(2.0);
                let gap = node.style.gap.unwrap_or(2.0);
                self.push(Primitive::Bars { rect, values: node.values.clone(), color: node.tone.clone(), radius, gap }, path);
            }
            NodeKind::Icon | NodeKind::Image | NodeKind::Empty => self.push(Primitive::Placeholder { rect, radius: node.style.radius.unwrap_or(0.0), kind: node.kind }, path),
            NodeKind::Spacer =>
            {}
        }

        if opacity < 1.0
        {
            self.push(Primitive::PopOpacity, path);
        }
    }

    /// 文本节点：矩形与对齐由父容器给出，截断行数写进图元。
    fn place_text(&mut self, node: &Node, rect: Rect, path: &str)
    {
        let size = node.style.size.unwrap_or_default();
        let weight = node.style.weight.unwrap_or_default();
        let mono = node.style.mono.unwrap_or(false);
        let align = node.style.text_align.unwrap_or(Align::Start);
        let max_lines = node.style.truncate_lines.unwrap_or(1).max(1);
        let text = node.text.clone().unwrap_or_default(); // 必填字段已在解析期校验。
        self.push(Primitive::Text { rect, text, size, weight, mono, color: node.tone.clone(), align, max_lines, underline: node.kind == NodeKind::Link }, path);
    }

    /// 徽标：在给定矩形内居中排一行文字（底盒由 `background` / `radius` 表达）。
    fn place_badge(&mut self, node: &Node, rect: Rect, path: &str)
    {
        let text_rect = Rect::new(rect.x + self.metrics.badge_padding, rect.y, (rect.width - 2.0 * self.metrics.badge_padding).max(0.0), rect.height);
        self.push(Primitive::Text { rect: text_rect, text: node.text.clone().unwrap_or_default(), size: node.style.size.unwrap_or_default(), weight: node.style.weight.unwrap_or_default(), mono: node.style.mono.unwrap_or(false), color: node.tone.clone(), align: Align::Center, max_lines: 1, underline: false }, path);
    }

    /// 纵向容器：固定高度、弹性高度与固有高度混排，支持 gap、padding、align 与 justify。
    fn place_column(&mut self, node: &Node, inner: Rect, path: &str)
    {
        let gap = node.style.gap.unwrap_or(0.0);
        let align = node.style.align.unwrap_or_default();
        let justify = node.style.justify.unwrap_or_default();
        // 保留原始子节点下标，路径才能准确指到节点。
        let children: Vec<(usize, &Node)> = node.children.iter().enumerate().filter(|(_, child)| Self::visible(child)).collect();
        if children.is_empty()
        {
            return;
        }
        let plain: Vec<&Node> = children.iter().map(|(_, child)| *child).collect();
        let widths = self.child_widths(&plain, inner.width, align);
        let mut slots: Vec<Slot> = Vec::with_capacity(children.len()); // 每个子节点的高度与权重。
        let mut flex_total = 0.0; // 弹性高度权重之和。
        for (index, (_, child)) in children.iter().enumerate()
        {
            let slot = match child.style.height
            {
                Some(Dimension::Fixed(height)) => Slot { width: widths[index], height: height.max(0.0), flex: None },
                Some(Dimension::Flex(weight)) =>
                {
                    flex_total += weight.flex;
                    Slot { width: widths[index], height: 0.0, flex: Some(weight.flex) }
                }
                None if child.kind == NodeKind::Spacer =>
                {
                    flex_total += 1.0;
                    Slot { width: widths[index], height: 0.0, flex: Some(1.0) }
                }
                None => Slot { width: widths[index], height: self.measure_height(child, widths[index]), flex: None },
            };
            slots.push(slot);
        }
        let gaps_total = gap * (children.len() - 1) as f32;
        let used: f32 = slots.iter().map(|slot| slot.height).sum::<f32>() + gaps_total;
        let leftover = (inner.height - used).max(0.0); // 供弹性子节点与 justify 分配的空间。
        if flex_total > 0.0
        {
            for slot in &mut slots
            {
                if let Some(weight) = slot.flex
                {
                    slot.height = leftover * weight / flex_total;
                }
            }
        }
        let assigned: f32 = slots.iter().map(|slot| slot.height).sum::<f32>() + gaps_total;
        let extra = (inner.height - assigned).max(0.0);
        let (mut cursor, extra_gap) = match justify
        {
            Justify::Start => (inner.y, 0.0),
            Justify::Center => (inner.y + extra * 0.5, 0.0),
            Justify::End => (inner.y + extra, 0.0),
            Justify::SpaceBetween if children.len() > 1 => (inner.y, extra / (children.len() - 1) as f32),
            Justify::SpaceBetween => (inner.y, 0.0),
        };
        for (index, (child_index, child)) in children.iter().enumerate()
        {
            let width = slots[index].width;
            let offset = match align
            {
                Align::Center => (inner.width - width) * 0.5,
                Align::End => inner.width - width,
                _ => 0.0,
            };
            let child_path = format!("{path}.children[{child_index}]");
            self.place(child, Rect::new(inner.x + offset, cursor, width, slots[index].height), &child_path);
            cursor += slots[index].height + gap + extra_gap;
        }
    }

    /// 横向容器：固定宽度、弹性宽度与固有宽度混排，支持 gap、padding、align 与 justify。
    fn place_row(&mut self, node: &Node, inner: Rect, path: &str)
    {
        let gap = node.style.gap.unwrap_or(0.0);
        let align = node.style.align.unwrap_or_default();
        let justify = node.style.justify.unwrap_or_default();
        let children: Vec<(usize, &Node)> = node.children.iter().enumerate().filter(|(_, child)| Self::visible(child)).collect();
        if children.is_empty()
        {
            return;
        }
        let plain: Vec<&Node> = children.iter().map(|(_, child)| *child).collect();
        let widths = self.child_widths(&plain, inner.width, Align::Stretch);
        let heights: Vec<f32> = children
            .iter()
            .enumerate()
            .map(|(index, (_, child))| match child.style.height
            {
                Some(Dimension::Fixed(height)) => height.max(0.0),
                _ if align == Align::Stretch => inner.height,
                _ => self.measure_height(child, widths[index]),
            })
            .collect();
        let gaps_total = gap * (children.len() - 1) as f32;
        let used: f32 = widths.iter().sum::<f32>() + gaps_total;
        let extra = (inner.width - used).max(0.0); // 供 justify 分配的空间。
        let (mut cursor, extra_gap) = match justify
        {
            Justify::Start => (inner.x, 0.0),
            Justify::Center => (inner.x + extra * 0.5, 0.0),
            Justify::End => (inner.x + extra, 0.0),
            Justify::SpaceBetween if children.len() > 1 => (inner.x, extra / (children.len() - 1) as f32),
            Justify::SpaceBetween => (inner.x, 0.0),
        };
        for (index, (child_index, child)) in children.iter().enumerate()
        {
            let height = heights[index];
            let offset = match align
            {
                Align::Center => (inner.height - height) * 0.5,
                Align::End => inner.height - height,
                _ => 0.0,
            };
            let child_path = format!("{path}.children[{child_index}]");
            self.place(child, Rect::new(cursor, inner.y + offset, widths[index], height), &child_path);
            cursor += widths[index] + gap + extra_gap;
        }
    }

    /// 网格容器：按 `columns` 列排列，行高取该行最高子节点。
    fn place_grid(&mut self, node: &Node, inner: Rect, path: &str)
    {
        let columns = node.style.columns.unwrap_or(2).max(1) as usize;
        let gap = node.style.gap.unwrap_or(0.0);
        let cell_width = ((inner.width - gap * (columns - 1) as f32) / columns as f32).max(0.0);
        let children: Vec<(usize, &Node)> = node.children.iter().enumerate().filter(|(_, child)| Self::visible(child)).collect();
        let mut cursor = inner.y; // 当前行的上边界。
        let mut start = 0;
        while start < children.len()
        {
            let row = &children[start..children.len().min(start + columns)];
            let row_height = row.iter().map(|(_, child)| self.measure_height(child, cell_width)).fold(0.0_f32, f32::max);
            for (column, (child_index, child)) in row.iter().enumerate()
            {
                let rect = Rect::new(inner.x + column as f32 * (cell_width + gap), cursor, cell_width, row_height);
                let child_path = format!("{path}.children[{child_index}]");
                self.place(child, rect, &child_path);
            }
            cursor += row_height + gap;
            start += columns;
        }
    }
}

/// 是否绘制「节点自身表面」（背景/描边）；其余节点把颜色令牌交给各自的图元。
fn draws_surface(kind: NodeKind) -> bool { matches!(kind, NodeKind::Column | NodeKind::Row | NodeKind::Grid | NodeKind::Scroll | NodeKind::Text | NodeKind::Link | NodeKind::Badge | NodeKind::Icon | NodeKind::Image | NodeKind::Empty) }

/// 取固定尺寸值。
fn fixed(dimension: Dimension) -> Option<f32>
{
    match dimension
    {
        Dimension::Fixed(value) => Some(value),
        Dimension::Flex(_) => None,
    }
}
