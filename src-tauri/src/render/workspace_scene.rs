//! 诊断专用的只读布局投影。只恢复并复制现有快照，不执行布局事务、目录迁移或插件内容。
use super::{
    draw::{Color, DrawCommand, DrawList, Rect, TextAlign},
    geometry::HostGeometry,
    host::{DemoReport, run_scene},
};
use crate::{
    contracts::{
        FenceLayoutMode, TileAppearance, TileTitleMode, WorkspaceRect, WorkspaceSnapshot,
        settings::{ApplicationPreferences, ApplicationTheme},
    },
    persistence::load_json,
    workspace::WorkspaceState,
};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use windows::{
    Win32::Foundation::E_FAIL,
    core::{Error, Result},
};

const HEADER: f32 = 28.;
fn fail(e: impl std::fmt::Display) -> Error { Error::new(E_FAIL, e.to_string()) }
/// 与正式入口读取同一个已生效位置；不使用可能迁移目录的 prepare_config_directory。
pub(super) fn read_scene() -> Result<(WorkspaceSnapshot, ApplicationPreferences)>
{
    let dir = config_dir()?;
    read_scene_from(&dir)
}

pub(super) fn config_dir() -> Result<PathBuf>
{
    let bootstrap = PathBuf::from(std::env::var_os("APPDATA").ok_or_else(|| fail("APPDATA 不可用"))?).join("io.github.tiamo1208.desktop-tools");
    let dir: PathBuf = load_json(&bootstrap.join("config-location.json")).map_err(fail)?.unwrap_or(bootstrap);
    Ok(dir)
}
pub(super) fn read_scene_from(dir: &Path) -> Result<(WorkspaceSnapshot, ApplicationPreferences)>
{
    let path = dir.join("workspace-layout.json");
    if !path.is_file()
    {
        return Err(fail("未找到已保存的工作区，不绘制虚构样例"));
    }
    let scene = WorkspaceState::load_with_components(path, &[], &[]).and_then(|s| s.snapshot()).map_err(|e| fail(e.message))?;
    let settings: ApplicationPreferences = load_json(&dir.join("settings.json")).map_err(fail)?.unwrap_or_default();
    settings.validate().map_err(fail)?;
    Ok((scene, settings))
}

/// 限时诊断与常驻原生宿主使用同一只读快照投影。
pub fn run_workspace() -> Result<DemoReport>
{
    let (scene, settings) = read_scene()?;
    run_scene(Duration::from_secs(8), |geometry, monitor, _, _| draw_snapshot(&scene, &settings, &monitor.id, geometry))
}

/// 源矩形是工作区局部 DIP，不加全局显示器偏移；超宽高先缩到工作区，再将边缘移回。
fn local_rect(bounds: WorkspaceRect, geometry: HostGeometry, collapsed: Option<f32>) -> Rect
{
    let (w, h) = geometry.dip_size();
    let width = (bounds.width as f32).clamp(0., w);
    let height = collapsed.unwrap_or(bounds.height as f32).clamp(0., h);
    Rect::new((bounds.left as f32).clamp(0., w - width), (bounds.top as f32).clamp(0., h - height), width, height)
}

/// 同一位置上的多个格子按持久层级绘制。组件无持久层级，使用快照顺序。
/// 组件标题与可见性：绘制与命中共用一份规则，避免两处漂移。
fn component_view(p: &ApplicationPreferences, component_id: &str) -> Option<(String, bool)>
{
    Some(match component_id
    {
        "official.local-clock" => ("本地时间".to_string(), p.clock_enabled),
        "official.music-control" => ("音乐控制".to_string(), p.music_enabled),
        "official.weather" => ("天气".to_string(), p.weather_enabled),
        "official.collect-box" => ("收集箱".to_string(), p.collect_enabled),
        "official.debug-test" => ("测试".to_string(), cfg!(debug_assertions) && p.debug_enabled),
        other => (other.to_string(), !p.disabled_plugins.iter().any(|v| v == other)),
    })
}

/// 外壳矩形与标题轨道高度：绘制与命中都用它，保证两边算的是同一个盒子。
fn shell_rects(bounds: WorkspaceRect, g: HostGeometry, appearance: Option<&TileAppearance>, collapsed: bool, closed_h: f32) -> (Rect, f32)
{
    let r = local_rect(bounds, g, collapsed.then_some(closed_h));
    let header = if collapsed || !appearance.is_some_and(|a| a.title_mode == TileTitleMode::Hover) { HEADER.min(r.height) } else { 0. };
    (r, header)
}

pub(super) fn draw_snapshot(s: &WorkspaceSnapshot, p: &ApplicationPreferences, id: &str, g: HostGeometry) -> DrawList { draw_with_content(s, p, id, g, None) }
/// 绘制与命中共用的块：`order` 只在 [`scene_blocks`] 里算一次，绘制顺序与命中优先级都来自它。
pub(super) struct Block
{
    pub id: String,
    pub kind: super::input::TargetKind,
    pub rect: Rect,                          // 绘制矩形（未裁剪）。
    pub header: f32,                         // 标题轨道高度，0 表示没有标题轨道。
    pub members: Vec<(String, Rect)>,        // 成员绘制矩形（未裁剪）。
    pub content_clip: Option<Rect>,          // 围栏内容框；命中用它裁成员。
    pub priority: i32,                       // 与前端 `data-hit-priority` 一致的优先级。
    pub z: i32,                              // 与前端计算的 z-index 一致的层级。
    pub list: DrawList,                      // 该块的绘制指令。
}

/// 与 `.desktop-tile` 的 `FENCE_Z_INDEX_BASE` 一致的层级基数（围栏与门户都用它）。
const TILE_Z_BASE: i32 = 1000;

/// 生成该显示器上的全部块，并**按绘制顺序**返回（层级低 → 高）。
///
/// 唯一排序点：`(z, priority)` 稳定升序。绘制按这个 Vec 顺序拼指令，命中按它的**逆序**查找，
/// 所以「谁画在上面」与「谁先被点到」永远一致（同层门户在组件之后画 ⇒ 门户先命中）。
/// 焦点块整体移到末尾：内容与边框一起上移，命中也随之最先命中焦点块。
pub(super) fn scene_blocks(s: &WorkspaceSnapshot, p: &ApplicationPreferences, id: &str, g: HostGeometry, content: Option<&super::content::Content>, focus: Option<&str>) -> Vec<Block>
{
    use super::input::TargetKind;
    let (fill, edge, text) = if p.desktop_appearance.theme == ApplicationTheme::Light
    {
        (Color(0xb9b9b994), Color(0x37424538), Color(0x202526ff))
    }
    else
    {
        (Color(0x20262894), Color(0xd2e0e361), Color(0xe8eff0ff))
    };
    let radius = p.corner_radius.min(8) as f32;
    let mut blocks: Vec<Block> = Vec::new();
    let shell = |bounds: WorkspaceRect, title: &str, appearance: Option<&TileAppearance>, collapsed: bool, closed_h: f32| -> (DrawList, Rect, f32) {
        let (r, header) = shell_rects(bounds, g, appearance, collapsed, closed_h);
        let mut l = DrawList::new();
        l.push(DrawCommand::FillRoundRect { rect: r, radius, color: fill });
        let inner = Rect::new(r.x + 0.5, r.y + 0.5, (r.width - 1.).max(0.), (r.height - 1.).max(0.));
        l.push(DrawCommand::StrokeRoundRect { rect: inner, radius: (radius - 0.5).max(0.), width: 1., color: edge });
        if header > 0.
        {
            let size = p.font_size as f32;
            let y = (header - size * 1.5).max(0.) / 2.;
            l.push(DrawCommand::Text { rect: Rect::new(r.x + 6., r.y + y, (r.width - 12.).max(0.), header - y), text: appearance.and_then(|a| a.title.as_deref()).unwrap_or(title).into(), size, weight: 600, color: text, align: TextAlign::Start, max_lines: 1 });
        }
        (l, r, header)
    };
    let mark = |rect| DrawCommand::StrokeRoundRect { rect, radius: 0., width: 1., color: edge };
    for item in &s.unclassified_items
    {
        if item.projection_monitor_id.as_deref().unwrap_or(&item.monitor_id) != id
        {
            continue;
        }
        let pt = item.projection_monitor_id.as_ref().and(item.projection_position).unwrap_or(item.position);
        let rect = local_rect(WorkspaceRect { left: pt.left, top: pt.top, width: p.desktop_grid.width.into(), height: p.desktop_grid.height.into() }, g, None);
        let mut l = DrawList::new();
        l.push(mark(rect));
        // 未归类桌面项在前端是 `desktop-item`，优先级 70（比围栏/门户/组件本体都高）。
        blocks.push(Block { id: item.stable_reference.clone(), kind: TargetKind::Item, rect, header: 0., members: Vec::new(), content_clip: None, priority: 70, z: item.z_index as i32, list: l });
    }
    for f in &s.fences
    {
        if f.projection_monitor_id.as_deref().unwrap_or(&f.monitor_id) != id
        {
            continue;
        }
        let a = s.tile_appearances.get(&format!("fence:{}", f.id));
        let collapsed = a.map_or(f.collapsed, |a| a.collapsed);
        let (mut l, r, header) = shell(f.projection_monitor_id.as_ref().and(f.projection_bounds).unwrap_or(f.bounds), &f.title, a, collapsed, HEADER);
        let mut members: Vec<(String, Rect)> = Vec::new();
        let mut content_clip = None;
        if !collapsed && r.height > header + 4.
        {
            let clip = Rect::new(r.x + 2., r.y + 2. + header, (r.width - 4.).max(0.), r.height - header - 4.);
            content_clip = Some(clip);
            l.push(DrawCommand::PushClip { rect: clip });
            let grid = p.fence_grid;
            let (w, h, dx, dy) = match f.layout_mode
            {
                FenceLayoutMode::Tile => (grid.width as f32, grid.height as f32, grid.column_gap as f32, grid.row_gap as f32),
                FenceLayoutMode::List => ((clip.width - 16.).max(0.), 46., 0., 4.),
            };
            let columns = (((clip.width - 16. + dx) / (w + dx).max(1.)).floor() as usize).max(1);
            for (i, member) in f.members.iter().enumerate()
            {
                let rect = Rect::new(clip.x + 8. + (i % columns) as f32 * (w + dx), clip.y + 8. + (i / columns) as f32 * (h + dy) - f.scroll_top as f32, w, h);
                if rect.y >= clip.bottom()
                {
                    break;
                }
                if rect.bottom() > clip.y
                {
                    l.push(mark(rect));
                    members.push((member.clone(), rect));
                }
            }
            l.push(DrawCommand::PopClip);
        }
        // 围栏本体：`1000 + 持久 z`、优先级 50。
        blocks.push(Block { id: f.id.clone(), kind: TargetKind::Fence, rect: r, header, members, content_clip, priority: 50, z: TILE_Z_BASE + f.z_index as i32, list: l });
    }
    for c in &s.components
    {
        if c.projection_monitor_id.as_deref().unwrap_or(&c.monitor_id) != id
        {
            continue;
        }
        let Some((label, enabled)) = component_view(p, &c.component_id) else { continue };
        if !enabled
        {
            continue;
        }
        let a = s.tile_appearances.get(&format!("component:{}", c.instance_id));
        let (mut l, r, header) = shell(c.projection_monitor_id.as_ref().and(c.projection_bounds).unwrap_or(c.bounds), &label, a, a.is_some_and(|a| a.collapsed), if c.component_id == "official.collect-box" { 56. } else { HEADER });
        if let Some(content) = content
        {
            content.draw(&mut l, &c.component_id, &c.instance_id, r, header, text);
        }
        // 组件本体是 `data-hit-kind="component"`、优先级 60；顶层组件没有自己的 z-index，按 0 计。
        blocks.push(Block { id: c.instance_id.clone(), kind: TargetKind::Component, rect: r, header, members: Vec::new(), content_clip: None, priority: 60, z: 0, list: l });
    }
    for portal in &s.folder_portals
    {
        if p.folder_portal_enabled && portal.monitor_id == id
        {
            let label = Path::new(&portal.root_path).file_name().and_then(|v| v.to_str()).unwrap_or("文件夹门户");
            let (l, r, header) = shell(portal.bounds, label, Some(&portal.appearance), portal.appearance.collapsed, HEADER);
            // 门户同样是 `1000 + 持久 z`、优先级 60（与组件同优先级，靠插入顺序决定先后：门户在后）。
            blocks.push(Block { id: portal.id.clone(), kind: TargetKind::Portal, rect: r, header, members: Vec::new(), content_clip: None, priority: 60, z: TILE_Z_BASE + portal.z_index as i32, list: l });
        }
    }

    // 唯一的排序点：层级低者先画/后被命中；同层按 priority，再按插入顺序（组件在门户之前）。
    blocks.sort_by_key(|block| (block.z, block.priority));
    if let Some(focus) = focus
    {
        // 焦点块整体上移：内容与边框一起画到最后，命中时也最先被查到。
        if let Some(index) = blocks.iter().position(|block| block.id == focus)
        {
            let focused = blocks.remove(index);
            blocks.push(focused);
        }
    }
    blocks
}

/// 普通绘制（无焦点态）：把块按绘制顺序拼成一条指令列表。
pub(super) fn draw_with_content(s: &WorkspaceSnapshot, p: &ApplicationPreferences, id: &str, g: HostGeometry, content: Option<&super::content::Content>) -> DrawList
{
    join(scene_blocks(s, p, id, g, content, None))
}

/// 把块的指令按顺序拼起来。
fn join(blocks: Vec<Block>) -> DrawList
{
    let mut list = DrawList::new();
    for block in blocks
    {
        for command in block.list.commands()
        {
            list.push(command.clone());
        }
    }
    list
}

/// 命中场景：直接由 [`scene_blocks`] 的**同一个顺序**生成，不再单独排序。
pub(super) fn scene_hits(s: &WorkspaceSnapshot, p: &ApplicationPreferences, id: &str, g: HostGeometry, focus: Option<&str>) -> Vec<super::input::HitEntry>
{
    use super::input::{HitEntry, TargetKind};
    let (width, height) = g.dip_size();
    let viewport = Rect::new(0., 0., width, height);
    let mut entries: Vec<HitEntry> = Vec::new();
    // 背景：整块表面、最低优先级，对应前端 `workspace-background`（priority 1）。
    entries.push(HitEntry::new("workspace-background", TargetKind::Background, viewport, 0., Vec::new(), 1, i32::MIN, Some(viewport)));
    for block in scene_blocks(s, p, id, g, None, focus)
    {
        // 成员与格子的命中范围都要和可见区域（内容框 / 视口）求交，不用未裁剪矩形。
        let members = block
            .members
            .into_iter()
            .filter_map(|(id, rect)| {
                let clip = match (block.content_clip, Some(viewport)) {
                    (Some(content), Some(viewport)) => intersect(content, viewport),
                    (None, Some(viewport)) => Some(viewport),
                    _ => None,
                };
                let clip = clip.unwrap_or(viewport);
                intersect(rect, clip).map(|visible| (id, visible))
            })
            .collect();
        entries.push(HitEntry::new(block.id, block.kind, block.rect, block.header, members, block.priority, block.z, Some(viewport)));
    }
    entries
}

/// 两个矩形的交集；没有交叠时返回 None。
pub(super) fn intersect(a: Rect, b: Rect) -> Option<Rect>
{
    let left = a.x.max(b.x);
    let top = a.y.max(b.y);
    let right = a.right().min(b.right());
    let bottom = a.bottom().min(b.bottom());
    if right > left && bottom > top { Some(Rect::new(left, top, right - left, bottom - top)) } else { None }
}

/// 带焦点态的场景：同一个块的列表，焦点块已经整体排到最后（内容 + 边框一起上移）。
pub(super) fn draw_with_focus(s: &WorkspaceSnapshot, p: &ApplicationPreferences, id: &str, g: HostGeometry, focus: Option<&str>) -> DrawList
{
    let mut blocks = scene_blocks(s, p, id, g, None, focus);
    let accent = super::input::focus_border_color(&p.desktop_appearance.accent_color);
    let radius = p.corner_radius.min(8) as f32;
    if let Some(focus) = focus
    {
        if let Some(block) = blocks.iter_mut().rev().find(|block| block.id == focus)
        {
            // 焦点边框画在该块自己的指令末尾：和内容同层，随块一起被抬到最上层。
            let r = block.rect;
            let inner = Rect::new(r.x + 0.5, r.y + 0.5, (r.width - 1.).max(0.), (r.height - 1.).max(0.));
            block.list.push(DrawCommand::StrokeRoundRect { rect: inner, radius: (radius - 0.5).max(0.), width: 1., color: accent });
        }
    }
    join(blocks)
}

/// 实际用于焦点边框的颜色（报告与绘制共用同一个值）。
pub(super) fn focus_border_hex(p: &ApplicationPreferences) -> String
{
    super::input::focus_border_color(&p.desktop_appearance.accent_color).to_hex_rgb()
}

/// 布局与偏好的指纹：只读文件字节做散列，用于证明诊断前后没有写盘。
pub(super) fn layout_fingerprint() -> Result<(String, String)>
{
    use std::hash::{Hash, Hasher};
    let dir = config_dir()?;
    let digest = |path: PathBuf| -> Result<String> {
        let bytes = std::fs::read(&path).map_err(|e| fail(format!("{}：{e}", path.display())))?;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        bytes.hash(&mut hasher);
        Ok(format!("{:016x}/{}B", hasher.finish(), bytes.len()))
    };
    Ok((digest(dir.join("workspace-layout.json"))?, digest(dir.join("settings.json"))?))
}

#[cfg(test)]
mod tests
{
    use super::super::geometry::{DpiScale, PixelRect};
    use super::*;

    /// 两屏共享同一份布局：只在有效投影屏绘制，输入快照不变，框与标题轨道均使用局部 DIP。
    #[test]
    fn projected_fence_with_placeholder_uses_source_geometry()
    {
        let scene: WorkspaceSnapshot = serde_json::from_value(serde_json::json!({"schemaVersion":1,"revision":4,"fences":[{"id":"f","title":"资料","monitorId":"A","projectionMonitorId":"B","projectionBounds":{"left":30,"top":40,"width":200,"height":160},"bounds":{"left":0,"top":0,"width":300,"height":200},"members":["a"],"layoutMode":"tile","scrollTop":0,"collapsed":false,"lastExpandedHeight":200,"zIndex":1}],"unclassifiedItems":[]})).unwrap();
        let g = HostGeometry::for_work_area(PixelRect::new(-1200, 0, 0, 900), DpiScale::new(144));
        let p = ApplicationPreferences { corner_radius: 12, ..Default::default() };
        assert!(draw_snapshot(&scene, &p, "A", g).is_empty());
        let list = draw_snapshot(&scene, &p, "B", g);
        assert!(list.is_balanced());
        assert!(matches!(list.commands()[0], DrawCommand::FillRoundRect { rect, radius: 8., .. } if rect == Rect::new(30., 40., 200., 160.)));
        assert!(matches!(list.commands()[1], DrawCommand::StrokeRoundRect { width: 1., .. }));
        assert!(matches!(&list.commands()[2], DrawCommand::Text { text, .. } if text == "资料"));
        assert!(matches!(list.commands()[3], DrawCommand::PushClip { rect } if rect.y == 40. + 2. + 28.));
        assert!(matches!(list.commands()[4], DrawCommand::StrokeRoundRect { rect, .. } if rect.x == 40. && rect.y == 78.));
    }

    #[test]
    fn clamp_relative_dip_on_negative_origin_monitor()
    {
        let g = HostGeometry::for_work_area(PixelRect::new(-1800, 30, 0, 1230), DpiScale::new(144));
        let r = local_rect(WorkspaceRect { left: 1100., top: -20., width: 240., height: 160. }, g, None);
        assert_eq!(r, Rect::new(960., 0., 240., 160.));
    }

    /// F1：绘制顺序与命中顺序来自同一个块列表；焦点块整体上移后两边一起变。
    #[test]
    fn draw_and_hit_share_one_block_order()
    {
        use super::super::input::HitScene;
        let scene: WorkspaceSnapshot = serde_json::from_value(serde_json::json!({
            "schemaVersion": 1, "revision": 7,
            "fences": [
                {"id": "low", "title": "低", "monitorId": "A", "bounds": {"left": 0.0, "top": 0.0, "width": 200.0, "height": 200.0}, "members": [], "layoutMode": "tile", "scrollTop": 0, "collapsed": false, "lastExpandedHeight": 200.0, "zIndex": 1},
                {"id": "high", "title": "高", "monitorId": "A", "bounds": {"left": 50.0, "top": 50.0, "width": 200.0, "height": 200.0}, "members": [], "layoutMode": "tile", "scrollTop": 0, "collapsed": false, "lastExpandedHeight": 200.0, "zIndex": 2}
            ],
            "unclassifiedItems": []
        })).unwrap();
        let p = ApplicationPreferences::default();
        let g = HostGeometry::for_work_area(PixelRect::new(0, 0, 800, 600), DpiScale::new(96));

        let blocks = scene_blocks(&scene, &p, "A", g, None, None);
        let block_ids: Vec<&str> = blocks.iter().map(|block| block.id.as_str()).collect();
        assert_eq!(block_ids, vec!["low", "high"]);
        let hits = scene_hits(&scene, &p, "A", g, None);
        let hit_ids: Vec<&str> = hits.iter().skip(1).map(|entry| entry.id.as_str()).collect();
        assert_eq!(hit_ids, block_ids, "命中条目顺序必须与绘制顺序一致（除背景外）");
        assert_eq!(HitScene::new(hits, None).hit(100.0, 100.0).unwrap().id, "high");

        // 焦点 low：块整体移到末尾，命中也是 low 最先，重叠区不再点到 high。
        let focused_blocks = scene_blocks(&scene, &p, "A", g, None, Some("low"));
        assert_eq!(focused_blocks.last().map(|block| block.id.as_str()), Some("low"));
        let focused_hits = scene_hits(&scene, &p, "A", g, Some("low"));
        assert_eq!(focused_hits.last().map(|entry| entry.id.as_str()), Some("low"));
        assert_eq!(HitScene::new(focused_hits, Some("low")).hit(100.0, 100.0).unwrap().id, "low", "焦点格置顶后应吃掉重叠区命中");

        // 焦点边框画在焦点块自己的指令末尾：内容与边框同层，一起上移。
        let list = draw_with_focus(&scene, &p, "A", g, Some("low"));
        let last_is_accent = matches!(list.commands().last(), Some(DrawCommand::StrokeRoundRect { width, .. }) if (*width - 1.0).abs() < f32::EPSILON);
        assert!(last_is_accent, "最后一条应是焦点边框描边");
    }

    /// F2：成员的命中范围是「成员矩形 ∩ 内容框 ∩ 视口」，越出可见区域的片段不参与命中。
    #[test]
    fn members_are_clipped_to_content_and_viewport()
    {
        let scene: WorkspaceSnapshot = serde_json::from_value(serde_json::json!({
            "schemaVersion": 1, "revision": 8,
            "fences": [
                {"id": "f", "title": "围栏", "monitorId": "A", "bounds": {"left": 40.0, "top": -60.0, "width": 300.0, "height": 200.0}, "members": ["m1", "m2", "m3", "m4", "m5", "m6"], "layoutMode": "tile", "scrollTop": 0, "collapsed": false, "lastExpandedHeight": 200.0, "zIndex": 1}
            ],
            "unclassifiedItems": []
        })).unwrap();
        let p = ApplicationPreferences::default();
        let g = HostGeometry::for_work_area(PixelRect::new(0, 0, 800, 600), DpiScale::new(96));
        let entries = scene_hits(&scene, &p, "A", g, None);
        let fence = entries.iter().find(|entry| entry.id == "f").expect("围栏条目");
        assert!(!fence.members.is_empty(), "至少应有可见成员");
        // 内容框按**实际绘制矩形**（已钳进工作区）推出，而不是按快照里的原始 bounds。
        let content = Rect::new(fence.rect.x + 2.0, fence.rect.y + 2.0 + fence.header, (fence.rect.width - 4.0).max(0.0), (fence.rect.height - fence.header - 4.0).max(0.0));
        let viewport = Rect::new(0.0, 0.0, 800.0, 600.0);
        for (id, rect) in &fence.members
        {
            assert!(rect.x >= content.x - 0.01 && rect.right() <= content.right() + 0.01, "成员 {id} 必须裁到内容框：{rect:?}");
            assert!(rect.y >= content.y - 0.01 && rect.bottom() <= content.bottom() + 0.01, "成员 {id} 必须裁到内容框：{rect:?}");
            assert!(rect.y >= viewport.y - 0.01 && rect.bottom() <= viewport.bottom() + 0.01, "成员 {id} 必须裁到视口：{rect:?}");
        }
    }
}
