//! 命中测试与焦点：纯逻辑，不依赖任何 Win32 句柄。
//!
//! 与绘制共用同一套矩形语义（见 [`super::workspace_scene`]）：
//! - 命中范围是格子的**边框盒**（`.desktop-tile` 用 `box-sizing: border-box` + `padding: 1px`，
//!   1 DIP 边框画在盒子内侧），所以「盒子矩形本身」就是命中范围；
//! - 矩形包含判定左闭右开，与 DOM 命中一致：正好落在右边/下边的点属于相邻格子，不属于自己；
//! - 标题轨道高度与绘制同源：折叠或非 hover 标题取 `min(28, 高度)`，hover/none 标题且未折叠取 0；
//! - 层级按持久 `z_index` 升序（后画的在上），同一 z 保持快照顺序；
//! - **焦点格子临时置顶**：`DesktopTileBase.css` 的 `[data-keyboard-focus="true"] { z-index: 17000 }`
//!   是临时层级、不写布局，命中与绘制都按 17000 参与排序。
//!
//! 焦点只存在于内存：点击空白失焦、Tab/Shift+Tab 顺序移动、方向键按几何就近移动；
//! 本模块不写任何偏好或布局文件。

use windows::core::Result;

use super::{
    draw::{Color, DrawList, Rect},
    geometry::{DpiScale, PixelRect},
};

/// 前端焦点态用的临时层级（`DesktopTileBase.css` 的 `z-index: 17000`）。
///
/// 原生不比较这个数值：`scene_blocks` 直接把焦点块移到块列表末尾，绘制与命中同时生效，
/// 效果等价于临时置顶，且不改持久 `z_index`。
pub const FOCUS_Z: i32 = 17_000;

/// 标题轨道高度（DIP），与 `.desktop-tile` 的 `grid-template-rows` 默认值一致。
pub const HEADER: f32 = 28.0;

/// 命中目标的种类。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetKind
{
    Fence,
    Component,
    Portal,
    Item,
    /// 工作区背景：整块表面，最低优先级；点它等于点空白。
    Background,
}

impl TargetKind
{
    /// 日志用的短名。
    pub fn as_str(&self) -> &'static str
    {
        match self
        {
            Self::Fence => "fence",
            Self::Component => "component",
            Self::Portal => "component",
            Self::Item => "item",
            Self::Background => "workspace-background",
        }
    }
}

/// 格子（围栏/组件/门户/未归类项）内部被命中的区域。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HitRegion
{
    /// 标题轨道（含折叠态标题）。
    Header,
    /// 内容区（标题轨道以下）。
    Content,
    /// 内容区里的一个成员（围栏成员或门户条目），带成员稳定身份。
    Member(String),
    /// 工作区背景（点它等于点空白）。
    Background,
}

impl HitRegion
{
    /// 日志用的短名。
    pub fn as_str(&self) -> String
    {
        match self
        {
            Self::Header => "header".to_string(),
            Self::Content => "content".to_string(),
            Self::Member(id) => format!("member:{id}"),
            Self::Background => "background".to_string(),
        }
    }
}

/// 一个可命中的格子。
#[derive(Clone, Debug)]
pub struct HitEntry
{
    pub id: String,
    pub kind: TargetKind,
    /// 边框盒矩形（工作区局部 DIP），与绘制使用同一个矩形。
    pub rect: Rect,
    /// 标题轨道高度；0 表示没有标题轨道（hover/none 标题且未折叠）。
    pub header: f32,
    /// 内容区里的成员矩形（工作区局部 DIP），按绘制顺序。
    pub members: Vec<(String, Rect)>,
    pub z: i32,
    /// 同一层级内的命中优先级：与前端 `data-hit-priority` 一致
    /// （workspace-background 1 < fence 50 < portal = component 60 < desktop-item = 标题工具 70）。
    pub priority: i32,
    /// 可见区域：格子的命中范围要与它求交（前端把矩形夹到视口与围栏内容框）。
    pub clip: Rect,
}

impl HitEntry
{
    /// 直接给出全部命中所需字段；`clip` 是该格子可见区域（视口与内容框的交集）。
    pub fn new(id: impl Into<String>, kind: TargetKind, rect: Rect, header: f32, members: Vec<(String, Rect)>, priority: i32, z: i32, clip: Option<Rect>) -> Self
    {
        Self { id: id.into(), kind, rect, header, members, z, priority, clip: clip.unwrap_or(rect) }
    }

    /// 命中范围 = 绘制矩形 ∩ 可见区域；完全不可见时返回 None（不会被点到）。
    pub fn visible(&self) -> Option<Rect>
    {
        let left = self.rect.x.max(self.clip.x);
        let top = self.rect.y.max(self.clip.y);
        let right = self.rect.right().min(self.clip.right());
        let bottom = self.rect.bottom().min(self.clip.bottom());
        if right > left && bottom > top { Some(Rect::new(left, top, right - left, bottom - top)) } else { None }
    }

    /// 焦点候选：背景不是格子，不能被聚焦。
    pub fn focusable(&self) -> bool
    {
        self.kind != TargetKind::Background
    }
}

/// 命中结果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit
{
    pub id: String,
    pub kind: TargetKind,
    pub region: HitRegion,
    /// 在 [`HitScene::entries`] 里的位置（稳定排序后）。
    pub entry: usize,
}

/// 一屏的命中场景。
pub struct HitScene
{
    entries: Vec<HitEntry>,
    order: Vec<usize>,
    focused: Option<usize>,
}

impl HitScene
{
    /// 按**调用方给的顺序**建立场景：这个顺序必须就是绘制顺序（低 → 高）。
    ///
    /// 本类型不再排序，避免出现「绘制按 z、命中按 (z, priority)」两套规则；
    /// 生产路径的顺序来自 `workspace_scene::scene_blocks`，焦点块在那里已经移到末尾。
    pub fn new(entries: Vec<HitEntry>, focused: Option<&str>) -> Self
    {
        let mut scene = Self { entries, order: Vec::new(), focused: None };
        scene.order = (0..scene.entries.len()).collect();
        if let Some(id) = focused
        {
            scene.focused = scene.order.iter().copied().find(|index| scene.entries[*index].id == id);
        }
        scene
    }

    /// 稳定排序后的条目（`entries()` 的顺序就是绘制顺序）。
    pub fn entries(&self) -> Vec<&HitEntry>
    {
        self.order.iter().map(|index| &self.entries[*index]).collect()
    }

    /// 焦点格子 id。
    pub fn focused_id(&self) -> Option<&str>
    {
        self.focused.map(|index| self.entries[index].id.as_str())
    }

    /// 焦点候选：排除背景，按 (top, left, id) 的阅读顺序。
    pub fn focus_candidates(&self) -> Vec<&HitEntry>
    {
        let mut candidates: Vec<&HitEntry> = self.entries.iter().filter(|entry| entry.focusable()).collect();
        candidates.sort_by(|a, b| a.rect.y.total_cmp(&b.rect.y).then(a.rect.x.total_cmp(&b.rect.x)).then(a.id.cmp(&b.id)));
        candidates
    }

    /// Tab 顺序：只在焦点候选之间，按 (top, left) 的阅读顺序。
    pub fn reading_order(&self) -> Vec<String>
    {
        let mut indexes: Vec<usize> = (0..self.entries.len()).filter(|index| self.entries[*index].focusable()).collect();
        indexes.sort_by(|a, b| {
            let (a, b) = (&self.entries[*a], &self.entries[*b]);
            a.rect.y.total_cmp(&b.rect.y).then(a.rect.x.total_cmp(&b.rect.x)).then(a.id.cmp(&b.id))
        });
        indexes.into_iter().map(|index| self.entries[index].id.clone()).collect()
    }

    /// 命中：从最上层往下找第一个包含该点的条目（DIP 坐标）。
    pub fn hit(&self, x: f32, y: f32) -> Option<Hit>
    {
        for index in self.order.iter().rev()
        {
            let entry = &self.entries[*index];
            // 命中范围必须裁到可见区域：被内容框或视口裁掉的片段不能中。
            let Some(visible) = entry.visible() else { continue };
            if visible.contains(x, y)
            {
                let region = region_at(entry, x, y);
                return Some(Hit { id: entry.id.clone(), kind: entry.kind, region, entry: *index });
            }
        }
        None
    }

    /// 物理像素坐标 → DIP 后命中；`origin` 是该显示器工作区左上角的物理像素。
    pub fn hit_physical(&self, physical_x: i32, physical_y: i32, origin: PixelRect, dpi: DpiScale) -> Option<Hit>
    {
        let (x, y) = dpi.px_point_to_dip(physical_x - origin.left, physical_y - origin.top);
        self.hit(x, y)
    }

    /// 取某个条目的矩形（焦点边框重画用）。
    pub fn rect_of(&self, id: &str) -> Option<Rect>
    {
        self.entries.iter().find(|entry| entry.id == id).map(|entry| entry.rect)
    }
}

/// 在条目内部继续判定区域：先标题轨道，再成员矩形，最后内容区。
fn region_at(entry: &HitEntry, x: f32, y: f32) -> HitRegion
{
    if entry.kind == TargetKind::Background
    {
        return HitRegion::Background;
    }
    if entry.header > 0.0 && y < entry.rect.y + entry.header
    {
        return HitRegion::Header;
    }
    // 成员只按「实际绘制矩形 ∩ 内容框 ∩ 视口」判定：上缘半露可命中，被裁掉的部分不能命中。
    let content_top = entry.rect.y + entry.header;
    if y >= content_top
    {
        for (id, rect) in &entry.members
        {
            if rect.contains(x, y)
            {
                return HitRegion::Member(id.clone());
            }
        }
    }
    HitRegion::Content
}

/// 一次焦点输入。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusAction
{
    /// 鼠标点击：命中格子则聚焦，命中空白则失焦。
    Click,
    Next,
    Previous,
    Left,
    Right,
    Up,
    Down,
}

impl FocusAction
{
    /// 日志用的短名。
    pub fn as_str(&self) -> &'static str
    {
        match self
        {
            Self::Click => "click",
            Self::Next => "tab",
            Self::Previous => "shift+tab",
            Self::Left => "left",
            Self::Right => "right",
            Self::Up => "up",
            Self::Down => "down",
        }
    }
}

/// 焦点状态：只在这里保存，不落盘。
#[derive(Clone, Debug, Default)]
pub struct FocusState
{
    focused: Option<String>,
}

impl FocusState
{
    /// 当前焦点格子。
    pub fn focused_id(&self) -> Option<&str>
    {
        self.focused.as_deref()
    }

    /// 应用一次输入，返回焦点变化（`None` 表示没有变化）。
    pub fn apply(&mut self, action: FocusAction, scene: &HitScene, hit: Option<&Hit>) -> Option<(Option<String>, Option<String>)>
    {
        let before = self.focused.clone();
        match action
        {
            // 命中工作区背景等于点空白：只失焦，不把背景当成可聚焦对象。
            FocusAction::Click => self.focused = hit.filter(|hit| hit.kind != TargetKind::Background).map(|hit| hit.id.clone()),
            FocusAction::Next | FocusAction::Previous =>
            {
                // 只在焦点候选（非背景的格子）之间移动；没有候选时保持无焦点。
                let order = scene.reading_order();
                if !order.is_empty()
                {
                    let step = if action == FocusAction::Next { 1i32 } else { -1 };
                    let current = self.focused.as_ref().and_then(|id| order.iter().position(|value| value == id));
                    let next = match current
                    {
                        Some(index) => (index as i32 + step).rem_euclid(order.len() as i32) as usize,
                        None => if step > 0 { 0 } else { order.len() - 1 },
                    };
                    self.focused = Some(order[next].clone());
                }
            }
            FocusAction::Left | FocusAction::Right | FocusAction::Up | FocusAction::Down =>
            {
                if let Some(next) = nearest(scene, self.focused.as_deref(), action)
                {
                    self.focused = Some(next);
                }
            }
        }
        if before == self.focused { None } else { Some((before, self.focused.clone())) }
    }

    /// 直接设置（诊断脚本里用于还原初始状态）。
    pub fn set(&mut self, id: Option<String>)
    {
        self.focused = id;
    }
}

/// 方向键：只在焦点候选里找，且必须整体在给定方向上。
///
/// 规则（与正式工作区的「当前容器优先」同构，这里是壳级近似）：
/// 1. 候选必须在这一方向上（严格大于 0.5 DIP）；
/// 2. 先选**正交投影有重叠**的候选（同一行/同一列），按方向上的间隙取最近；
/// 3. 没有投影重叠时才按「间隙 + 0.5 × 横移」取最近；
/// 4. 候选为空或无焦点时取阅读顺序的第一个（不选背景）。
fn nearest(scene: &HitScene, focused: Option<&str>, action: FocusAction) -> Option<String>
{
    let candidates = scene.focus_candidates();
    let Some(current) = candidates.iter().find(|entry| Some(entry.id.as_str()) == focused) else {
        return candidates.first().map(|entry| entry.id.clone());
    };
    let (cx, cy) = (current.rect.x + current.rect.width / 2.0, current.rect.y + current.rect.height / 2.0);
    let mut overlapping: Option<(f32, &str)> = None;
    let mut fallback: Option<(f32, &str)> = None;
    for entry in candidates.iter()
    {
        let (ex, ey) = (entry.rect.x + entry.rect.width / 2.0, entry.rect.y + entry.rect.height / 2.0);
        // 方向上的间隙与正交方向的横移（用矩形区间判断是否投影重叠）。
        let (gap, cross, projected) = match action
        {
            FocusAction::Left => (current.rect.x - entry.rect.right(), (cy - ey).abs(), entry.rect.y < current.rect.bottom() && entry.rect.bottom() > current.rect.y),
            FocusAction::Right => (entry.rect.x - current.rect.right(), (cy - ey).abs(), entry.rect.y < current.rect.bottom() && entry.rect.bottom() > current.rect.y),
            FocusAction::Up => (current.rect.y - entry.rect.bottom(), (cx - ex).abs(), entry.rect.x < current.rect.right() && entry.rect.right() > current.rect.x),
            _ => (entry.rect.y - current.rect.bottom(), (cx - ex).abs(), entry.rect.x < current.rect.right() && entry.rect.right() > current.rect.x),
        };
        if gap <= 0.5
        {
            continue;
        }
        if projected && overlapping.map(|(value, _)| gap < value).unwrap_or(true)
        {
            overlapping = Some((gap, &entry.id));
        }
        let score = gap + cross * 0.5;
        if fallback.map(|(value, _)| score < value).unwrap_or(true)
        {
            fallback = Some((score, &entry.id));
        }
    }
    overlapping.or(fallback).map(|(_, id)| id.to_string())
}

/// 焦点边框颜色：等价于 `--fence-focus-border`（浅深两套主题都取强调色）。
pub fn focus_border_color(accent: &str) -> Color
{
    parse_hex_color(accent, Color::rgba(0x25, 0x63, 0xEB, 0xFF))
}

impl Color
{
    /// 取 `#RRGGBB` 文本（忽略 alpha），报告里复述的就是实际绘制用的值。
    pub fn to_hex_rgb(&self) -> String
    {
        let [red, green, blue, _] = self.to_unit();
        format!("#{:02X}{:02X}{:02X}", (red * 255.0).round() as u8, (green * 255.0).round() as u8, (blue * 255.0).round() as u8)
    }
}

/// 解析 `#rrggbb` / `rrggbb`；非法输入返回兜底色。
pub fn parse_hex_color(value: &str, fallback: Color) -> Color
{
    let text = value.trim().trim_start_matches('#');
    if text.len() != 6 || !text.chars().all(|c| c.is_ascii_hexdigit())
    {
        return fallback;
    }
    let channel = |range: std::ops::Range<usize>| u8::from_str_radix(&text[range], 16).unwrap_or(0);
    Color::rgba(channel(0..2), channel(2..4), channel(4..6), 0xFF)
}

#[cfg(test)]
mod tests
{
    use super::*;
    use crate::contracts::settings::ApplicationPreferences;
    use crate::render::geometry::{DpiScale, HostGeometry, PixelRect};

    /// 造一条围栏条目（`clip` 默认等于自身矩形）。
    fn entry(id: &str, x: f32, y: f32, w: f32, h: f32, header: f32, z: i32, members: &[(&str, Rect)]) -> HitEntry
    {
        HitEntry {
            id: id.to_string(),
            kind: TargetKind::Fence,
            rect: Rect::new(x, y, w, h),
            header,
            members: members.iter().map(|(id, rect)| (id.to_string(), *rect)).collect(),
            z,
            priority: 50,
            clip: Rect::new(x, y, w, h),
        }
    }

    /// 造一条带可见区域裁剪的条目（成员矩形也按裁剪后的可见范围给出）。
    fn clipped(id: &str, rect: Rect, clip: Rect, kind: TargetKind, priority: i32, z: i32, members: &[(&str, Rect)]) -> HitEntry
    {
        HitEntry::new(id, kind, rect, 0., members.iter().map(|(id, rect)| (id.to_string(), *rect)).collect(), priority, z, Some(clip))
    }

    /// F1：条目顺序就是绘制顺序，命中按逆序查找——画在上面的先被点到。
    #[test]
    fn paint_order_is_hit_order()
    {
        let low = entry("low", 0.0, 0.0, 200.0, 200.0, HEADER, 1000, &[]);
        let high = entry("high", 50.0, 50.0, 200.0, 200.0, HEADER, 2000, &[]);
        // 调用方给的就是绘制顺序：low 先画，high 后画。
        let scene = HitScene::new(vec![low, high], None);
        assert_eq!(scene.hit(100.0, 100.0).unwrap().id, "high", "后画的先命中");
        assert_eq!(scene.hit(10.0, 10.0).unwrap().id, "low");
    }

    /// F1：同层时后画者胜（门户在组件之后 ⇒ 门户先命中），不再出现「画组件、命中门户」分叉。
    #[test]
    fn same_layer_later_block_wins_both_orders()
    {
        let component = clipped("component", Rect::new(0.0, 0.0, 120.0, 120.0), Rect::new(0.0, 0.0, 400.0, 400.0), TargetKind::Component, 60, 1000, &[]);
        let portal = clipped("portal", Rect::new(20.0, 20.0, 120.0, 120.0), Rect::new(0.0, 0.0, 400.0, 400.0), TargetKind::Portal, 60, 1000, &[]);
        let scene = HitScene::new(vec![component.clone(), portal.clone()], None);
        assert_eq!(scene.hit(60.0, 60.0).unwrap().id, "portal");
        // 焦点块（组件）被移到末尾后，命中与绘制一起变成组件在上。
        let raised = HitScene::new(vec![portal, component], Some("component"));
        assert_eq!(raised.hit(60.0, 60.0).unwrap().id, "component");
        assert_eq!(raised.focused_id(), Some("component"));
    }

    /// 越界与右/下边界：矩形外、右边界、下边界都不命中（左闭右开）。
    #[test]
    fn outside_and_right_edges_miss()
    {
        let scene = HitScene::new(vec![entry("a", 100.0, 100.0, 200.0, 150.0, HEADER, 0, &[])], None);
        assert!(scene.hit(50.0, 50.0).is_none());
        assert!(scene.hit(100.0, 100.0).is_some());
        assert!(scene.hit(299.9, 249.9).is_some());
        assert!(scene.hit(300.0, 200.0).is_none(), "右边界属于相邻格子");
        assert!(scene.hit(200.0, 250.0).is_none(), "下边界同理");
    }

    /// 描边边界：1 DIP 边框在边框盒内侧；盒内 0.5 DIP 命中，盒外 0.5 DIP 不命中。
    #[test]
    fn border_belongs_to_the_box()
    {
        let scene = HitScene::new(vec![entry("a", 100.0, 100.0, 200.0, 150.0, HEADER, 0, &[])], None);
        assert_eq!(scene.hit(100.5, 130.0).unwrap().id, "a");
        assert!(scene.hit(99.5, 130.0).is_none());
        assert_eq!(scene.hit(200.0, 100.0).unwrap().region, HitRegion::Header);
        assert_eq!(scene.hit(200.0, 127.9).unwrap().region, HitRegion::Header);
        assert_eq!(scene.hit(200.0, 128.0).unwrap().region, HitRegion::Content);
    }

    /// F2：成员按「绘制矩形 ∩ 内容框 ∩ 视口」判定——上缘半露可命中，下缘/左右被裁不可命中。
    #[test]
    fn member_clipping_covers_four_edges()
    {
        // 视口 0..400，围栏内容框 0..200（成员在 y=150..250，下缘被内容框裁到 200）。
        let viewport = Rect::new(0.0, 0.0, 400.0, 400.0);
        let content = Rect::new(0.0, 0.0, 400.0, 200.0);
        let clip = crate::render::workspace_scene::intersect(content, viewport).unwrap();
        // 成员在内容框上方 20 DIP 处露出一半（模拟向上滚动）：可见部分是 y=0..30。
        let half_hidden = Rect::new(50.0, -20.0, 100.0, 50.0);
        let visible = crate::render::workspace_scene::intersect(half_hidden, clip).unwrap();
        assert_eq!((visible.x, visible.y, visible.width, visible.height), (50.0, 0.0, 100.0, 30.0));
        let scene = HitScene::new(vec![clipped("fence", Rect::new(0.0, 0.0, 400.0, 200.0), viewport, TargetKind::Fence, 50, 1000, &[("t", visible)])], None);
        assert_eq!(scene.hit(60.0, 10.0).unwrap().region, HitRegion::Member("t".into()), "上缘半露仍可命中");
        assert_eq!(scene.hit(60.0, 45.0).unwrap().region, HitRegion::Content, "被裁掉的部分不算命中成员");

        // 下缘与左右被裁：可见区域之外不命中该成员。
        let bottom_cut = crate::render::workspace_scene::intersect(Rect::new(50.0, 180.0, 100.0, 100.0), clip).unwrap();
        assert_eq!(bottom_cut.height, 20.0);
        let side_cut = crate::render::workspace_scene::intersect(Rect::new(-40.0, 10.0, 100.0, 40.0), clip).unwrap();
        assert_eq!((side_cut.x, side_cut.width), (0.0, 60.0));
        let scene = HitScene::new(vec![clipped("fence", Rect::new(0.0, 0.0, 400.0, 200.0), viewport, TargetKind::Fence, 50, 1000, &[("bottom", bottom_cut), ("side", side_cut)])], None);
        assert_eq!(scene.hit(60.0, 195.0).unwrap().region, HitRegion::Member("bottom".into()));
        assert_eq!(scene.hit(10.0, 20.0).unwrap().region, HitRegion::Member("side".into()));
        assert!(scene.hit(60.0, 210.0).is_none(), "内容框之外的成员片段不命中");
    }

    /// F2：整块格子也要裁到视口——视口外的部分不命中。
    #[test]
    fn entry_clipped_to_viewport_never_hits_outside()
    {
        let viewport = Rect::new(0.0, 0.0, 200.0, 200.0);
        let scene = HitScene::new(vec![clipped("big", Rect::new(-50.0, 100.0, 400.0, 50.0), viewport, TargetKind::Component, 60, 0, &[])], None);
        assert!(scene.hit(10.0, 120.0).is_some(), "视口内的部分是可见的");
        assert!(scene.hit(-20.0, 120.0).is_none(), "视口左侧之外不可命中");
        assert!(scene.hit(250.0, 120.0).is_none(), "视口右侧之外不可命中");
    }

    /// DPI 缩放：物理像素先减工作区原点、再按比例换成 DIP（144 DPI、负原点工作区）。
    #[test]
    fn physical_points_scale_with_dpi()
    {
        let origin = PixelRect::new(-1920, 40, 0, 1120);
        let dpi = DpiScale::new(144);
        let scene = HitScene::new(vec![entry("a", 100.0, 100.0, 200.0, 150.0, HEADER, 0, &[])], None);
        assert_eq!(scene.hit_physical(-1770, 190, origin, dpi).unwrap().id, "a");
        assert_eq!(scene.hit_physical(-1769, 191, origin, dpi).unwrap().id, "a");
        assert!(scene.hit_physical(-1771, 190, origin, dpi).is_none());
    }

    /// F3：只有背景时，点击/Tab/方向键都不能把背景变成焦点。
    #[test]
    fn background_only_scene_never_focuses()
    {
        let background = clipped("workspace-background", Rect::new(0.0, 0.0, 800.0, 600.0), Rect::new(0.0, 0.0, 800.0, 600.0), TargetKind::Background, 1, i32::MIN, &[]);
        let scene = HitScene::new(vec![background], None);
        let hit = scene.hit(400.0, 300.0).expect("背景覆盖整块表面");
        assert_eq!(hit.kind, TargetKind::Background);
        assert!(scene.focus_candidates().is_empty());
        let mut focus = FocusState::default();
        focus.apply(FocusAction::Click, &scene, Some(&hit));
        assert_eq!(focus.focused_id(), None, "点背景只失焦");
        focus.apply(FocusAction::Next, &scene, None);
        assert_eq!(focus.focused_id(), None, "Tab 不选背景");
        for action in [FocusAction::Left, FocusAction::Right, FocusAction::Up, FocusAction::Down]
        {
            focus.apply(action, &scene, None);
            assert_eq!(focus.focused_id(), None, "方向键不选背景（{action:?}）");
        }
    }

    /// F3：重叠布局下 Tab 只在候选之间按阅读顺序环移，背景永远不入选。
    #[test]
    fn tab_skips_background_and_follows_reading_order()
    {
        let background = clipped("workspace-background", Rect::new(0.0, 0.0, 500.0, 500.0), Rect::new(0.0, 0.0, 500.0, 500.0), TargetKind::Background, 1, i32::MIN, &[]);
        let a = clipped("a", Rect::new(0.0, 0.0, 100.0, 100.0), Rect::new(0.0, 0.0, 500.0, 500.0), TargetKind::Fence, 50, 1000, &[]);
        let b = clipped("b", Rect::new(120.0, 0.0, 100.0, 100.0), Rect::new(0.0, 0.0, 500.0, 500.0), TargetKind::Component, 60, 0, &[]);
        let scene = HitScene::new(vec![background, a, b], None);
        assert_eq!(scene.reading_order(), vec!["a".to_string(), "b".to_string()]);
        let mut focus = FocusState::default();
        focus.apply(FocusAction::Next, &scene, None);
        assert_eq!(focus.focused_id(), Some("a"));
        focus.apply(FocusAction::Next, &scene, None);
        assert_eq!(focus.focused_id(), Some("b"));
        focus.apply(FocusAction::Next, &scene, None);
        assert_eq!(focus.focused_id(), Some("a"), "环移回到第一个");
        focus.apply(FocusAction::Previous, &scene, None);
        assert_eq!(focus.focused_id(), Some("b"));
    }

    /// F3：方向键按「方向投影重叠优先」选候选；稀疏布局里该方向没有格子就不动。
    #[test]
    fn arrows_prefer_projection_overlap_and_respect_sparse_layout()
    {
        let background = clipped("workspace-background", Rect::new(0.0, 0.0, 800.0, 600.0), Rect::new(0.0, 0.0, 800.0, 600.0), TargetKind::Background, 1, i32::MIN, &[]);
        // a 在左，右侧同一行有 b（投影重叠）、右下方有 c（更近但不同行）。
        let a = clipped("a", Rect::new(0.0, 0.0, 100.0, 100.0), Rect::new(0.0, 0.0, 800.0, 600.0), TargetKind::Fence, 50, 1000, &[]);
        let b = clipped("b", Rect::new(200.0, 10.0, 100.0, 100.0), Rect::new(0.0, 0.0, 800.0, 600.0), TargetKind::Fence, 50, 1000, &[]);
        let c = clipped("c", Rect::new(120.0, 300.0, 100.0, 100.0), Rect::new(0.0, 0.0, 800.0, 600.0), TargetKind::Component, 60, 0, &[]);
        let scene = HitScene::new(vec![background, a, b, c], None);
        let mut focus = FocusState::default();
        focus.set(Some("a".to_string()));
        focus.apply(FocusAction::Right, &scene, None);
        assert_eq!(focus.focused_id(), Some("b"), "同行候选优先于更近但错行的候选");
        focus.apply(FocusAction::Right, &scene, None);
        assert_eq!(focus.focused_id(), Some("b"), "右侧没有候选时保持不动");
        focus.apply(FocusAction::Left, &scene, None);
        assert_eq!(focus.focused_id(), Some("a"));
        // 无焦点时方向键取阅读顺序的第一个候选（不是背景）。
        let mut empty = FocusState::default();
        empty.apply(FocusAction::Down, &scene, None);
        assert_eq!(empty.focused_id(), Some("a"));
    }

    /// 焦点边框色解析：合法十六进制直接用，非法回落到强调色；报告文本与实际颜色同源。
    #[test]
    fn focus_color_parses_hex_and_reports_actual_value()
    {
        assert_eq!(focus_border_color("#ffb03a"), Color::rgba(0xFF, 0xB0, 0x3A, 0xFF));
        assert_eq!(focus_border_color("#FFB03A").to_hex_rgb(), "#FFB03A");
        assert_eq!(focus_border_color("not-a-color"), Color::rgba(0x25, 0x63, 0xEB, 0xFF));
        assert_eq!(focus_border_color("").to_hex_rgb(), "#2563EB");
    }

    /// 造一个只含围栏的快照；`projection` 给出 (源屏, 投影矩形) 用于生效矩形用例。
    fn snapshot_with(fences: &[(&str, f64, f64, Option<(&str, f64, f64)>)]) -> crate::contracts::WorkspaceSnapshot
    {
        let fences: Vec<serde_json::Value> = fences
            .iter()
            .enumerate()
            .map(|(index, (monitor, x, y, projection))| {
                let mut value = serde_json::json!({
                    "id": format!("f-{index}"), "title": "围栏", "monitorId": monitor,
                    "bounds": {"left": x, "top": y, "width": 200.0, "height": 160.0},
                    "members": [], "layoutMode": "tile", "scrollTop": 0, "collapsed": false,
                    "lastExpandedHeight": 160.0, "zIndex": 1
                });
                if let Some((target, px, py)) = projection
                {
                    value["projectionMonitorId"] = serde_json::json!(target);
                    value["projectionBounds"] = serde_json::json!({"left": px, "top": py, "width": 200.0, "height": 160.0});
                }
                value
            })
            .collect();
        serde_json::from_value(serde_json::json!({"schemaVersion": 1, "revision": 1, "fences": fences, "unclassifiedItems": []})).unwrap()
    }

    fn geometry_1080() -> HostGeometry
    {
        HostGeometry::for_work_area(PixelRect::new(0, 0, 1920, 1080), DpiScale::new(96))
    }

    fn plan(scene: &crate::contracts::WorkspaceSnapshot, runtime: &[&str]) -> Vec<ScreenPlan>
    {
        plan_with_primary(scene, runtime, runtime[0])
    }

    fn plan_with_primary(scene: &crate::contracts::WorkspaceSnapshot, runtime: &[&str], primary: &str) -> Vec<ScreenPlan>
    {
        let ids: Vec<String> = runtime.iter().map(|id| id.to_string()).collect();
        plan_topology(scene, &ids, primary, |_| geometry_1080())
    }

    /// 单屏 + 身份匹配：直接用，无需别名。
    #[test]
    fn topology_single_screen_matched()
    {
        let scene = snapshot_with(&[("\\\\.\\DISPLAY1", 10.0, 10.0, None)]);
        let plans = plan(&scene, &["\\\\.\\DISPLAY1"]);
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].source_id.as_deref(), Some("\\\\.\\DISPLAY1"));
        assert!(plans[0].verifiable);
        assert!(plans[0].note.is_none());
    }

    /// 单屏 + 唯一来源 + 生效矩形放得下：允许别名并写明映射。

    /// 单屏 + 多个来源屏：歧义，拒绝（不得按对象数量挑一屏）。

    /// 合法空屏（快照里没有任何持久对象）：拒绝，不用样例冒充真实布局。

    /// 单屏 + 唯一来源，但**生效矩形**（投影矩形）超出表面：拒绝。

    /// 两块运行屏都查不到自己的源、只有旧源 A：两屏都不可验，且不得把同一布局投两次。
    #[test]
    fn topology_two_unknown_screens_never_share_one_source()
    {
        let scene = snapshot_with(&[("\\\\.\\DISPLAY1", 0.0, 0.0, None)]);
        let plans = plan(&scene, &["\\\\.\\DISPLAY6", "\\\\.\\DISPLAY7"]);
        assert_eq!(plans.len(), 2);
        for screen in &plans
        {
            assert!(!screen.verifiable, "多屏活动拓扑下不做别名");
            assert!(screen.source_id.is_none(), "不匹配的屏不得拿到别的屏布局");
        }
        // 同一个来源屏不得出现两次。
        let mut used: Vec<&str> = plans.iter().filter_map(|screen| screen.source_id.as_deref()).collect();
        used.sort_unstable();
        let before = used.len();
        used.dedup();
        assert_eq!(before, used.len(), "来源屏不得重复映射");
    }

    /// A 仍活动、B 是合法空屏：B 不得取 A 的布局。
    #[test]
    fn topology_matched_screen_does_not_leak_to_empty_screen()
    {
        let scene = snapshot_with(&[("\\\\.\\DISPLAY1", 0.0, 0.0, None)]);
        let plans = plan(&scene, &["\\\\.\\DISPLAY1", "\\\\.\\DISPLAY2"]);
        assert_eq!(plans[0].source_id.as_deref(), Some("\\\\.\\DISPLAY1"));
        assert!(plans[0].verifiable);
        assert!(plans[1].source_id.is_none(), "合法空屏不得借用 A");
        assert!(!plans[1].verifiable);
    }

    /// F3（t35）：本机单屏直配 DISPLAY1 → DISPLAY1 用测试锁定，避免以后改动弄坏直配分支。
    #[test]
    fn topology_direct_match_is_locked()
    {
        let scene = snapshot_with(&[("\\\\.\\DISPLAY1", 0.0, 0.0, None)]);
        let plans = plan_with_primary(&scene, &["\\\\.\\DISPLAY1"], "\\\\.\\DISPLAY1");
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].source_id.as_deref(), Some("\\\\.\\DISPLAY1"), "身份相同必须直接配");
        assert!(plans[0].verifiable);
        assert_eq!(plans[0].effective_targets, 1, "过滤后目标仍在自己的屏上");
        assert!(plans[0].note.is_none());
    }

    /// F2（t35）：持久 A、投射 C、运行 C —— 前端投射过滤只会投到主屏，C 不能宣称可验。

    /// F1（t35）：枚举一致才进入判定；不一致时描述不同（判定在调用侧默认拒绝）。
    #[test]
    fn topology_enumeration_consistency()
    {
        let a = vec!["\\\\.\\DISPLAY1 @ 0,0,2560x1400 dpi96".to_string()];
        assert!(topology_consistent(&a, &a.clone()), "完全相同的两次枚举应一致");
        let changed_work = vec!["\\\\.\\DISPLAY1 @ 0,0,1920x1080 dpi96".to_string()];
        assert!(!topology_consistent(&a, &changed_work), "工作区变化必须判不一致");
        let changed_id = vec!["\\\\.\\DISPLAY6 @ 0,0,2560x1400 dpi96".to_string()];
        assert!(!topology_consistent(&a, &changed_id), "身份变化必须判不一致");
        let extra = vec!["\\\\.\\DISPLAY1 @ 0,0,2560x1400 dpi96".to_string(), "\\\\.\\DISPLAY2 @ 2560,0,2560x1400 dpi96".to_string()];
        assert!(!topology_consistent(&a, &extra), "屏数量变化必须判不一致");
        assert!(!topology_consistent(&a, &[]), "空拓扑必须判不一致");
    }

    /// 造一个只含一个组件的快照（用于「关闭的格子不产出目标」用例）。
    fn snapshot_with_component(monitor: &str, component_id: &str) -> crate::contracts::WorkspaceSnapshot
    {
        serde_json::from_value(serde_json::json!({
            "schemaVersion": 1, "revision": 1,
            "fences": [],
            "components": [{
                "instanceId": "c1", "componentId": component_id, "monitorId": monitor,
                "bounds": {"left": 10.0, "top": 10.0, "width": 360.0, "height": 280.0},
                "stateVersion": 1, "state": {}
            }],
            "unclassifiedItems": []
        })).unwrap()
    }

    /// F4-a：空拓扑不算一致成功；拓扑不一致或各屏目标为 0 一律不可验（正向导出，默认 false）。
    #[test]
    fn verifiable_is_derived_positively()
    {
        let ok_plan = vec![ScreenPlan { runtime_id: "C".into(), source_id: Some("C".into()), verifiable: true, effective_targets: 1, note: None }];
        let unmatched = vec![ScreenPlan { runtime_id: "D".into(), source_id: None, verifiable: false, effective_targets: 0, note: Some("多屏不别名".into()) }];
        assert!(!derive_verifiable(true, true, &[], &[]), "空拓扑 [] == [] 不算一致成功");
        assert!(!derive_verifiable(false, false, &ok_plan, &[1]), "拓扑不一致必须不可验");
        assert!(!derive_verifiable(true, false, &[], &[]), "无计划必须不可验");
        assert!(!derive_verifiable(true, false, &ok_plan, &[0]), "目标为 0（只剩背景）必须不可验");
        assert!(!derive_verifiable(true, false, &unmatched, &[0]), "有屏无可验来源必须不可验");
        assert!(derive_verifiable(true, false, &ok_plan, &[1]), "四条件齐备才可验");
    }

    /// F4-a 接线：宿主实际建窗是 C+D，而计划只认得 C —— 绘制前就应拒绝整轮，不是等遇到 D 再清。
    #[test]
    fn dual_host_topology_with_single_known_source_is_rejected_upfront()
    {
        let scene = snapshot_with(&[("\\\\.\\DISPLAY1", 0.0, 0.0, None)]);
        let plans = plan_with_primary(&scene, &["\\\\.\\DISPLAY1", "\\\\.\\DISPLAY6"], "\\\\.\\DISPLAY1");
        assert_eq!(plans.len(), 2);
        assert!(plans[0].verifiable, "已知屏本身有来源");
        assert!(!plans[1].verifiable, "未知屏在多屏拓扑下不别名");
        assert!(plans[1].source_id.is_none());
        // 聚合结论必须在绘制前由这两块屏共同导出：任一屏不可验 → 整轮不可验。
        assert!(!derive_verifiable(true, false, &plans, &[1, 0]));
    }

    /// 身份精确匹配是唯一绘制路径：匹配屏 source_id == runtime_id，且真有非背景目标。
    #[test]
    fn identity_match_is_the_only_draw_path()
    {
        let scene = snapshot_with(&[("\\\\.\\DISPLAY1", 20.0, 30.0, None)]);
        let identity = identity_scene(&scene);
        let active = ["\\\\.\\DISPLAY1".to_string()];
        let plans = plan_topology(&identity, &active, "\\\\.\\DISPLAY1", |_| geometry_1080());
        assert_eq!(plans[0].source_id.as_deref(), Some("\\\\.\\DISPLAY1"));
        assert!(plans[0].verifiable);
        assert_eq!(plans[0].effective_targets, 1);
        assert_eq!(screen_target_count(&identity, &ApplicationPreferences::default(), "\\\\.\\DISPLAY1", geometry_1080()), 1);
        assert!(derive_verifiable(true, false, &plans, &[1]));
    }

    /// 身份不匹配即拒绝：没有别名、没有投射，原因写明「别名未启用」。
    #[test]
    fn identity_mismatch_is_rejected_without_alias()
    {
        // 快照里对象在 A，运行屏是 C（A 已断开，也没有任何投射可用）。
        let scene = snapshot_with(&[("\\\\.\\DISPLAY1", 20.0, 30.0, None)]);
        let identity = identity_scene(&scene);
        let active = ["\\\\.\\DISPLAY6".to_string()];
        let plans = plan_topology(&identity, &active, "\\\\.\\DISPLAY6", |_| geometry_1080());
        assert!(plans[0].source_id.is_none(), "身份不匹配不得绘制");
        assert!(!plans[0].verifiable);
        assert_eq!(plans[0].effective_targets, 0);
        let note = plans[0].note.clone().unwrap_or_default();
        assert!(note.contains("别名未启用"), "原因要写明诊断期不提供别名：{note}");
        assert_eq!(screen_target_count(&identity, &ApplicationPreferences::default(), "\\\\.\\DISPLAY6", geometry_1080()), 0);
        assert!(!derive_verifiable(true, false, &plans, &[0]));
    }

    /// 身份场景会清空历史投射：别的屏的对象不会串到本屏，也不影响本屏计数。
    #[test]
    fn identity_scene_clears_projection_and_never_leaks()
    {
        let scene = snapshot_with(&[("\\\\.\\DISPLAY1", 10.0, 10.0, Some(("\\\\.\\DISPLAY6", 10.0, 10.0)))]);
        let identity = identity_scene(&scene);
        assert!(identity.fences[0].projection_monitor_id.is_none(), "投射字段必须清空");
        let prefs = ApplicationPreferences::default();
        assert_eq!(screen_target_count(&identity, &prefs, "\\\\.\\DISPLAY1", geometry_1080()), 1);
        assert_eq!(screen_target_count(&identity, &prefs, "\\\\.\\DISPLAY6", geometry_1080()), 0, "另一块屏不能拿到它");
    }

    /// F2：成员跨内容右缘但仍在格子外框内——跨出部分不可命中，未跨部分可命中。
    #[test]
    fn member_crossing_content_right_edge_hits_only_inside_content()
    {
        // 围栏外框 0..400，内容框只到 x=300；成员 250..350 跨过内容右缘。
        let fence_rect = Rect::new(0.0, 0.0, 400.0, 200.0);
        let viewport = Rect::new(0.0, 0.0, 800.0, 600.0);
        let content = Rect::new(0.0, 0.0, 300.0, 200.0);
        let clip = crate::render::workspace_scene::intersect(content, viewport).unwrap();
        let member = Rect::new(250.0, 40.0, 100.0, 40.0);
        let visible = crate::render::workspace_scene::intersect(member, clip).unwrap();
        assert_eq!((visible.x, visible.width), (250.0, 50.0), "可见部分只到内容右缘");
        let scene = HitScene::new(vec![clipped("fence", fence_rect, viewport, TargetKind::Fence, 50, 1000, &[("m", visible)])], None);
        // 正例：内容框内的部分命中成员。
        assert_eq!(scene.hit(260.0, 60.0).unwrap().region, HitRegion::Member("m".into()));
        assert_eq!(scene.hit(299.0, 60.0).unwrap().region, HitRegion::Member("m".into()));
        // 反例：跨出内容右缘、但仍在格子外框内的部分只命中格子内容区。
        let outside = scene.hit(320.0, 60.0).expect("仍在格子外框内");
        assert_eq!(outside.id, "fence");
        assert_eq!(outside.region, HitRegion::Content, "跨出内容右缘的部分不得命中成员");
        assert!(scene.hit(360.0, 60.0).is_some(), "成员之外的格子内容区仍可命中");
    }

    /// F5：脚本点必须一个在视口内（命中背景）、一个在视口外（无命中），标签与判定一致。
    #[test]
    fn plan_generates_in_viewport_background_and_true_outside_point()
    {
        let viewport = Rect::new(0.0, 0.0, 800.0, 600.0);
        let background = clipped("workspace-background", viewport, viewport, TargetKind::Background, 1, i32::MIN, &[]);
        let fence = clipped("fence", Rect::new(0.0, 0.0, 300.0, 200.0), viewport, TargetKind::Fence, 50, 1000, &[]);
        let entries = vec![background, fence];
        let plan = plan_for(&entries, 800.0, 600.0);
        let scene = HitScene::new(entries, None);
        let blank = plan.iter().find(|step| step.label.contains("屏内空白")).and_then(|step| step.point).expect("必须生成屏内空白点");
        assert!(blank.0 > 0.0 && blank.1 > 0.0 && blank.0 < 800.0 && blank.1 < 600.0, "空白点必须在视口内：{blank:?}");
        assert_eq!(scene.hit(blank.0, blank.1).map(|hit| hit.kind), Some(TargetKind::Background), "屏内空白点应命中背景");
        let outside = plan.iter().find(|step| step.label.contains("工作区外")).and_then(|step| step.point).expect("必须生成越界点");
        assert!(outside.0 < 0.0 || outside.1 < 0.0 || outside.0 >= 800.0 || outside.1 >= 600.0, "越界点必须在视口外：{outside:?}");
        assert!(scene.hit(outside.0, outside.1).is_none(), "越界点不应命中任何东西");
    }
}

/// 真实快照为空时使用的样例布局（只用于演示同一条命中/焦点路径，不写入任何文件）。
fn sample_scene(monitor_id: &str) -> crate::contracts::WorkspaceSnapshot
{
    let fence = |id: &str, x: f64, y: f64, w: f64, h: f64, z: u32, members: Vec<&str>, title: &str| {
        serde_json::json!({
            "id": id, "title": title, "monitorId": monitor_id,
            "bounds": {"left": x, "top": y, "width": w, "height": h},
            "members": members, "layoutMode": "tile", "scrollTop": 0,
            "collapsed": false, "lastExpandedHeight": h, "zIndex": z
        })
    };
    serde_json::from_value(serde_json::json!({
        "schemaVersion": 1,
        "revision": 0,
        "fences": [
            fence("sample-fence-a", 60.0, 60.0, 360.0, 280.0, 1, vec!["a1", "a2", "a3", "a4"], "样例围栏 A"),
            fence("sample-fence-b", 460.0, 60.0, 240.0, 280.0, 2, vec!["b1", "b2"], "样例围栏 B"),
            fence("sample-fence-c", 60.0, 380.0, 360.0, 200.0, 3, vec![], "样例围栏 C")
        ],
        "unclassifiedItems": [
            {"stableReference": "sample-item-1", "monitorId": monitor_id, "position": {"left": 500.0, "top": 380.0}, "zIndex": 0},
            {"stableReference": "sample-item-2", "monitorId": monitor_id, "position": {"left": 620.0, "top": 380.0}, "zIndex": 0}
        ]
    })).expect("内置样例必须可反序列化")
}

/// 诊断脚本的一步。
#[derive(Clone)]
struct Step
{
    action: FocusAction,
    /// 光标点（工作区局部 DIP）；键盘操作为 None。
    point: Option<(f32, f32)>,
    label: &'static str,
}

/// 一步的可核对日志。
#[derive(Clone, Debug, serde::Serialize)]
pub struct StepLog
{
    pub step: usize,
    pub monitor: String,
    pub action: String,
    pub point: Option<String>,
    pub hit: String,
    pub region: String,
    pub focus_before: Option<String>,
    pub focus_after: Option<String>,
    pub focus_changed: bool,
}

/// 命中与焦点诊断的完整结果。
#[derive(Clone, Debug, serde::Serialize)]
pub struct InputReport
{
    pub demo: super::host::DemoReport,
    /// 命中场景来自已保存快照还是内置样例（真实快照为空时用样例，日志里会注明）。
    pub source: String,
    /// 本轮结果能否当作「该运行屏真实布局」的证据（别名被拒时为 false）。
    pub verifiable: bool,
    /// 全局拓扑决策：每块活动屏用哪个来源屏、是否可验、原因。
    pub screens: Vec<ScreenPlan>,
    /// 枚举一（DPI 感知设置之前）的拓扑描述。
    pub topology_before: Vec<String>,
    /// 枚举二（DPI 感知生效后）的拓扑描述。
    pub topology_after: Vec<String>,
    /// 两次枚举是否一致；不一致时全部判定不可验、不绘制。
    pub topology_consistent: bool,
    /// 实际用于焦点边框的颜色（与绘制同一个值）。
    pub focus_border: String,
    pub notes: Vec<String>,
    pub fingerprint_before: Fingerprint,
    pub fingerprint_after: Fingerprint,
    pub fingerprint_unchanged: bool,
    pub steps: Vec<StepLog>,
    pub unverified: Vec<String>,
}

/// 布局/偏好指纹（只读散列）。
#[derive(Clone, Debug, serde::Serialize)]
pub struct Fingerprint
{
    pub layout: String,
    pub settings: String,
}

/// 一块活动屏的诊断决策：用哪个快照屏做来源、结果是否可验、原因。
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct ScreenPlan
{
    pub runtime_id: String,
    /// `None` 表示这块屏不绘制（身份不匹配且不允许别名）。
    pub source_id: Option<String>,
    /// 本轮该屏的结果能否当作「真实布局」的证据。
    pub verifiable: bool,
    /// 按前端投射过滤后，真正会落在这块屏上的对象数（0 表示这屏实际是空的）。
    pub effective_targets: usize,
    pub note: Option<String>,
}

/// 把一次枚举压成可比的描述（身份 + 工作区 + DPI），用于拓扑与几何变化比对。
pub fn describe_topology(monitors: &[crate::contracts::DesktopHostMonitor]) -> Vec<String>
{
    let mut described: Vec<String> = monitors
        .iter()
        .map(|monitor| format!("{} @ {},{},{}x{} dpi{}", monitor.id, monitor.work_x, monitor.work_y, monitor.work_width, monitor.work_height, monitor.dpi))
        .collect();
    described.sort();
    described
}

/// 两次拓扑描述是否一致（集合与每项都相同）。
pub fn topology_consistent(before: &[String], after: &[String]) -> bool
{
    before == after
}

/// 该屏自己持久持有的对象数（只算围栏/组件/门户，保持原有筛选口径）。
fn owns_count(s: &crate::contracts::WorkspaceSnapshot, monitor: &str) -> usize
{
    s.fences.iter().filter(|f| f.monitor_id == monitor).count() + s.components.iter().filter(|c| c.monitor_id == monitor).count() + s.folder_portals.iter().filter(|p| p.monitor_id == monitor).count()
}

/// 全局拓扑决策：**只认「运行屏身份 == 持久屏身份」这一条路径**。诊断期不提供跨屏别名。
///
/// 身份匹配 → 用该屏自己的持久对象绘制（`source_id == runtime_id`）；
/// 不匹配 → `source_id = None`、`verifiable = false`、不绘制，原因写明「别名未启用」。
/// 因此来源键与绘制键恒等，不存在「按 A 计数、按 C 筛选」这类分歧。
pub fn plan_topology(s: &crate::contracts::WorkspaceSnapshot, runtime_ids: &[String], primary_id: &str, geometry_of: impl Fn(&str) -> crate::render::geometry::HostGeometry) -> Vec<ScreenPlan>
{
    let _ = primary_id;
    let mut plans: Vec<ScreenPlan> = Vec::new();
    for runtime_id in runtime_ids
    {
        let targets = owns_count(s, runtime_id);
        if targets > 0
        {
            // 适配仍要看这块宿主自己的几何：矩形放不进本屏表面就判不可验。
            let geometry = geometry_of(runtime_id);
            let (width, height) = geometry.dip_size();
            let fits = |rect: &crate::contracts::WorkspaceRect| rect.left >= 0. && rect.top >= 0. && rect.left + rect.width <= f64::from(width) + 0.5 && rect.top + rect.height <= f64::from(height) + 0.5;
            let all_fit = s.fences.iter().filter(|f| f.monitor_id == *runtime_id).all(|f| fits(&f.bounds))
                && s.components.iter().filter(|c| c.monitor_id == *runtime_id).all(|c| fits(&c.bounds))
                && s.folder_portals.iter().filter(|p| p.monitor_id == *runtime_id).all(|p| fits(&p.bounds));
            if !all_fit
            {
                plans.push(ScreenPlan { runtime_id: runtime_id.clone(), source_id: None, verifiable: false, effective_targets: 0, note: Some(format!("运行屏 {runtime_id} 的持久对象放不进本屏 {width}x{height} DIP 表面，本轮不可验")) });
                continue;
            }
            plans.push(ScreenPlan { runtime_id: runtime_id.clone(), source_id: Some(runtime_id.clone()), verifiable: true, effective_targets: targets, note: None });
        }
        else
        {
            plans.push(ScreenPlan { runtime_id: runtime_id.clone(), source_id: None, verifiable: false, effective_targets: 0, note: Some(format!("屏身份不匹配，别名未启用：运行屏 {runtime_id} 在快照里没有持久对象（诊断期不提供跨屏别名，不绘制、不可验）")) });
        }
    }
    plans
}

/// 从一屏的命中条目录出诊断脚本：所有点都从真实矩形与视口推出，不写死坐标。
fn plan_for(entries: &[HitEntry], width: f32, height: f32) -> Vec<Step>
{
    let sorted: Vec<&HitEntry> = {
        let mut list: Vec<&HitEntry> = entries.iter().filter(|entry| entry.kind != TargetKind::Background).collect();
        list.sort_by(|a, b| a.rect.y.total_cmp(&b.rect.y).then(a.rect.x.total_cmp(&b.rect.x)));
        list
    };
    let center = |entry: &HitEntry| (entry.rect.x + entry.rect.width / 2.0, entry.rect.y + entry.rect.height / 2.0);
    let header_point = |entry: &HitEntry| (entry.rect.x + 8.0, entry.rect.y + entry.header.max(1.0) / 2.0);
    let mut steps: Vec<Step> = Vec::new();
    if let Some(first) = sorted.first()
    {
        steps.push(Step { action: FocusAction::Click, point: Some(center(first)), label: "点击第一个格子中心" });
        if first.header > 0.0
        {
            steps.push(Step { action: FocusAction::Click, point: Some(header_point(first)), label: "点击同一格子标题栏" });
        }
        if let Some((_, rect)) = first.members.first()
        {
            steps.push(Step { action: FocusAction::Click, point: Some((rect.x + rect.width / 2.0, rect.y + rect.height / 2.0)), label: "点击内容区第一个成员" });
        }
        // 屏内空白：在视口内逐点找一个不属于任何格子的位置，命中结果必须是 workspace-background。
        let mut blank: Option<(f32, f32)> = None;
        for step in 0..64
        {
            let x = 12.0 + (step % 8) as f32 * ((width - 24.0).max(1.0) / 8.0);
            let y = 12.0 + (step / 8) as f32 * ((height - 24.0).max(1.0) / 8.0);
            if x <= 0.0 || y <= 0.0 || x >= width || y >= height
            {
                continue;
            }
            if !entries.iter().any(|entry| entry.kind != TargetKind::Background && entry.visible().is_some_and(|visible| visible.contains(x, y)))
            {
                blank = Some((x, y));
                break;
            }
        }
        steps.push(Step { action: FocusAction::Click, point: blank, label: "点屏内空白（应命中背景并失焦）" });
        if let Some(second) = sorted.get(1).or_else(|| sorted.first())
        {
            steps.push(Step { action: FocusAction::Click, point: Some(center(second)), label: "点击第二个格子中心" });
        }
        steps.push(Step { action: FocusAction::Next, point: None, label: "Tab 到下一个" });
        steps.push(Step { action: FocusAction::Next, point: None, label: "Tab 到下一个" });
        steps.push(Step { action: FocusAction::Previous, point: None, label: "Shift+Tab 回上一个" });
        steps.push(Step { action: FocusAction::Right, point: None, label: "方向键右" });
        steps.push(Step { action: FocusAction::Down, point: None, label: "方向键下" });
        steps.push(Step { action: FocusAction::Click, point: Some((-40.0, -40.0)), label: "点工作区外（负坐标，应无命中）" });
        // 最后停在第一个围栏上：保持态能直接看到焦点边框。
        if let Some(fence) = sorted.iter().find(|entry| entry.kind == TargetKind::Fence)
        {
            steps.push(Step { action: FocusAction::Click, point: Some(center(fence)), label: "点击围栏（保持焦点态）" });
            steps.push(Step { action: FocusAction::Click, point: Some(center(fence)), label: "保持焦点态" });
        }
    }
    steps
}

/// 身份场景：清空全部投射字段，保证「一块屏只画它自己的持久对象」。
///
/// 诊断期不投射（正式协调会把断开屏的内容投到主屏，那是正式行为，不在本诊断覆盖范围）；
/// 这样来源键与绘制键恒等，不会再出现「计数按 A、筛选按 C」这类分歧。
pub fn identity_scene(s: &crate::contracts::WorkspaceSnapshot) -> crate::contracts::WorkspaceSnapshot
{
    let mut scene = s.clone();
    for fence in &mut scene.fences
    {
        fence.projection_monitor_id = None;
        fence.projection_bounds = None;
    }
    for component in &mut scene.components
    {
        component.projection_monitor_id = None;
        component.projection_bounds = None;
    }
    for item in &mut scene.unclassified_items
    {
        item.projection_monitor_id = None;
        item.projection_position = None;
    }
    scene
}

/// 这一屏在有效场景里到底有多少个**非背景**可命中目标（= 真会画出且能点到的图元数）。
///
/// 判定、绘制、命中都用同一份有效场景，所以这里数出来的就是屏幕上的实际内容；
/// 返回 0 表示「过滤后只剩背景」，该屏不可验、不绘制。
pub fn screen_target_count(scene: &crate::contracts::WorkspaceSnapshot, prefs: &crate::contracts::settings::ApplicationPreferences, monitor_id: &str, geometry: crate::render::geometry::HostGeometry) -> usize
{
    crate::render::workspace_scene::scene_hits(scene, prefs, monitor_id, geometry, None).iter().filter(|entry| entry.kind != super::input::TargetKind::Background).count()
}

/// 可验结论只由「确认状态」正向导出：默认 false，只有全部条件成立才为 true。
pub fn derive_verifiable(topology_consistent: bool, topology_empty: bool, plans: &[ScreenPlan], targets: &[usize]) -> bool
{
    if !topology_consistent || topology_empty || plans.is_empty() || targets.len() != plans.len()
    {
        return false;
    }
    plans.iter().all(|plan| plan.source_id.is_some() && plan.verifiable) && targets.iter().all(|count| *count > 0)
}

/// 跑一次命中与焦点诊断：真实快照 + 真实表面，脚本驱动，日志逐条可核对。
///
/// 拓扑只有一个事实来源：宿主在 DPI 生效后建窗用的那份监视器列表（由 `run_scene_steps` 逐屏传入）。
/// 诊断期**只画身份精确匹配的屏**（不提供跨屏别名、不做投射）；任何不确定都失败关闭。
pub fn run_input() -> Result<InputReport>
{
    use crate::render::workspace_scene::{draw_with_focus, focus_border_hex, layout_fingerprint, read_scene, scene_hits};
    let (snapshot, prefs) = read_scene()?;
    let has_items = |s: &crate::contracts::WorkspaceSnapshot| !s.fences.is_empty() || !s.components.is_empty() || !s.folder_portals.is_empty() || !s.unclassified_items.is_empty();
    let mut notes: Vec<String> = Vec::new();
    let source;
    let sample;
    let scene = if has_items(&snapshot)
    {
        source = "snapshot".to_string();
        &snapshot
    }
    else
    {
        source = "sample".to_string();
        notes.push("已保存的工作区快照里没有任何格子，改用内置样例（不写盘，仅内存）演示同一条命中/焦点路径".to_string());
        sample = sample_scene("__sample__");
        &sample
    };
    let fingerprint_before = {
        let (layout, settings) = layout_fingerprint()?;
        Fingerprint { layout, settings }
    };

    // 宿主实际建窗的拓扑与每块屏自己的几何（唯一事实）；step 0 只记录、不绘制。
    let observed: std::cell::RefCell<std::collections::BTreeMap<String, String>> = std::cell::RefCell::new(std::collections::BTreeMap::new());
    let geometry_map: std::cell::RefCell<std::collections::HashMap<String, crate::render::geometry::HostGeometry>> = std::cell::RefCell::new(std::collections::HashMap::new());
    let confirmed_cell: std::cell::RefCell<Option<Confirmed>> = std::cell::RefCell::new(None);
    let topology_at_confirm: std::cell::RefCell<Vec<String>> = std::cell::RefCell::new(Vec::new());
    let frame_failed = std::cell::Cell::new(false);
    let geometry_changed = std::cell::Cell::new(false);
    let notes_cell = std::cell::RefCell::new(Vec::<String>::new());
    let mut focus = FocusState::default();
    let mut logs: Vec<StepLog> = Vec::new();
    let mut plans: std::collections::HashMap<String, Vec<Step>> = std::collections::HashMap::new();
    let step_count = 12usize; // 10 个交互步骤 + 围栏焦点 + 保持一帧用于取证

    let demo = super::host::run_scene_steps(
        |step, geometry, monitor, index, _family| {
            let monitor_id = monitor.id.clone();
            // 记录这块宿主自己的几何与身份；后续任何一帧的几何变化都判不可验。
            {
                let description = describe_topology(std::slice::from_ref(monitor)).pop().unwrap_or_default();
                let mut observed = observed.borrow_mut();
                match observed.get(&monitor_id)
                {
                    Some(previous) if *previous != description => geometry_changed.set(true),
                    _ => {}
                }
                observed.insert(monitor_id.clone(), description);
                let mut geometries = geometry_map.borrow_mut();
                match geometries.get(&monitor_id)
                {
                    Some(previous) if *previous != geometry => geometry_changed.set(true),
                    _ => {}
                }
                geometries.insert(monitor_id.clone(), geometry);
            }
            if step == 0
            {
                return DrawList::new();
            }

            // 第一帧之后：用宿主实际建窗拓扑确认一次；失败/为空 → 永久不可验、空绘制。
            if confirmed_cell.borrow().is_none()
            {
                let topology: Vec<String> = {
                    let observed = observed.borrow();
                    let mut values: Vec<String> = observed.values().cloned().collect();
                    values.sort();
                    values
                };
                let ids: Vec<String> = { geometry_map.borrow().keys().cloned().collect() };
                if topology.is_empty() || ids.is_empty()
                {
                    notes_cell.borrow_mut().push("宿主未提供任何建窗拓扑（零屏或无帧），拓扑确认失败：不可验、不绘制".to_string());
                    *confirmed_cell.borrow_mut() = Some(Confirmed { effective: scene.clone(), plans: Vec::new(), verifiable: false, topology: topology.clone() });
                    *topology_at_confirm.borrow_mut() = topology;
                }
                else
                {
                    // 身份精确匹配的唯一场景：清空投射，一块屏只画自己的对象。
                    let mut identity = identity_scene(scene);
                    if source == "sample"
                    {
                        let primary = ids.first().cloned().unwrap_or_default();
                        for fence in &mut identity.fences
                        {
                            fence.monitor_id = primary.clone();
                        }
                        for item in &mut identity.unclassified_items
                        {
                            item.monitor_id = primary.clone();
                        }
                    }
                    let geometries = geometry_map.borrow();
                    let plans_raw = plan_topology(&identity, &ids, "", |id| *geometries.get(id).unwrap_or(&geometry));
                    drop(geometries);
                    let targets: Vec<usize> = plans_raw.iter().map(|plan| match &plan.source_id
                    {
                        Some(_) => screen_target_count(&identity, &prefs, &plan.runtime_id, *geometry_map.borrow().get(&plan.runtime_id).unwrap_or(&geometry)),
                        None => 0,
                    }).collect();
                    let consistent = topology.len() == ids.len();
                    let verifiable = derive_verifiable(consistent, topology.is_empty(), &plans_raw, &targets) && source != "sample";
                    if !verifiable
                    {
                        notes_cell.borrow_mut().push(format!("身份匹配确认失败（拓扑 {:?}、各屏目标数 {:?}、来源 {:?}）：不可验、不绘制", topology, targets, plans_raw));
                    }
                    *topology_at_confirm.borrow_mut() = topology.clone();
                    *confirmed_cell.borrow_mut() = Some(Confirmed { effective: identity, plans: plans_raw, verifiable, topology });
                }
            }
            let confirmed_ref = confirmed_cell.borrow();
            let Some(confirmed_now) = confirmed_ref.as_ref() else { return DrawList::new() };
            if !confirmed_now.verifiable() || geometry_changed.get()
            {
                return DrawList::new();
            }
            // 后置自证：先构建最终绘制列表，再由它决定可验；空帧把最终结论置 false。
            let effective = &confirmed_now.effective;
            let Some(screen) = confirmed_now.plans.iter().find(|plan| plan.runtime_id == monitor_id).cloned() else { return DrawList::new() };
            let Some(_source_id) = screen.source_id.clone() else
            {
                if let Some(note) = &screen.note
                {
                    let mut notes = notes_cell.borrow_mut();
                    if !notes.contains(note)
                    {
                        notes.push(note.clone());
                    }
                }
                return DrawList::new();
            };
            // 绘制键恒等：按运行屏 id 取条目（身份场景里没有投射，不会串屏）。
            let entries = scene_hits(effective, &prefs, &monitor_id, geometry, focus.focused_id());
            if entries.iter().filter(|entry| entry.kind != TargetKind::Background).count() == 0
            {
                frame_failed.set(true);
                return DrawList::new();
            }
            let (dip_width, dip_height) = geometry.dip_size();
            let plan = plans.entry(monitor_id.clone()).or_insert_with(|| plan_for(&entries, dip_width, dip_height)).clone();
            if index == 0
            {
                let current = HitScene::new(entries, focus.focused_id());
                let step_now = plan.get(step % plan.len().max(1));
                let (action, point, label) = match step_now
                {
                    Some(step) => (step.action, step.point, step.label),
                    None => (FocusAction::Click, None, "无脚本"),
                };
                let hit = point.and_then(|(x, y)| current.hit(x, y));
                let before = focus.focused_id().map(str::to_string);
                let change = focus.apply(action, &current, hit.as_ref());
                logs.push(StepLog {
                    step,
                    monitor: monitor_id.clone(),
                    action: format!("{}（{label}）", action.as_str()),
                    point: point.map(|(x, y)| format!("({x:.1},{y:.1})")),
                    hit: hit.as_ref().map(|hit| format!("{}:{}", hit.kind.as_str(), hit.id)).unwrap_or_else(|| "无".to_string()),
                    region: hit.as_ref().map(|hit| hit.region.as_str()).unwrap_or_else(|| "-".to_string()),
                    focus_before: before,
                    focus_after: focus.focused_id().map(str::to_string),
                    focus_changed: change.is_some(),
                });
            }
            draw_with_focus(effective, &prefs, &monitor_id, geometry, focus.focused_id())
        },
        step_count,
        std::time::Duration::from_millis(std::env::var("DESKTOP_NATIVE_INPUT_HOLD_MS").ok().and_then(|value| value.parse().ok()).unwrap_or(320)),
    )?;

    let confirmed = confirmed_cell.into_inner();
    let topology_at_confirm = topology_at_confirm.into_inner();
    let topology_final: Vec<String> = {
        let observed = observed.borrow();
        let mut values: Vec<String> = observed.values().cloned().collect();
        values.sort();
        values
    };
    let consistent = !topology_final.is_empty() && topology_at_confirm == topology_final;
    let mut notes = notes;
    notes.extend(notes_cell.into_inner());
    if geometry_changed.get()
    {
        notes.push("运行中观察到 DPI/工作区变化：诊断只认证启动时冻结的拓扑，本轮判不可验".to_string());
    }
    if frame_failed.get()
    {
        notes.push("某一帧过滤后只剩背景：后置自证失败，最终结论判不可验".to_string());
    }
    let fingerprint_after = {
        let (layout, settings) = layout_fingerprint()?;
        Fingerprint { layout, settings }
    };
    let fingerprint_unchanged = fingerprint_before.layout == fingerprint_after.layout && fingerprint_before.settings == fingerprint_after.settings;
    let screens = confirmed.as_ref().map(|value| value.plans.clone()).unwrap_or_default();
    let verifiable = confirmed.as_ref().map(|value| value.verifiable()).unwrap_or(false) && consistent && !geometry_changed.get() && !frame_failed.get();
    let _ = confirmed;
    Ok(InputReport {
        demo,
        source,
        verifiable,
        screens,
        topology_before: topology_at_confirm.clone(),
        topology_after: topology_final,
        topology_consistent: consistent,
        focus_border: focus_border_hex(&prefs),
        notes,
        fingerprint_before,
        fingerprint_after,
        fingerprint_unchanged,
        steps: logs,
        unverified: vec![
            "真实输入通路未接入：宿主窗口不接收鼠标/键盘（诊断用脚本驱动同一套命中与焦点函数）".to_string(),
            "IME、触控与辅助技术未验证".to_string(),
            "混合 DPI 下的方向键就近选择只在单屏验证".to_string(),
            "方向键采用壳级「投影重叠优先」，与正式工作区的「当前容器优先」不完全等价".to_string(),
            "标题栏按钮尚未绘制，因此没有按钮级命中区".to_string(),
            "诊断期不提供跨屏别名、也不做投射：屏身份不匹配即不可验、不绘制；多屏身份稳定性留给后续独立任务".to_string(),
        ],
    })
}

/// 确认后的状态：身份场景 + 每屏计划 + 可验结论（默认 false，正向导出）。
struct Confirmed
{
    effective: crate::contracts::WorkspaceSnapshot,
    plans: Vec<ScreenPlan>,
    verifiable: bool,
    topology: Vec<String>,
}

impl Confirmed
{
    fn verifiable(&self) -> bool
    {
        self.verifiable && !self.topology.is_empty()
    }
}
