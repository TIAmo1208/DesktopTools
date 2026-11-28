//! 文件夹门户外壳与条目网格（**只读**接入现有门户状态，占位方块）。
//!
//! 取数：目录名 / 面包屑层级 / 条目名称与类型来自现有 `FolderPortalSnapshot` / `FolderPortalItem`
//! （`src-tauri/src/commands/folder_portal.rs`），本模块不新开目录扫描、不改读写链路。
//!
//! 数值出处（`docs/design/文件夹门户.md`）：
//! - 公共标题轨道 `28px`；导航轨道最低 `28px`；面包屑基准字号 `12px`、节点间距 `4px`；
//! - 紧凑列表行高最低 `28px`、`16px` 系统图标、`13px` 名称、图标与名称间距 `8px`；
//! - 平铺：默认项目 `68 × 90px`、横向间距 `6px`、纵向间距 `8px`、系统图像 `48px`、名称最多两行；
//! - 圆角 `min(8px, --surface-radius)`，当前默认 `6px`。
//! 未在规范中规定、由本实现自定的值在注释里标「自定」。

use super::draw::{Color, DrawCommand, DrawList, Rect, TextAlign};
use windows::core::Result;

/// 标题轨道高（文件夹门户.md「公共标题轨道 28px」）。
pub const TITLE_BAR_HEIGHT: f32 = 28.0;
/// 导航（面包屑）轨道高（「导航轨道最低 28px」）。
pub const NAV_BAR_HEIGHT: f32 = 28.0;
/// 面包屑字号（「面包屑基准字号 12px」）。
pub const BREADCRUMB_FONT: f32 = 12.0;
/// 面包屑节点间距（「节点间距 4px」）。
pub const BREADCRUMB_GAP: f32 = 4.0;
/// 平铺项目尺寸（「默认项目尺寸 68×90px」）。
pub const CELL_WIDTH: f32 = 68.0;
pub const CELL_HEIGHT: f32 = 90.0;
/// 平铺间距（「横向间距 6px、纵向间距 8px」）。
pub const CELL_GAP_X: f32 = 6.0;
pub const CELL_GAP_Y: f32 = 8.0;
/// 系统图像尺寸（「系统图像 48px」）。
pub const THUMB_SIZE: f32 = 48.0;
/// 条目名称字体（「13px 名称」，平铺名称最多两行）。
pub const ITEM_FONT: f32 = 13.0;
/// 名称最多行数（「名称最多两行」）。
pub const ITEM_MAX_LINES: u16 = 2;
/// 内容内边距（自定：规范要求「外壳和带边框子项的容器保留至少 1px 内边距」，这里取 8 DIP 便于网格留白）。
pub const CONTENT_PADDING: f32 = 8.0;
/// 表面圆角（「min(8px, --surface-radius)」，当前默认 6px）。
pub const SURFACE_RADIUS: f32 = 6.0;
/// 最小可渲染宽：两侧内距 + 一个单元格（84 DIP）。
pub const MIN_WIDTH: f32 = CELL_WIDTH + CONTENT_PADDING * 2.0;
/// 最小可渲染高：标题 28 + 导航 28 + 上下内距 8×2 + 一整行单元格 90 = 162 DIP。
/// 之前漏掉内距写成 146，导致 84×146 会排出 0 行的越界布局。
pub const MIN_HEIGHT: f32 = TITLE_BAR_HEIGHT + NAV_BAR_HEIGHT + CONTENT_PADDING * 2.0 + CELL_HEIGHT;
/// 溢出提示带高（**诊断算法选择**：16.8 = 字号 12 × 1.4，规范未规定）。
pub const HINT_HEIGHT: f32 = 16.8;
/// 面包屑箭头宽（**诊断算法选择**，规范只要求「节点间距 4px」）。
pub const ARROW_WIDTH: f32 = 10.0;
/// 字宽估算系数（**诊断算法选择**：中文按字号 0.6、即 12px → 7.2；原生侧没有真实排版宽度）。
pub const CHAR_WIDTH: f32 = 7.2;
/// 名称与缩略图之间的上间距（**诊断算法选择**，规范未规定）。
pub const NAME_GAP: f32 = 4.0;
/// 名称行高系数（**诊断算法选择**：13 × 1.25；规范只说「最多两行」）。
pub const LINE_HEIGHT_FACTOR: f32 = 1.25;

/// 门户读取状态（对应现有实现的空 / 读取中 / 就绪）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortalState
{
    /// 目录读取中（现有语义：占位文案）。
    Loading,
    /// 读取完成但没有条目。
    Empty,
    /// 有内容。
    Ready,
}

impl PortalState
{
    /// 状态文案（空 / 读取中）。
    pub fn label(self) -> &'static str
    {
        match self
        {
            Self::Loading => "正在读取…",
            Self::Empty => "文件夹为空",
            Self::Ready => "",
        }
    }
}

/// 一个条目（名称 + 是否目录）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortalItem
{
    pub name: String,
    pub is_dir: bool,
}

/// 只读的门户视图数据。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PortalView
{
    pub folder: String,
    /// 面包屑层级（根 → 当前）；空表示未绑定。
    pub breadcrumb: Vec<String>,
    pub items: Vec<PortalItem>,
    pub state: PortalState,
}

impl Default for PortalState
{
    fn default() -> Self
    {
        Self::Loading
    }
}

/// 布局档位（与音乐格保持同一套「呈现目标」判等口径）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortalLayoutKind
{
    /// 宽布局：项目更大、每行更多列。
    Wide,
    /// 紧凑布局：项目按最小单元排布。
    Compact,
}

/// 绘制样式（进入去重键）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PortalStyle
{
    pub text: Color,
    pub fence_text: Color,
    pub dark: bool,
}

/// 一次布局的产物（内容盒局部 DIP）。
#[derive(Clone, Debug, PartialEq)]
pub struct PortalLayout
{
    pub kind: PortalLayoutKind,
    pub title: Rect,
    pub breadcrumb: Rect,
    /// 网格可用区域。
    pub grid: Rect,
    /// 每行可放列数。
    pub columns: usize,
    /// 可完整显示的行数。
    pub rows: usize,
    /// 是否降级：放不下「整行 + 提示带」，保留整行但放弃溢出提示。
    pub degraded: bool,
    /// 溢出提示区（有独立带宽；`reserve_hint=false` 时高为 0）。
    pub hint: Rect,
    /// 能完整显示的条目数（`columns × rows`）。
    pub visible: usize,
}

/// 按「数据 + 内容盒」决定布局：**先按无提示带算容量**，只有 Ready 且确实溢出才预留提示带；
/// 若「整行 + 提示带」放不下，则**分开降级**——保留整行容量、放弃提示（`degraded = true`），
/// 而不是为了提示把网格压到 0 行。
pub fn layout_for(kind: PortalLayoutKind, content: Rect, state: PortalState, total_items: usize) -> std::result::Result<PortalLayout, String>
{
    let base = layout_checked(kind, content, false)?;
    let overflows = state == PortalState::Ready && total_items > base.visible;
    if !overflows
    {
        return Ok(base);
    }
    match layout_checked(kind, content, true)
    {
        Ok(with_hint) => Ok(with_hint),
        // 放不下「整行 + 提示」：退回无带布局，标记降级（不画溢出提示）。
        Err(_) =>
        {
            let mut degraded = base;
            degraded.degraded = true;
            Ok(degraded)
        }
    }
}

/// 计算布局并做最小尺寸校验（与音乐格同一套门禁思路）。
pub fn layout_checked(kind: PortalLayoutKind, content: Rect, reserve_hint: bool) -> std::result::Result<PortalLayout, String>
{
    if !content.width.is_finite() || !content.height.is_finite() || !content.x.is_finite() || !content.y.is_finite()
    {
        return Err("门户内容盒含 NaN / 无穷，拒绝布局".to_string());
    }
    if content.width < 0.0 || content.height < 0.0
    {
        return Err(format!("门户内容盒为负（{:.1}x{:.1}），拒绝布局", content.width, content.height));
    }
    // 高度门禁含上下内距与（需要时）独立提示带：基准 162，带提示时 +16.8。
    let minimum_height = MIN_HEIGHT + if reserve_hint { HINT_HEIGHT } else { 0.0 };
    if content.width < MIN_WIDTH || content.height < minimum_height
    {
        return Err(format!("门户内容盒 {:.0}x{:.0} DIP 太小（至少 {MIN_WIDTH:.0}x{minimum_height:.1}）", content.width, content.height));
    }
    let title = Rect::new(content.x, content.y, content.width, TITLE_BAR_HEIGHT);
    let breadcrumb = Rect::new(content.x, title.bottom(), content.width, NAV_BAR_HEIGHT);
    // 溢出提示独占底部一带：网格高度先扣掉提示带，提示区与名称不再相交。
    let hint_height = if reserve_hint { HINT_HEIGHT } else { 0.0 };
    let grid_height = (content.bottom() - breadcrumb.bottom() - CONTENT_PADDING * 2.0 - hint_height).max(0.0);
    let grid = Rect::new(content.x + CONTENT_PADDING, breadcrumb.bottom() + CONTENT_PADDING, (content.width - CONTENT_PADDING * 2.0).max(0.0), grid_height);
    let hint = Rect::new(content.x + CONTENT_PADDING, content.bottom() - CONTENT_PADDING - HINT_HEIGHT, (content.width - CONTENT_PADDING * 2.0).max(0.0), hint_height);
    let (cell_w, gap_x) = match kind
    {
        PortalLayoutKind::Wide => (CELL_WIDTH, CELL_GAP_X),
        PortalLayoutKind::Compact => (CELL_WIDTH, CELL_GAP_X),
    };
    let columns = (((grid.width + gap_x) / (cell_w + gap_x)).floor() as usize).max(1);
    let rows = (((grid.height + CELL_GAP_Y) / (CELL_HEIGHT + CELL_GAP_Y)).floor() as usize).max(0);
    if rows == 0
    {
        return Err("网格放不下一整行，拒绝布局（不排 0 行）".to_string());
    }
    Ok(PortalLayout { kind, title, breadcrumb, grid, hint, columns, rows, visible: columns * rows, degraded: false })
}

/// 网格单元矩形（按行优先排列，左上起；行间距 8、列间距 6）。
pub fn grid_cells(layout: &PortalLayout, count: usize) -> Vec<Rect>
{
    let shown = count.min(layout.visible);
    (0..shown)
        .map(|index| {
            let column = index % layout.columns;
            let row = index / layout.columns;
            Rect::new(layout.grid.x + column as f32 * (CELL_WIDTH + CELL_GAP_X), layout.grid.y + row as f32 * (CELL_HEIGHT + CELL_GAP_Y), CELL_WIDTH, CELL_HEIGHT)
        })
        .collect()
}

/// 超出可视区的条目数（>0 时绘制「还有 N 项」标记）。
pub fn overflow_count(layout: &PortalLayout, total: usize) -> usize
{
    total.saturating_sub(layout.visible)
}

/// 绘制：标题、面包屑、网格占位方块与名称（单行两行截断）。
pub fn draw(view: &PortalView, layout: &PortalLayout, style: PortalStyle) -> DrawList
{
    let mut list = DrawList::new();
    let text = style.text;
    let muted = style.fence_text.with_alpha(0.72);
    let surface = if style.dark { Color::rgba(0x24, 0x2A, 0x2C, 0xFF) } else { Color::rgba(0xFF, 0xFF, 0xFF, 0xFF) };
    // 标题：目录名（空则显示「文件夹」）。
    let title = if view.folder.trim().is_empty() { "文件夹".to_string() } else { view.folder.clone() };
    list.push(DrawCommand::Text { rect: Rect::new(layout.title.x + CONTENT_PADDING, layout.title.y + (TITLE_BAR_HEIGHT - ITEM_FONT * 1.4) / 2.0, (layout.title.width - CONTENT_PADDING * 2.0).max(0.0), ITEM_FONT * 1.4), text: title, size: ITEM_FONT, weight: 500, color: text, align: TextAlign::Start, max_lines: 1 });
    // 面包屑：先算箭头与节点可用宽，再决定发哪些绘制指令；末节点留固定配额，绝不越过内容右边界。
    if !view.breadcrumb.is_empty()
    {
        let padding = CONTENT_PADDING;
        let left = layout.breadcrumb.x + padding;
        let right = layout.breadcrumb.x + layout.breadcrumb.width - padding;
        let mut nodes: Vec<String> = view.breadcrumb.clone();
        let node_width = |text: &str| (text.chars().count() as f32 * CHAR_WIDTH).min((right - left).max(0.0));
        // 估算超宽时折叠中间层级（首 / … / 末），末节点至少留 LAST_NODE_QUOTA。
        let total: f32 = nodes.iter().map(|node| node_width(node)).sum::<f32>() + (nodes.len().saturating_sub(1)) as f32 * (ARROW_WIDTH + BREADCRUMB_GAP);
        if total > right - left && nodes.len() > 2
        {
            nodes = vec![nodes.first().cloned().unwrap_or_default(), "…".to_string(), nodes.last().cloned().unwrap_or_default()];
        }
        const LAST_NODE_QUOTA: f32 = 24.0;
        let mut x = left;
        for (index, node) in nodes.iter().enumerate()
        {
            let remaining = (right - x).max(0.0);
            if remaining < LAST_NODE_QUOTA.min(ARROW_WIDTH + BREADCRUMB_GAP + 1.0)
            {
                break;
            }
            if index > 0
            {
                let arrow = ARROW_WIDTH.min(remaining);
                list.push(DrawCommand::Text { rect: Rect::new(x, layout.breadcrumb.y + 6.0, arrow, BREADCRUMB_FONT * 1.2), text: "›".to_string(), size: BREADCRUMB_FONT, weight: 400, color: muted, align: TextAlign::Start, max_lines: 1 });
                x += arrow + BREADCRUMB_GAP;
            }
            if x >= right
            {
                break;
            }
            let is_last = index + 1 == nodes.len();
            let budget = (right - x).max(0.0);
            // 末节点留配额：非末节点最多用到 budget − LAST_NODE_QUOTA，避免把末节点挤没。
            let width = if is_last { budget.min(node_width(node)) } else { (budget - LAST_NODE_QUOTA).max(0.0).min(node_width(node)) };
            if width <= 0.0
            {
                break;
            }
            list.push(DrawCommand::Text { rect: Rect::new(x, layout.breadcrumb.y + 6.0, width, BREADCRUMB_FONT * 1.2), text: node.clone(), size: BREADCRUMB_FONT, weight: 400, color: muted, align: TextAlign::Start, max_lines: 1 });
            x += width + BREADCRUMB_GAP;
        }
    }
    // 空 / 读取中：居中占位文案，不画网格。
    if view.state != PortalState::Ready
    {
        list.push(DrawCommand::Text { rect: Rect::new(layout.grid.x, layout.grid.y + layout.grid.height / 2.0 - ITEM_FONT, layout.grid.width, ITEM_FONT * 1.4), text: view.state.label().to_string(), size: ITEM_FONT, weight: 400, color: muted, align: TextAlign::Center, max_lines: 1 });
        return list;
    }
    // 网格：占位方块 + 名称（最多两行）。
    let cells = grid_cells(layout, view.items.len());
    for (cell, item) in cells.iter().zip(view.items.iter())
    {
        let thumb = Rect::new(cell.x + (CELL_WIDTH - THUMB_SIZE) / 2.0, cell.y, THUMB_SIZE, THUMB_SIZE);
        list.push(DrawCommand::FillRoundRect { rect: thumb, radius: SURFACE_RADIUS, color: surface });
        list.push(DrawCommand::StrokeRoundRect { rect: Rect::new(thumb.x + 0.5, thumb.y + 0.5, THUMB_SIZE - 1.0, THUMB_SIZE - 1.0), radius: SURFACE_RADIUS, width: 1.0, color: muted.with_alpha(0.5) });
        list.push(DrawCommand::Text { rect: Rect::new(cell.x, thumb.bottom() + 4.0, CELL_WIDTH, ITEM_FONT * 1.25 * f32::from(ITEM_MAX_LINES)), text: item.name.clone(), size: ITEM_FONT, weight: 400, color: text, align: TextAlign::Center, max_lines: ITEM_MAX_LINES });
    }
    // 超出可视区：提示画在**独立提示带**（layout.hint）里，与网格各行不重叠。
    let overflow = overflow_count(layout, view.items.len());
    if overflow > 0 && layout.hint.height > 0.0
    {
        list.push(DrawCommand::Text { rect: layout.hint, text: format!("还有 {overflow} 项"), size: BREADCRUMB_FONT, weight: 400, color: muted, align: TextAlign::Center, max_lines: 1 });
    }
    list
}

/// 「一次数据更新 → 一次绘制」：按「呈现目标」判等（数据 + 布局 + 样式）。
#[derive(Default)]
pub struct PortalPresenter
{
    last: Option<(PortalView, PortalLayout, PortalStyle)>,
    draws: usize,
    skipped: usize,
    /// 真正构造 DrawList 的次数（判等通过才 +1）。
    built: usize,
}

impl PortalPresenter
{
    /// 提交一帧；三者全等则不绘制（返回 `None`）。
    /// 提交一帧「数据 + 布局 + 样式」；**判等通过才执行 `build` 生成 DrawList**。
    pub fn submit(&mut self, view: &PortalView, layout: &PortalLayout, style: &PortalStyle, build: impl FnOnce() -> DrawList) -> Option<DrawList>
    {
        if let Some((last_view, last_layout, last_style)) = self.last.as_ref()
        {
            if last_view == view && last_layout == layout && last_style == style
            {
                self.skipped += 1;
                return None;
            }
        }
        let list = build();
        self.built += 1;
        self.last = Some((view.clone(), layout.clone(), *style));
        self.draws += 1;
        Some(list)
    }

    /// 真正构造出 `DrawList` 的次数。
    pub fn built(&self) -> usize
    {
        self.built
    }

    pub fn draws(&self) -> usize
    {
        self.draws
    }

    pub fn skipped(&self) -> usize
    {
        self.skipped
    }
}

/// 一次门户诊断的报告。
#[derive(Clone, Debug, serde::Serialize)]
pub struct PortalReport
{
    pub pid: u32,
    pub with_items: Vec<String>,
    pub empty_state: Vec<String>,
    pub draws_with_items: usize,
    pub skipped_with_items: usize,
    pub exit_code: i32,
}

/// 跑一次门户诊断：只摆数据、算布局、生成绘制列表并统计，不建窗、不写盘。
pub fn run_portal() -> Result<PortalReport>
{
    let style = PortalStyle { text: Color::rgba(0xE8, 0xEF, 0xF0, 0xFF), fence_text: Color::rgba(0xD2, 0xE0, 0xE3, 0xFF), dark: true };
    let view = PortalView {
        folder: "设计稿".into(),
        breadcrumb: vec!["桌面".into(), "项目".into(), "设计稿".into()],
        items: (0..10).map(|index| PortalItem { name: format!("很长的文件名示例-{index}.png"), is_dir: index % 3 == 0 }).collect(),
        state: PortalState::Ready,
    };
    let mut presenter = PortalPresenter::default();
    let mut with_items = Vec::new();
    for (kind, content) in [
        (PortalLayoutKind::Wide, Rect::new(0.0, 0.0, 396.0, 300.0)),
        (PortalLayoutKind::Compact, Rect::new(0.0, 0.0, 240.0, 260.0)),
    ]
    {
        // 容量先按无提示带算；只有 Ready 且确实溢出才预留提示带（放不下时降级为不画提示）。
        let layout = layout_for(kind, content, view.state, view.items.len()).map_err(|reason| windows::core::Error::new(windows::Win32::Foundation::E_FAIL, reason))?;
        // 只在判等通过时才生成绘制列表（不再先 draw 后 submit）。
        let list = presenter.submit(&view, &layout, &style, || draw(&view, &layout, style));
        let changed = list.is_some();
        let commands = list.as_ref().map(|list| list.commands().len()).unwrap_or(0);
        with_items.push(format!(
            "{kind:?}: content={:.0}x{:.0} columns={} rows={} visible={} cells={} overflow={} degraded={} commands={} changed={changed}",
            content.width,
            content.height,
            layout.columns,
            layout.rows,
            layout.visible,
            grid_cells(&layout, view.items.len()).len(),
            overflow_count(&layout, view.items.len()),
            layout.degraded,
            commands
        ));
    }
    let compact = layout_for(PortalLayoutKind::Compact, Rect::new(0.0, 0.0, 240.0, 260.0), view.state, view.items.len()).map_err(|reason| windows::core::Error::new(windows::Win32::Foundation::E_FAIL, reason))?;
    let drawn_again = presenter.submit(&view, &compact, &style, || draw(&view, &compact, style)).is_some();
    let skipped_again = presenter.submit(&view, &compact, &style, || draw(&view, &compact, style)).is_none();
    with_items.push(format!("重复提交同一「数据+布局+样式」：再绘制={drawn_again} 第二次跳过={skipped_again}"));
    let mut empty_state = Vec::new();
    for state in [PortalState::Empty, PortalState::Loading]
    {
        let view = PortalView { folder: "空文件夹".into(), breadcrumb: vec!["桌面".into()], items: Vec::new(), state };
        let list = draw(&view, &compact, style);
        let texts: Vec<String> = list.commands().iter().filter_map(|command| match command { DrawCommand::Text { text, .. } => Some(text.clone()), _ => None }).collect();
        empty_state.push(format!("{state:?}: texts={texts:?} commands={}", list.commands().len()));
    }
    Ok(PortalReport { pid: std::process::id(), with_items, empty_state, draws_with_items: presenter.draws(), skipped_with_items: presenter.skipped(), exit_code: 0 })
}

#[cfg(test)]
mod tests
{
    use super::*;

    fn style() -> PortalStyle
    {
        PortalStyle { text: Color::rgba(0xE8, 0xEF, 0xF0, 0xFF), fence_text: Color::rgba(0xD2, 0xE0, 0xE3, 0xFF), dark: true }
    }

    fn view(items: usize, state: PortalState) -> PortalView
    {
        PortalView { folder: "设计稿".into(), breadcrumb: vec!["桌面".into(), "项目".into()], items: (0..items).map(|index| PortalItem { name: format!("文件-{index}.png"), is_dir: false }).collect(), state }
    }

    /// 网格：列数 = floor((宽+6)/(68+6))，行数 = floor((高+8)/(90+8))，行列间距 6/8。
    #[test]
    fn grid_geometry_follows_spec()
    {
        let content = Rect::new(0.0, 0.0, 396.0, 300.0);
        let layout = layout_checked(PortalLayoutKind::Wide, content, true).expect("应放行");
        assert_eq!(layout.title.height, 28.0, "标题轨道 28");
        assert_eq!(layout.breadcrumb.height, 28.0, "导航轨道最低 28");
        // 网格宽 = 396 - 8*2 = 380 → (380+6)/74 = 5 列；高 = 300-56-16 = 228 → (228+8)/98 = 2 行。
        assert_eq!(layout.columns, 5);
        assert_eq!(layout.rows, 2);
        let cells = grid_cells(&layout, 10);
        assert_eq!(cells.len(), 10, "可视 5×2=10 个");
        assert_eq!(cells[1].x - cells[0].x, CELL_WIDTH + CELL_GAP_X, "列间距 6");
        assert_eq!(cells[5].y - cells[0].y, CELL_HEIGHT + CELL_GAP_Y, "行间距 8");
        assert_eq!(cells[0].width, CELL_WIDTH);
        assert_eq!(cells[0].height, CELL_HEIGHT);
    }

    /// 超出可视区：只画可视数量，并给出「还有 N 项」标记。
    #[test]
    fn overflow_is_truncated_with_marker()
    {
        let layout = layout_checked(PortalLayoutKind::Wide, Rect::new(0.0, 0.0, 396.0, 300.0), true).expect("应放行");
        let data = view(13, PortalState::Ready);
        assert_eq!(grid_cells(&layout, data.items.len()).len(), layout.visible, "只画可视条目");
        assert_eq!(overflow_count(&layout, data.items.len()), 13 - layout.visible);
        let list = draw(&data, &layout, style());
        let texts: Vec<String> = list.commands().iter().filter_map(|command| match command { DrawCommand::Text { text, .. } => Some(text.clone()), _ => None }).collect();
        assert!(texts.iter().any(|text| text.starts_with("还有 ")), "缺少溢出标记：{texts:?}");
    }

    /// 长文件名：名称最多两行（`max_lines = 2`），不换列也不越出单元格。
    #[test]
    fn long_names_are_two_lines_and_inside_cells()
    {
        let layout = layout_checked(PortalLayoutKind::Wide, Rect::new(0.0, 0.0, 396.0, 300.0), true).expect("应放行");
        let mut data = view(2, PortalState::Ready);
        data.items[0].name = "非常非常非常非常非常非常非常非常长的文件名.png".into();
        let list = draw(&data, &layout, style());
        let name_text = list.commands().iter().find_map(|command| match command {
            DrawCommand::Text { rect, max_lines, text, .. } if text.contains("非常") => Some((*rect, *max_lines)),
            _ => None,
        });
        let (rect, max_lines) = name_text.expect("应画名称");
        assert_eq!(max_lines, ITEM_MAX_LINES, "名称最多两行");
        let cell = grid_cells(&layout, 2)[0];
        assert!(rect.x >= cell.x - 0.01 && rect.right() <= cell.right() + 0.01, "名称不得越出单元格");
    }

    /// 空态与读取中：居中占位文案，不画网格方块。
    #[test]
    fn empty_and_loading_states_show_placeholder()
    {
        let layout = layout_checked(PortalLayoutKind::Compact, Rect::new(0.0, 0.0, 240.0, 260.0), true).expect("应放行");
        for (state, expected) in [(PortalState::Empty, "文件夹为空"), (PortalState::Loading, "正在读取…")]
        {
            let data = view(0, state);
            let list = draw(&data, &layout, style());
            let texts: Vec<String> = list.commands().iter().filter_map(|command| match command { DrawCommand::Text { text, .. } => Some(text.clone()), _ => None }).collect();
            assert!(texts.iter().any(|text| text == expected), "{state:?} 缺少占位文案：{texts:?}");
            assert!(!list.commands().iter().any(|command| matches!(command, DrawCommand::FillRoundRect { .. })), "空/读取中不画方块");
        }
    }

    /// 边界：过小盒拒绝；正常盒内所有元素都在内容盒内。
    #[test]
    fn tiny_boxes_rejected_and_elements_contained()
    {
        assert!(layout_checked(PortalLayoutKind::Compact, Rect::new(0.0, 0.0, 60.0, 60.0), true).is_err(), "过小盒必须拒绝");
        assert!(layout_checked(PortalLayoutKind::Wide, Rect::new(0.0, 0.0, 80.0, 150.0), true).is_err(), "宽不足（< 68+8×2）必须拒绝");
        let content = Rect::new(10.0, 20.0, 396.0, 300.0);
        let layout = layout_checked(PortalLayoutKind::Wide, content, true).expect("应放行");
        for (label, rect) in [("title", layout.title), ("breadcrumb", layout.breadcrumb), ("grid", layout.grid)]
        {
            assert!(rect.x >= content.x - 0.01 && rect.right() <= content.right() + 0.01, "{label} 横向越界");
            assert!(rect.y >= content.y - 0.01 && rect.bottom() <= content.bottom() + 0.01, "{label} 纵向越界");
        }
        for cell in grid_cells(&layout, 10)
        {
            assert!(cell.right() <= layout.grid.right() + 0.01 && cell.bottom() <= layout.grid.bottom() + 0.01, "单元格越出网格：{cell:?}");
        }
    }

    /// F1：判等通过才构造 DrawList —— 同一呈现目标连提三次，实际构造次数 = 1、跳过 2。
    #[test]
    fn presenter_builds_only_on_change()
    {
        let mut presenter = PortalPresenter::default();
        let style = style();
        let wide = layout_checked(PortalLayoutKind::Wide, Rect::new(0.0, 0.0, 396.0, 300.0), true).expect("应放行");
        let compact = layout_checked(PortalLayoutKind::Compact, Rect::new(0.0, 0.0, 240.0, 260.0), true).expect("应放行");
        let data = view(3, PortalState::Ready);
        let build = |view: &PortalView, layout: &PortalLayout| draw(view, layout, style);
        assert!(presenter.submit(&data, &wide, &style, || build(&data, &wide)).is_some());
        assert!(presenter.submit(&data, &wide, &style, || build(&data, &wide)).is_none());
        assert!(presenter.submit(&data, &wide, &style, || build(&data, &wide)).is_none());
        assert_eq!(presenter.built(), 1, "同值三次只构造一次 DrawList");
        assert_eq!((presenter.draws(), presenter.skipped()), (1, 2));
        // 布局变化。
        assert!(presenter.submit(&data, &compact, &style, || build(&data, &compact)).is_some());
        assert_eq!(presenter.built(), 2, "布局变化必须构造一次");
        // 条目变化。
        let mut more = data.clone();
        more.items.push(PortalItem { name: "新文件.txt".into(), is_dir: false });
        assert!(presenter.submit(&more, &compact, &style, || build(&more, &compact)).is_some());
        assert_eq!(presenter.built(), 3, "条目变化必须构造一次");
        // 样式变化。
        let mut light = style;
        light.dark = false;
        assert!(presenter.submit(&more, &compact, &light, || build(&more, &compact)).is_some());
        assert_eq!(presenter.built(), 4, "样式变化必须构造一次");
    }

    /// F2：最小尺寸含上下 8 DIP 内距 —— 84×146 拒绝、84×162 通过；NaN / 无穷 / 负值一律拒绝。
    #[test]
    fn minimum_size_includes_padding_and_rejects_non_finite()
    {
        assert_eq!(MIN_WIDTH, 84.0, "宽 = 68 + 8×2");
        assert_eq!(MIN_HEIGHT, 162.0, "高 = 28 + 28 + 8×2 + 90");
        let rejected = layout_checked(PortalLayoutKind::Compact, Rect::new(0.0, 0.0, 84.0, 146.0), false);
        assert!(rejected.is_err(), "84×146 必须拒绝（容纳不下一整行）");
        // 带提示带时阈值再 +16.8：84×162 放不下提示带，84×178.8 才够。
        assert!(layout_checked(PortalLayoutKind::Compact, Rect::new(0.0, 0.0, 84.0, 162.0), true).is_err(), "带提示带 84×162 必须拒绝（需 178.8）");
        assert_eq!(layout_checked(PortalLayoutKind::Compact, Rect::new(0.0, 0.0, 84.0, 178.8), true).expect("84×178.8 应通过").rows, 1);
        let accepted = layout_checked(PortalLayoutKind::Compact, Rect::new(0.0, 0.0, 84.0, 162.0), false);
        let layout = accepted.expect("84×162 应通过");
        assert_eq!(layout.rows, 1, "最小合法尺寸恰好一行");
        assert!(layout.grid.height >= CELL_HEIGHT, "网格必须容得下一整行");
        // 非有限值与负值。
        assert!(layout_checked(PortalLayoutKind::Compact, Rect::new(0.0, 0.0, f32::NAN, 300.0), true).is_err(), "NaN 宽必须拒绝");
        assert!(layout_checked(PortalLayoutKind::Compact, Rect::new(0.0, 0.0, 396.0, f32::INFINITY), true).is_err(), "无穷高必须拒绝");
        assert!(layout_checked(PortalLayoutKind::Compact, Rect::new(0.0, 0.0, -10.0, 300.0), true).is_err(), "负宽必须拒绝");
    }

    /// F3：溢出提示在独立提示带内，不与第二行名称相交；面包屑在 84 宽下不越右边界。
    #[test]
    fn hint_band_and_breadcrumb_stay_inside()
    {
        let style = style();
        // 240×260：两行网格 + 提示带。
        let layout = layout_checked(PortalLayoutKind::Compact, Rect::new(0.0, 0.0, 240.0, 260.0), true).expect("应放行");
        let data = view(13, PortalState::Ready);
        let list = draw(&data, &layout, style);
        let mut hint: Option<Rect> = None;
        let mut name_rects: Vec<Rect> = Vec::new();
        for command in list.commands()
        {
            if let DrawCommand::Text { rect, text, .. } = command
            {
                if text.starts_with("还有 ") { hint = Some(*rect); }
                else if text.starts_with("文件-") { name_rects.push(*rect); }
            }
        }
        let hint = hint.expect("溢出时应有提示");
        assert!(hint.height > 0.0, "提示必须有独立高度");
        for name in &name_rects
        {
            let overlap = (hint.y.min(name.bottom()) - name.y.max(hint.y)).max(0.0);
            assert!(overlap <= 0.01, "提示不得与名称相交：hint={hint:?} name={name:?}");
        }
        assert!(hint.y >= layout.grid.bottom() - 0.01, "提示带在网格之下");
        // 84 宽：面包屑不得越出内容右边界。
        let narrow = layout_checked(PortalLayoutKind::Compact, Rect::new(0.0, 0.0, 84.0, 162.0), false).expect("84×162（无提示带）应通过");
        let mut deep = view(2, PortalState::Ready);
        deep.breadcrumb = vec!["桌面".into(), "项目".into(), "设计稿".into(), "很长的目录名字".into()];
        let list = draw(&deep, &narrow, style);
        let right = narrow.breadcrumb.right() - CONTENT_PADDING;
        for command in list.commands()
        {
            if let DrawCommand::Text { rect, .. } = command
            {
                assert!(rect.right() <= right + 0.01, "文本越出内容右边界：{rect:?} > {right:.1}");
                assert!(rect.width > 0.0, "零宽文本不应发出绘制指令：{rect:?}");
            }
        }
    }
}
