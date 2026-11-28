//! 布局输出：带绝对坐标的绘制图元与一帧绘制列表。
//!
//! 图元只携带主题令牌名，颜色解析留给绘制层；本模块不引用 `render::` 的窗口、Direct2D
//! 或绘制指令代码，两者通过「令牌名 + 坐标 + 文本」这一层数据约定衔接。

use serde::{Deserialize, Serialize};

use crate::render_tree::node::{Align, FontWeight, NodeKind, TextSize};
use crate::render_tree::tokens::ThemeToken;

/// 轴对齐矩形（DIP，原点在渲染树根容器的左上角）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Rect
{
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect
{
    /// 按左上角与尺寸构造。
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self { Self { x, y, width, height } }

    /// 右边界坐标。
    pub fn right(&self) -> f32 { self.x + self.width }

    /// 下边界坐标。
    pub fn bottom(&self) -> f32 { self.y + self.height }

    /// 向内收缩给定内距，尺寸不为负。
    pub fn inset(&self, padding: f32) -> Self { Self::new(self.x + padding, self.y + padding, (self.width - 2.0 * padding).max(0.0), (self.height - 2.0 * padding).max(0.0)) }

    /// 替换高度。
    pub fn with_height(&self, height: f32) -> Self { Self::new(self.x, self.y, self.width, height.max(0.0)) }

    /// 替换宽度。
    pub fn with_width(&self, width: f32) -> Self { Self::new(self.x, self.y, width.max(0.0), self.height) }

    /// 平移到新位置（尺寸不变）。
    pub fn moved_to(&self, x: f32, y: f32) -> Self { Self::new(x, y, self.width, self.height) }
}

/// 描边参数。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PrimitiveStroke
{
    pub color: ThemeToken,
    pub width: f32,
}

/// 一个绘制图元。
///
/// `fill` / `color` / `track` 为 `None` 表示交给绘制层取该图元的主题默认色，
/// 这是契约里写明的缺省，不是对非法输入的回退。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "primitive", rename_all = "snake_case")]
pub enum Primitive
{
    /// 圆角矩形（背景、描边、徽标底、图标占位共用）；`radius` 为 0 时是直角矩形。
    RoundRect
    {
        rect: Rect, radius: f32, fill: Option<ThemeToken>, stroke: Option<PrimitiveStroke>
    },
    /// 单段或多行文本；`max_lines` 为截断行数（1 表示单行）。
    Text
    {
        rect: Rect, text: String, size: TextSize, weight: FontWeight, mono: bool, color: Option<ThemeToken>, align: Align, max_lines: u16, underline: bool
    },
    /// 线性进度条。
    Progress
    {
        rect: Rect, radius: f32, value: f32, track: Option<ThemeToken>, fill: Option<ThemeToken>
    },
    /// 环形进度（`thickness` 为环宽）。
    Ring
    {
        rect: Rect, thickness: f32, value: f32, track: Option<ThemeToken>, fill: Option<ThemeToken>
    },
    /// 状态圆点。
    StatusDot
    {
        rect: Rect, color: Option<ThemeToken>
    },
    /// 分隔线（高度一般为 1 DIP）。
    Divider
    {
        rect: Rect, color: Option<ThemeToken>
    },
    /// 迷你折线图；`values` 为 0–1 的归一化序列。
    Sparkline
    {
        rect: Rect, values: Vec<f32>, color: Option<ThemeToken>, width: f32
    },
    /// 条带柱图；`values` 为 0–1 的归一化序列。
    Bars
    {
        rect: Rect, values: Vec<f32>, color: Option<ThemeToken>, radius: f32, gap: f32
    },
    /// 占位图元：图标、图片与空态共用，绘制层按 `kind` 选择图形或图标。
    Placeholder
    {
        rect: Rect, radius: f32, kind: NodeKind
    },
    /// 入栈裁剪，必须与 [`Primitive::PopClip`] 配对。
    PushClip
    {
        rect: Rect
    },
    /// 出栈裁剪。
    PopClip,
    /// 入栈整体透明度（0.0–1.0），必须与 [`Primitive::PopOpacity`] 配对。
    PushOpacity
    {
        opacity: f32
    },
    /// 出栈透明度。
    PopOpacity,
}

/// 栈里的一项：裁剪或透明度。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StackKind
{
    Clip,
    Opacity,
}

/// 栈校验失败的原因。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StackError
{
    /// 出栈时栈是空的。
    UnmatchedPop(StackKind),
    /// 出栈项与栈顶类型不同。
    Crossed(StackKind, StackKind),
    /// 序列结束时仍有未出栈的项。
    LeftOpen(StackKind),
}

/// 一帧的绘制图元序列。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DrawList
{
    primitives: Vec<Primitive>,
    /// 与 `primitives` 一一对应的节点路径；手工构造的空列表允许为空。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    paths: Vec<String>,
}

impl DrawList
{
    /// 空列表。
    pub fn new() -> Self { Self::default() }

    /// 按已有图元序列构造（无路径信息）。
    pub fn from_primitives(primitives: Vec<Primitive>) -> Self
    {
        let paths = vec![String::new(); primitives.len()]; // 路径缺省为空串，适配层报错时退化为根路径。
        Self { primitives, paths }
    }

    /// 按「图元 + 节点路径」序列构造（布局引擎的出口）。
    pub(crate) fn from_entries(primitives: Vec<Primitive>, paths: Vec<String>) -> Self { Self { primitives, paths } }

    /// 第 `index` 条图元来自哪个节点（根为 `$`）；越界或未记录时返回 `"$"`。
    pub fn path_of(&self, index: usize) -> &str { self.paths.get(index).map(String::as_str).filter(|path| !path.is_empty()).unwrap_or("$") }

    /// 追加一个图元（无路径信息）。
    pub fn push(&mut self, primitive: Primitive)
    {
        self.primitives.push(primitive);
        self.paths.push(String::new());
    }

    /// 只读访问全部图元。
    pub fn primitives(&self) -> &[Primitive] { &self.primitives }

    /// 图元数量。
    pub fn len(&self) -> usize { self.primitives.len() }

    /// 是否为空。
    pub fn is_empty(&self) -> bool { self.primitives.is_empty() }

    /// 校验裁剪与透明度栈严格配对（Direct2D 的裁剪与图层必须后进先出）。
    pub fn validate(&self) -> Result<(), StackError>
    {
        let mut stack: Vec<StackKind> = Vec::new(); // 单一栈，记录类型与顺序。
        for primitive in &self.primitives
        {
            match primitive
            {
                Primitive::PushClip { .. } => stack.push(StackKind::Clip),
                Primitive::PushOpacity { .. } => stack.push(StackKind::Opacity),
                Primitive::PopClip => match stack.pop()
                {
                    Some(StackKind::Clip) =>
                    {}
                    Some(other) => return Err(StackError::Crossed(StackKind::Clip, other)),
                    None => return Err(StackError::UnmatchedPop(StackKind::Clip)),
                },
                Primitive::PopOpacity => match stack.pop()
                {
                    Some(StackKind::Opacity) =>
                    {}
                    Some(other) => return Err(StackError::Crossed(StackKind::Opacity, other)),
                    None => return Err(StackError::UnmatchedPop(StackKind::Opacity)),
                },
                _ =>
                {}
            }
        }
        match stack.pop()
        {
            None => Ok(()),
            Some(kind) => Err(StackError::LeftOpen(kind)),
        }
    }

    /// 栈是否配对。
    pub fn is_balanced(&self) -> bool { self.validate().is_ok() }
}
