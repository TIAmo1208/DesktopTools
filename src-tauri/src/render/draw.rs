//! 绘制指令模型：桌面宿主、菜单、浮岛、Toast 共用的最小契约。
//!
//! 这一层刻意窄：只描述「画什么」，不含主题令牌、不含业务语义、不做布局。
//! 坐标单位统一为 DIP（96 DPI 下的逻辑像素），由 [`crate::render::geometry::DpiScale`] 在绘制时换算。

use serde::{Deserialize, Serialize};

/// 轴对齐矩形（DIP）。
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
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

    /// 右下角坐标。
    pub fn right(&self) -> f32 { self.x + self.width }

    /// 底边坐标。
    pub fn bottom(&self) -> f32 { self.y + self.height }

    /// 是否包含给定点（左闭右开，与 Win32 `PtInRect` 一致）。
    pub fn contains(&self, x: f32, y: f32) -> bool { x >= self.x && x < self.right() && y >= self.y && y < self.bottom() }

    /// 收缩到非负尺寸。
    pub fn normalized(&self) -> Self { Self::new(self.x, self.y, self.width.max(0.0), self.height.max(0.0)) }
}

/// 颜色：`0xRRGGBBAA`（Alpha 在最低字节，便于直接对着设计稿写值）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Color(pub u32);

impl Color
{
    /// 由分量构造，各分量 0–255。
    pub const fn rgba(red: u8, green: u8, blue: u8, alpha: u8) -> Self { Self(((red as u32) << 24) | ((green as u32) << 16) | ((blue as u32) << 8) | alpha as u32) }

    /// 全透明。
    pub const TRANSPARENT: Self = Self(0);

    /// 拆成 `[r, g, b, a]`（各分量 0.0–1.0），供 Direct2D 使用。
    pub fn to_unit(self) -> [f32; 4]
    {
        let red = ((self.0 >> 24) & 0xFF) as f32 / 255.0;
        let green = ((self.0 >> 16) & 0xFF) as f32 / 255.0;
        let blue = ((self.0 >> 8) & 0xFF) as f32 / 255.0;
        let alpha = (self.0 & 0xFF) as f32 / 255.0;
        [red, green, blue, alpha]
    }

    /// 替换透明度（0.0–1.0），保留 RGB。
    pub fn with_alpha(self, alpha: f32) -> Self
    {
        let alpha = (alpha.clamp(0.0, 1.0) * 255.0).round() as u32;
        Self((self.0 & 0xFFFF_FF00) | alpha)
    }
}

/// 文本水平对齐。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TextAlign
{
    Start,
    Center,
    End,
}

/// 单条绘制指令。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum DrawCommand
{
    /// 圆角矩形填充；`radius` 为 0 时退化为直角矩形。
    FillRoundRect
    {
        rect: Rect, radius: f32, color: Color
    },
    /// 圆角矩形描边；描边以矩形边界为中线（与 Canvas `strokeRect` 一致）。
    StrokeRoundRect
    {
        rect: Rect, radius: f32, width: f32, color: Color
    },
    /// 单段文本按矩形裁剪，不补省略号；`max_lines` 非零时额外裁到前 N 行的实际布局高度。
    Text
    {
        rect: Rect, text: String, size: f32, weight: u16, color: Color, align: TextAlign, max_lines: u16
    },
    /// 单行内容按 DirectWrite 字符边界截断并显示系统省略号，不改变旧 Text 的多行契约。
    Caption
    {
        rect: Rect, text: String, size: f32, weight: u16, color: Color, align: TextAlign
    },
    /// 入栈轴对齐裁剪，必须与 [`DrawCommand::PopClip`] 配对。
    PushClip
    {
        rect: Rect
    },
    /// 出栈裁剪。
    PopClip,
    /// 入栈整体透明度（0.0–1.0），必须与 [`DrawCommand::PopOpacity`] 配对。
    PushOpacity
    {
        opacity: f32
    },
    /// 出栈透明度。
    PopOpacity,
    /// 位图按目标矩形绘制（预乘 Alpha）；比例由调用方用 [`fit_center`] 保证，painter 不拉伸。
    #[serde(skip)]
    Image
    {
        rect: Rect, image: ImageRef
    },
}

/// 一张已解码位图的共享引用：像素是预乘 BGRA，跨线程只读。
///
/// 判等按**同一份像素**（`Arc` 指针）比较，不逐像素比较：帧间重绘只关心「还是不是那张图」，
/// painter 也用同一个身份做位图缓存键。
#[derive(Clone)]
pub struct ImageRef(std::sync::Arc<crate::platform::windows::shell_image::ShellImage>);

impl ImageRef
{
    /// 包装一份 Shell 图像（像素只复制一次，之后共享同一个 `Arc`）。
    pub fn new(image: std::sync::Arc<crate::platform::windows::shell_image::ShellImage>) -> Self { Self(image) }

    /// 像素宽。
    pub fn width(&self) -> u32 { self.0.width.max(0) as u32 }

    /// 像素高。
    pub fn height(&self) -> u32 { self.0.height.max(0) as u32 }

    /// 预乘 BGRA 像素（行优先，行距 = 宽度 × 4）。
    pub fn pixels(&self) -> &[u8] { &self.0.bgra }

    /// 身份：同一份像素共享同一个值，判等与 painter 位图缓存都用它。
    pub fn identity(&self) -> usize { std::sync::Arc::as_ptr(&self.0) as usize }
}

impl PartialEq for ImageRef
{
    fn eq(&self, other: &Self) -> bool { self.identity() == other.identity() }
}

impl std::fmt::Debug for ImageRef
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "ImageRef({}x{})", self.width(), self.height()) }
}

/// 在 `cell` 内按源图宽高比等比居中完整显示（不拉伸、不裁切）。
///
/// 规范：「缩略图等比居中，不拉伸或裁切」。源图尺寸无效（0）时返回零矩形，由调用方跳过。
pub fn fit_center(image: (u32, u32), cell: Rect) -> Rect
{
    let (width, height) = (image.0 as f32, image.1 as f32);
    if width <= 0.0 || height <= 0.0 || cell.width <= 0.0 || cell.height <= 0.0
    {
        return Rect::new(cell.x, cell.y, 0.0, 0.0);
    }
    let scale = (cell.width / width).min(cell.height / height);
    let (w, h) = (width * scale, height * scale);
    Rect::new(cell.x + (cell.width - w) / 2.0, cell.y + (cell.height - h) / 2.0, w, h)
}

/// 一帧的绘制指令序列。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DrawList
{
    commands: Vec<DrawCommand>,
}

impl DrawList
{
    /// 空序列。
    pub fn new() -> Self { Self::default() }

    /// 追加一条指令。
    pub fn push(&mut self, command: DrawCommand) { self.commands.push(command); }

    /// 只读访问全部指令。
    pub fn commands(&self) -> &[DrawCommand] { &self.commands }

    /// 指令数量。
    pub fn len(&self) -> usize { self.commands.len() }

    /// 是否为空。
    pub fn is_empty(&self) -> bool { self.commands.is_empty() }

    /// 栈是否严格配对：用一个记录类型的 LIFO，交叉嵌套（例如 PushClip → PushOpacity → PopClip）
    /// 会被判为不成立。Direct2D 的裁剪与图层必须后进先出，跨类交叉在运行时是非法状态，
    /// 所以校验放在开始 GPU 绘制之前。
    pub fn is_balanced(&self) -> bool { validate_stacks(self.commands.iter()).is_ok() }

    /// 校验失败时的原因（用于错误信息与单测）。
    pub fn validate(&self) -> std::result::Result<(), StackError> { validate_stacks(self.commands.iter()) }

    /// 返回整体平移后的副本：宿主按 `BeginDraw` 的更新偏移把整帧挪回原点时用它。
    pub fn translated(&self, dx: f32, dy: f32) -> Self
    {
        if dx == 0.0 && dy == 0.0
        {
            return self.clone();
        }
        let shift = |rect: &Rect| Rect::new(rect.x + dx, rect.y + dy, rect.width, rect.height);
        let commands = self
            .commands
            .iter()
            .map(|command| match command
            {
                DrawCommand::FillRoundRect { rect, radius, color } => DrawCommand::FillRoundRect { rect: shift(rect), radius: *radius, color: *color },
                DrawCommand::StrokeRoundRect { rect, radius, width, color } => DrawCommand::StrokeRoundRect { rect: shift(rect), radius: *radius, width: *width, color: *color },
                DrawCommand::Text { rect, text, size, weight, color, align, max_lines } => DrawCommand::Text { rect: shift(rect), text: text.clone(), size: *size, weight: *weight, color: *color, align: *align, max_lines: *max_lines },
                DrawCommand::Caption { rect, text, size, weight, color, align } => DrawCommand::Caption { rect: shift(rect), text: text.clone(), size: *size, weight: *weight, color: *color, align: *align },
                DrawCommand::PushClip { rect } => DrawCommand::PushClip { rect: shift(rect) },
                DrawCommand::PopClip => DrawCommand::PopClip,
                DrawCommand::PushOpacity { opacity } => DrawCommand::PushOpacity { opacity: *opacity },
                DrawCommand::PopOpacity => DrawCommand::PopOpacity,
                DrawCommand::Image { rect, image } => DrawCommand::Image { rect: shift(rect), image: image.clone() },
            })
            .collect();
        Self { commands }
    }
}

/// 栈里的一项：裁剪或透明度（顺序敏感，类型必须匹配）。
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
    /// 出栈项与栈顶类型不同（跨类交叉嵌套）。
    Crossed(StackKind, StackKind),
    /// 序列结束时仍有未出栈的项。
    LeftOpen(StackKind),
}

/// 用单一类型化 LIFO 校验指令序列的栈配对。
pub fn validate_stacks<'a>(commands: impl IntoIterator<Item = &'a DrawCommand>) -> std::result::Result<(), StackError>
{
    let mut stack: Vec<StackKind> = Vec::new(); // 单一栈，记录类型与顺序。
    for command in commands
    {
        match command
        {
            DrawCommand::PushClip { .. } => stack.push(StackKind::Clip),
            DrawCommand::PushOpacity { .. } => stack.push(StackKind::Opacity),
            DrawCommand::PopClip | DrawCommand::PopOpacity =>
            {
                let want = if matches!(command, DrawCommand::PopClip) { StackKind::Clip } else { StackKind::Opacity };
                match stack.pop()
                {
                    None => return Err(StackError::UnmatchedPop(want)),
                    Some(top) if top != want => return Err(StackError::Crossed(top, want)),
                    Some(_) =>
                    {}
                }
            }
            _ =>
            {}
        }
    }
    match stack.pop()
    {
        Some(kind) => Err(StackError::LeftOpen(kind)),
        None => Ok(()),
    }
}

#[cfg(test)]
mod stack_tests
{
    //! `render::draw` 栈校验的单测：同序通过、跨类交叉拒绝、未配对拒绝。
    //!
    // 契约与用例放在同一个可审查单元里，不单独开测试文件。

    use super::{Color, DrawCommand, DrawList, Rect, StackError, StackKind};

    /// 造一条裁剪或透明度的最小指令。
    fn clip() -> DrawCommand { DrawCommand::PushClip { rect: Rect::new(0.0, 0.0, 10.0, 10.0) } }

    fn opacity(value: f32) -> DrawCommand { DrawCommand::PushOpacity { opacity: value } }

    /// 同类型嵌套与正确交错（先入后出）必须通过。
    #[test]
    fn well_ordered_stacks_pass()
    {
        let mut nested = DrawList::new();
        nested.push(clip());
        nested.push(clip());
        nested.push(DrawCommand::PopClip);
        nested.push(DrawCommand::PopClip);
        assert!(nested.is_balanced());

        let mut layered = DrawList::new();
        layered.push(opacity(0.5));
        layered.push(clip());
        layered.push(DrawCommand::PopClip);
        layered.push(DrawCommand::PopOpacity);
        assert!(layered.is_balanced());
        assert!(layered.validate().is_ok());
    }

    /// 跨类交叉嵌套必须被拒绝：PushClip → PushOpacity → PopClip → PopOpacity。
    #[test]
    fn crossed_stacks_are_rejected()
    {
        let mut crossed = DrawList::new();
        crossed.push(clip());
        crossed.push(opacity(0.8));
        crossed.push(DrawCommand::PopClip); // 栈顶是 Opacity，却想关 Clip
        crossed.push(DrawCommand::PopOpacity);
        // 两个计数器会双双归零，只有记录类型的 LIFO 能发现交叉。
        assert!(!crossed.is_balanced());
        assert_eq!(crossed.validate(), Err(StackError::Crossed(StackKind::Opacity, StackKind::Clip)));

        let mut crossed_other_way = DrawList::new();
        crossed_other_way.push(opacity(0.5));
        crossed_other_way.push(clip());
        crossed_other_way.push(DrawCommand::PopOpacity);
        assert_eq!(crossed_other_way.validate(), Err(StackError::Crossed(StackKind::Clip, StackKind::Opacity)));
    }

    /// 空栈出栈与未闭合都要报出来，且区分「谁没闭合」。
    #[test]
    fn unmatched_and_left_open_stacks_are_rejected()
    {
        let mut extra_pop = DrawList::new();
        extra_pop.push(DrawCommand::PopOpacity);
        assert_eq!(extra_pop.validate(), Err(StackError::UnmatchedPop(StackKind::Opacity)));

        let mut left_open = DrawList::new();
        left_open.push(clip());
        assert_eq!(left_open.validate(), Err(StackError::LeftOpen(StackKind::Clip)));

        let mut left_open_opacity = DrawList::new();
        left_open_opacity.push(opacity(1.0));
        assert_eq!(left_open_opacity.validate(), Err(StackError::LeftOpen(StackKind::Opacity)));
    }

    /// 平移只改坐标、不改尺寸与顺序，栈校验结果随之保持一致。
    #[test]
    fn translation_keeps_structure_and_geometry_size()
    {
        let mut list = DrawList::new();
        list.push(clip());
        list.push(opacity(0.5));
        list.push(DrawCommand::FillRoundRect { rect: Rect::new(4.0, 6.0, 20.0, 10.0), radius: 2.0, color: Color::rgba(1, 2, 3, 4) });
        list.push(DrawCommand::PopOpacity);
        list.push(DrawCommand::PopClip);

        let moved = list.translated(-12.0, -30.0);
        assert_eq!(moved.len(), list.len());
        assert!(moved.is_balanced());
        match &moved.commands()[2]
        {
            DrawCommand::FillRoundRect { rect, .. } =>
            {
                assert_eq!((rect.x, rect.y), (-8.0, -24.0));
                assert_eq!((rect.width, rect.height), (20.0, 10.0));
            }
            other => panic!("第三条应仍是填充指令，实际 {other:?}"),
        }
        // 零位移不复制结构（返回自身语义等价）。
        assert_eq!(list.translated(0.0, 0.0), list);
    }
}
