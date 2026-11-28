//! 音乐格内容的原生绘制（**只读**接入现有媒体状态）。
//!
//! 取数：曲目/艺人/播放状态来自 `crate::platform::windows::music::MusicSnapshot`（只读引用，不改链路），
//! 进度与频谱由调用方从既有 `music-timeline-changed` / `audio-spectrum-changed` 事件里取，本模块不新开采集。
//!
//! 数值出处（`docs/design/视觉规范.md`）：
//! - L179 宽布局无内边距、左封面右信息；紧凑布局封面作裁切背景、信息与按钮贴底、内边距 `6px`；
//! - L180 频谱独立条带：宽布局在右栏信息下方、宽=右栏宽、高 `18 DIP`、信息下间距 `6 DIP`；紧凑布局贴内容底边、
//!   宽占满内容（最小 `168 DIP`）、高 `18 DIP`；条带透明无圆角无描边、`pointer-events: none`、柱色取
//!   `--fence-text` 的 `75%`、底部对齐；
//! - L181 实绘段数 = min(设置段数, floor(条带宽 / 3))，单段横向步进 ≤ `12 DIP`，宽裕时整组居中；
//!   暂停/静音/无会话/采集失败时画 `1 DIP` 高、同色 `30%` 的基线占位。
//! `docs/design/音乐控件.md`：歌曲名与艺人继承 `--interface-font-size`（13px）、名称字重 400、艺人透明度 0.78。

use super::draw::{Color, DrawCommand, DrawList, Rect, TextAlign};

use windows::core::Result;

/// 频谱段数上限（设置里的默认段数）。
pub const SPECTRUM_BANDS: usize = 24;
/// 频谱条带高度（视觉规范 L180）。
pub const SPECTRUM_HEIGHT: f32 = 18.0;
/// 单段最大横向步进（视觉规范 L181）。
pub const SPECTRUM_MAX_STEP: f32 = 12.0;
/// 单段最小步进：实绘段数 = min(段数, floor(带宽 / 3))（视觉规范 L181）。
pub const SPECTRUM_MIN_STEP: f32 = 3.0;
/// 紧凑布局内边距（视觉规范 L179）。
pub const COMPACT_PADDING: f32 = 6.0;
/// 信息与频谱之间间距（视觉规范 L180）。
pub const INFO_GAP: f32 = 6.0;
/// 紧凑布局内容高（含条带，视觉规范 L181）。
pub const COMPACT_HEIGHT: f32 = 112.0;
/// 紧凑布局最小内容宽（视觉规范 L181）。
pub const COMPACT_WIDTH: f32 = 168.0;
/// 行盒高度 = 字号（`1lh`，line-height 1，视觉规范 L179「文字元素高 1lh」）。
///
/// 取舍说明：原生文本绘制把「行盒」直接当作 `Text` 的矩形高度，基线由画笔内部按字号决定；
/// 因此这里行盒 = 13 DIP，不额外加行距（行间距 `2px` 由相邻行盒之间显式留出）。
pub const LINE_HEIGHT: f32 = 13.0;
/// 最小可渲染内容尺寸：宽 `96`（封面 40 + 间距 12 + 文字 44）、高 `40`（两行文字 + 进度）。
/// 小于该尺寸一律拒绝，不做负数/零宽硬算。
/// 最小宽度：两侧内边距 + 至少 1 个图标的实际内容（规范只要求「大于两侧留白 + 实际内容」）。
pub const MIN_WIDTH: f32 = 24.0;
/// 紧凑布局最小高：进度条（最先出界元素）底部 = `h − 频谱高 − 43`，故 关频谱 ≥ 43、开频谱 ≥ 61。
pub const COMPACT_MIN_HEIGHT_OFF: f32 = 43.0;
pub const COMPACT_MIN_HEIGHT_ON: f32 = 61.0;
/// 宽布局最小内容高：两行 13 + 间距 2/6 + 进度 3 + 间距 6 + 频谱 18 = 61 DIP。
pub const WIDE_MIN_HEIGHT: f32 = 61.0;
/// 兼容旧名（等价于紧凑态关频谱阈值）。
pub const MIN_HEIGHT: f32 = COMPACT_MIN_HEIGHT_OFF;

/// 宽布局参考内容尺寸（视觉规范 L180 举例 396 × 148）。
pub const WIDE_WIDTH: f32 = 396.0;
pub const WIDE_HEIGHT: f32 = 148.0;

/// 布局形态。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MusicLayoutKind
{
    /// 宽布局：左封面、右上信息、右下按钮区、信息下方频谱。
    Wide,
    /// 紧凑布局：封面铺满作背景、信息与按钮贴底、频谱贴内容底边。
    Compact,
}

/// 绘制样式（进入去重键：颜色/主题变化同样要重绘）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MusicStyle
{
    pub text: Color,
    pub fence_text: Color,
    pub dark: bool,
}

/// 只读的音乐视图数据（由调用方从现有状态与事件拼装，本模块不做采集）。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MusicView
{
    pub title: String,
    /// 艺人；空串表示没有艺人信息（降级为只显示标题）。
    pub artist: String,
    /// 播放状态原文（`playing` / `paused` / `stopped` 等，来自 `MusicSnapshot::playback`）。
    pub playback: String,
    /// 进度 0.0–1.0；`None` 表示没有可用的时间轴（不画进度填充）。
    pub progress: Option<f64>,
    /// 频谱：0–255 的对数幅度，索引从低频到高频（`SpectrumFrame::bins`）；空表示无数据。
    pub bins: Vec<u8>,
    /// 是否静音（静音与暂停一样画基线）。
    pub muted: bool,
    /// 显示配置里的频谱段数；`None` 表示调用方未提供（**仅 fixture/诊断缺省**才回落到 24）。
    pub bands: Option<usize>,
}

impl MusicView
{
    /// 实际绘制段数 = min(配置段数, floor(条带宽 / 3))；配置缺省时才用 24（仅 fixture 缺省）。
    pub fn effective_bands(&self, spectrum: Rect) -> usize
    {
        let configured = self.bands.unwrap_or(SPECTRUM_BANDS);
        configured.min((spectrum.width / SPECTRUM_MIN_STEP).floor().max(0.0) as usize)
    }

    /// 是否无曲目（标题为空）——用于「无曲目」降级。
    pub fn is_empty(&self) -> bool
    {
        self.title.trim().is_empty()
    }

    /// 是否应画基线而不是实柱（暂停 / 静音 / 无会话 / 采集失败，视觉规范 L181）。
    pub fn is_idle(&self) -> bool
    {
        self.is_empty() || self.muted || self.playback != "playing" || self.bins.is_empty()
    }
}

/// 一次布局的产物（窗口/内容局部 DIP）。
#[derive(Clone, Debug, PartialEq)]
pub struct MusicLayout
{
    pub kind: MusicLayoutKind,
    /// 封面：宽布局左侧正方形；紧凑布局铺满内容（作为裁切背景，这里只给矩形）。
    pub artwork: Rect,
    /// 标题行（单行省略）。
    pub title: Rect,
    /// 艺人行（单行省略）。
    pub artist: Rect,
    /// 进度条轨道。
    pub progress: Rect,
    /// 频谱条带。
    pub spectrum: Rect,
    /// 是否显示频谱条带（关闭开关时整行不占位）。
    pub spectrum_visible: bool,
}

/// 计算布局并做**最小尺寸校验**：空盒或过小的内容盒直接拒绝，不做负数硬算。
///
/// 拒绝阈值见 [`MIN_WIDTH`] / [`MIN_HEIGHT`]；`layout()` 保留为不做校验的纯几何函数（调用方已保证尺寸时用）。
pub fn layout_checked(kind: MusicLayoutKind, content: Rect, spectrum_enabled: bool) -> std::result::Result<MusicLayout, String>
{
    if content.width <= MIN_WIDTH
    {
        return Err(format!("音乐内容盒宽 {:.0} DIP 太小（需大于两侧留白 + 内容，至少 >{MIN_WIDTH:.0}）", content.width));
    }
    // 高度阈值以**最先出界的元素**为准：紧凑态进度条底部 = h − 频谱高 − 43。
    let minimum_height = match (kind, spectrum_enabled)
    {
        (MusicLayoutKind::Compact, true) => COMPACT_MIN_HEIGHT_ON,
        (MusicLayoutKind::Compact, false) => COMPACT_MIN_HEIGHT_OFF,
        (MusicLayoutKind::Wide, _) => WIDE_MIN_HEIGHT,
    };
    if content.height < minimum_height
    {
        return Err(format!("内容高 {:.0} DIP 不足（该档位至少 {minimum_height:.0}）", content.height));
    }
    Ok(layout(kind, content, spectrum_enabled))
}

/// 计算布局。`content` 是内容盒（不含标题栏），`spectrum_enabled` 对应「显示实时频谱」开关。
pub fn layout(kind: MusicLayoutKind, content: Rect, spectrum_enabled: bool) -> MusicLayout
{
    match kind
    {
        MusicLayoutKind::Wide =>
        {
            // 封面 = min((内容宽 − 24) × 0.4, 内容高)（`docs/design/音乐控件.md` 封面公式）。
            let artwork_side = (((content.width - 24.0) * 0.4).max(0.0)).min(content.height);
            let artwork = Rect::new(content.x, content.y, artwork_side, artwork_side);
            let column_x = content.x + artwork_side + 12.0; // 左右列间距 12px。
            let column = Rect::new(column_x, content.y, (content.right() - column_x).max(0.0), content.height);
            let line = content.height.min(LINE_HEIGHT);
            let title = Rect::new(column.x, column.y, column.width, line);
            let artist = Rect::new(column.x, column.y + line + 2.0, column.width, line); // 行间距 2px。
            let progress_y = artist.bottom() + 6.0;
            let progress = Rect::new(column.x, progress_y, column.width, 3.0);
            let spectrum_height = if spectrum_enabled { SPECTRUM_HEIGHT } else { 0.0 };
            let spectrum_y = progress.bottom() + INFO_GAP;
            let spectrum = Rect::new(column.x, spectrum_y, column.width, spectrum_height);
            MusicLayout { kind, artwork, title, artist, progress, spectrum, spectrum_visible: spectrum_enabled }
        }
        MusicLayoutKind::Compact =>
        {
            let padding = COMPACT_PADDING;
            let spectrum_height = if spectrum_enabled { SPECTRUM_HEIGHT } else { 0.0 };
            let artwork = Rect::new(content.x, content.y, content.width, (content.height - spectrum_height).max(0.0));
            let inner_x = content.x + padding;
            let inner_width = (content.width - padding * 2.0).max(0.0);
            let spectrum = Rect::new(content.x, content.bottom() - spectrum_height, content.width, spectrum_height);
            let line = LINE_HEIGHT;
            let artist_y = artwork.bottom() - padding - line;
            let title_y = artist_y - 2.0 - line;
            let progress = Rect::new(inner_x, title_y - 6.0 - 3.0, inner_width, 3.0);
            MusicLayout {
                kind,
                artwork,
                title: Rect::new(inner_x, title_y, inner_width, line),
                artist: Rect::new(inner_x, artist_y, inner_width, line),
                progress,
                spectrum,
                spectrum_visible: spectrum_enabled,
            }
        }
    }
}

/// 频谱条柱矩形：`count = min(bands, floor(width / 3))`，步进 ≤ 12，宽裕时整组居中，底部对齐。
pub fn spectrum_bars(spectrum: Rect, bands: usize) -> Vec<Rect>
{
    if spectrum.width <= 0.0 || spectrum.height <= 0.0 || bands == 0
    {
        return Vec::new();
    }
    let count = bands.min((spectrum.width / SPECTRUM_MIN_STEP).floor().max(0.0) as usize);
    if count == 0
    {
        return Vec::new();
    }
    let step = (spectrum.width / count as f32).min(SPECTRUM_MAX_STEP);
    let bar_width = (step * 0.6).max(1.0);
    let group = step * count as f32;
    let start = spectrum.x + (spectrum.width - group) / 2.0;
    (0..count)
        .map(|index| Rect::new(start + index as f32 * step + (step - bar_width) / 2.0, spectrum.y, bar_width, spectrum.height))
        .collect()
}

/// 把任意长度的 `bins` 重采样到 `count` 段：**缩段取组内峰值**（最大值），升段取最近样本。
///
/// 依据视觉规范 L181「按峰值重采样」——直接取索引样本会在缩段时漏掉峰值（反例见单测）。
pub fn resample_bins(bins: &[u8], count: usize) -> Vec<u8>
{
    if count == 0 || bins.is_empty()
    {
        return Vec::new();
    }
    if bins.len() <= count
    {
        return (0..count)
            .map(|index| bins[(index * bins.len() / count).min(bins.len() - 1)])
            .collect();
    }
    (0..count)
        .map(|index| {
            let start = index * bins.len() / count;
            let end = ((index + 1) * bins.len() / count).max(start + 1);
            bins[start..end].iter().copied().max().unwrap_or(0)
        })
        .collect()
}

/// 某个柱的高度比例：0–255 → 0.05–1.0（留 5% 让静音段仍可见）。
pub fn bar_ratio(value: u8) -> f32
{
    0.05 + (f32::from(value) / 255.0) * 0.95
}

/// 绘制：封面占位框、单行省略的标题/艺人、进度条、频谱条带。
///
/// 颜色：正文 `text`、艺人 `text` 的 0.78（`docs/design/音乐控件.md`）、柱色 `fence_text` 的 75%（视觉规范 L180）。
pub fn draw(view: &MusicView, layout: &MusicLayout, text: Color, fence_text: Color, dark: bool) -> DrawList
{
    let mut list = DrawList::new();
    let surface = if dark { Color::rgba(0x24, 0x2A, 0x2C, 0xFF) } else { Color::rgba(0xFF, 0xFF, 0xFF, 0xFF) };
    let muted = if dark { Color::rgba(0xA8, 0xB4, 0xB6, 0xFF) } else { Color::rgba(0x5C, 0x66, 0x68, 0xFF) };
    // 封面占位框（刻意降级：尚未接入图片解码）。
    list.push(DrawCommand::FillRoundRect { rect: layout.artwork, radius: 4.0, color: surface });
    list.push(DrawCommand::StrokeRoundRect { rect: Rect::new(layout.artwork.x + 0.5, layout.artwork.y + 0.5, (layout.artwork.width - 1.0).max(0.0), (layout.artwork.height - 1.0).max(0.0)), radius: 4.0, width: 1.0, color: muted });
    let (title, artist) = match view.is_empty()
    {
        // 无曲目降级：标题位置给固定文案，艺人不画。
        true => ("未在播放".to_string(), String::new()),
        false => (view.title.clone(), view.artist.clone()),
    };
    list.push(DrawCommand::Text { rect: layout.title, text: title, size: 13.0, weight: 400, color: text, align: TextAlign::Start, max_lines: 1 });
    if !artist.trim().is_empty()
    {
        list.push(DrawCommand::Text { rect: layout.artist, text: artist, size: 13.0, weight: 400, color: text.with_alpha(0.78), align: TextAlign::Start, max_lines: 1 });
    }
    // 进度条：轨道 + 填充（无时间轴时不画填充）。
    list.push(DrawCommand::FillRoundRect { rect: layout.progress, radius: 1.5, color: muted.with_alpha(0.28) });
    if let Some(progress) = view.progress
    {
        let ratio = progress.clamp(0.0, 1.0) as f32;
        list.push(DrawCommand::FillRoundRect { rect: Rect::new(layout.progress.x, layout.progress.y, layout.progress.width * ratio, layout.progress.height), radius: 1.5, color: text.with_alpha(0.85) });
    }
    // 频谱：条带内柱状，底部对齐；空闲时画 1 DIP 基线（同色 30%）。
    if layout.spectrum_visible
    {
        let bars = spectrum_bars(layout.spectrum, view.effective_bands(layout.spectrum));
        if view.is_idle()
        {
            list.push(DrawCommand::FillRoundRect { rect: Rect::new(layout.spectrum.x, layout.spectrum.bottom() - 1.0, layout.spectrum.width, 1.0), radius: 0.0, color: fence_text.with_alpha(0.30) });
        }
        else
        {
            let resampled = resample_bins(&view.bins, bars.len());
            for (index, bar) in bars.iter().enumerate()
            {
                let value = resampled.get(index).copied().unwrap_or(0);
                let height = (bar.height * bar_ratio(value)).max(1.0);
                list.push(DrawCommand::FillRoundRect { rect: Rect::new(bar.x, bar.bottom() - height, bar.width, height), radius: 0.0, color: fence_text.with_alpha(0.75) });
            }
        }
    }
    list
}

/// 「一次数据更新 → 一次绘制」的呈现器：只在数据变化时产出绘制列表。
#[derive(Default)]
pub struct MusicPresenter
{
    /// 上一次真正绘制的「数据 + 布局 + 样式」组合；任一变化都必须重绘。
    last: Option<(MusicView, MusicLayout, MusicStyle)>,
    draws: usize,
    skipped: usize,
    /// 真正构造出 DrawList 的次数（与 draws 同步，独立计数便于断言「没有凭空生成列表」）。
    built: usize,
}

impl MusicPresenter
{
    /// 提交一帧「数据 + 布局 + 样式」，**只有在判等通过时才调用 `build` 生成绘制列表**。
    ///
    /// 判等键 = 数据（曲目/艺人/播放态/进度/频谱/段数）+ 布局（档位与全部元素矩形）+ 样式（颜色/主题）。
    /// 相同则直接返回 `None`，`build` **不会被执行**（因此不会产生 DrawList）；
    /// 不同才执行 `build`、递增 `draws` 并返回列表。`built` 统计真正构造出的 DrawList 次数，供测试断言。
    pub fn submit(&mut self, view: &MusicView, layout: &MusicLayout, style: &MusicStyle, build: impl FnOnce() -> DrawList) -> Option<DrawList>
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

    /// 真正构造出 `DrawList` 的次数（判等通过才 +1）。
    pub fn built(&self) -> usize
    {
        self.built
    }

    /// 绘制次数（一次数据变化一次）。
    pub fn draws(&self) -> usize
    {
        self.draws
    }

    /// 被跳过的重复提交次数。
    pub fn skipped(&self) -> usize
    {
        self.skipped
    }
}

#[cfg(test)]
mod tests
{
    use super::*;

    fn view(title: &str, artist: &str, bins: Vec<u8>) -> MusicView
    {
        MusicView { title: title.into(), artist: artist.into(), playback: "playing".into(), progress: Some(0.5), bins, muted: false, bands: None }
    }

    /// 宽布局：封面 = min((396−24)×0.4, 148) = 148；右栏 x = 148+12；频谱在信息下方 6 DIP。
    #[test]
    fn wide_layout_follows_spec_numbers()
    {
        let content = Rect::new(0.0, 0.0, WIDE_WIDTH, WIDE_HEIGHT);
        let layout = layout(MusicLayoutKind::Wide, content, true);
        assert_eq!(layout.artwork.width, 148.0, "封面 = min((396−24)×0.4, 148) = 148");
        assert_eq!(layout.title.x, 148.0 + 12.0, "左右列间距 12px");
        assert_eq!(layout.spectrum.height, SPECTRUM_HEIGHT, "频谱高 18 DIP");
        assert_eq!(layout.spectrum.height, 18.0);
        assert!((layout.spectrum.y - (layout.progress.bottom() + 6.0)).abs() < 0.01, "信息下间距 6 DIP");
        assert_eq!(layout.spectrum.width, WIDE_WIDTH - 160.0, "频谱宽 = 右栏宽（396−148−12=236）");
    }

    /// 紧凑布局：内边距 6，频谱贴内容底边、宽占满；关闭频谱时条带不占位。
    #[test]
    fn compact_layout_reserves_spectrum_only_when_enabled()
    {
        let content = Rect::new(0.0, 0.0, COMPACT_WIDTH, COMPACT_HEIGHT);
        let on = layout(MusicLayoutKind::Compact, content, true);
        assert_eq!(on.spectrum.height, 18.0);
        assert_eq!(on.spectrum.bottom(), content.bottom(), "频谱贴内容底边");
        assert_eq!(on.spectrum.width, COMPACT_WIDTH, "宽占满内容");
        assert_eq!(on.artwork.height, COMPACT_HEIGHT - 18.0, "封面高度 = 内容高 − 18");
        assert_eq!(on.title.x, COMPACT_PADDING, "紧凑内边距 6px");
        let off = layout(MusicLayoutKind::Compact, content, false);
        assert_eq!(off.spectrum.height, 0.0, "关闭开关时条带整行不占位");
        assert_eq!(off.artwork.height, COMPACT_HEIGHT, "封面用满内容高");
    }

    /// 单行省略：标题与艺人矩形高度都等于一行（13px），不换行。
    #[test]
    fn title_and_artist_are_single_line()
    {
        let content = Rect::new(0.0, 0.0, WIDE_WIDTH, WIDE_HEIGHT);
        let layout = layout(MusicLayoutKind::Wide, content, true);
        assert_eq!(layout.title.height, 13.0);
        assert_eq!(layout.artist.height, 13.0);
        let list = draw(&view("很长很长很长很长很长很长很长很长的标题", "艺人", vec![128; 24]), &layout, Color::rgba(0xE8, 0xEF, 0xF0, 0xFF), Color::rgba(0xE8, 0xEF, 0xF0, 0xFF), true);
        let text_commands: Vec<&DrawCommand> = list.commands().iter().filter(|command| matches!(command, DrawCommand::Text { .. })).collect();
        for command in text_commands
        {
            match command
            {
                DrawCommand::Text { max_lines, .. } => assert_eq!(*max_lines, 1, "标题/艺人必须单行"),
                _ => {}
            }
        }
    }

    /// 频谱：24 段；带宽不足时按 floor(宽/3) 减少；步进 ≤ 12；底部对齐。
    #[test]
    fn spectrum_bars_respect_count_step_and_alignment()
    {
        let wide = spectrum_bars(Rect::new(0.0, 0.0, 236.0, 18.0), SPECTRUM_BANDS);
        assert_eq!(wide.len(), 24, "236/3 = 78 ≥ 24，取满 24 段");
        let step = wide[1].x - wide[0].x;
        assert!(step <= 12.0 + 0.01, "步进 ≤ 12，实际 {step}");
        assert_eq!(wide[0].y, 0.0, "柱顶在条带顶部");
        assert!((wide[0].bottom() - 18.0).abs() < 0.01 || wide[0].height <= 18.0);
        let narrow = spectrum_bars(Rect::new(0.0, 0.0, 40.0, 18.0), SPECTRUM_BANDS);
        assert_eq!(narrow.len(), 13, "floor(40/3) = 13 段");
        // 边界：0 段与零宽都不画。
        assert!(spectrum_bars(Rect::new(0.0, 0.0, 236.0, 18.0), 0).is_empty());
        assert!(spectrum_bars(Rect::new(0.0, 0.0, 0.0, 18.0), SPECTRUM_BANDS).is_empty());
    }

    /// 进度与柱高映射：0 / 中间 / 满 / 0 与 255 边界。
    #[test]
    fn progress_and_bar_ratio_map_boundaries()
    {
        assert!((bar_ratio(0) - 0.05).abs() < 1e-6, "0 → 5%");
        assert!((bar_ratio(255) - 1.0).abs() < 1e-6, "255 → 100%");
        assert!(bar_ratio(128) > 0.5 && bar_ratio(128) < 0.55);
        let content = Rect::new(0.0, 0.0, WIDE_WIDTH, WIDE_HEIGHT);
        let layout = layout(MusicLayoutKind::Wide, content, true);
        let text = Color::rgba(0xE8, 0xEF, 0xF0, 0xFF);
        for (progress, expected) in [(0.0, 0.0), (0.5, 0.5), (1.0, 1.0), (1.4, 1.0), (-0.3, 0.0)]
        {
            let mut data = view("歌", "艺人", vec![200; 24]);
            data.progress = Some(progress);
            let list = draw(&data, &layout, text, text, true);
            let fills: Vec<&DrawCommand> = list
                .commands()
                .iter()
                .filter(|command| matches!(command, DrawCommand::FillRoundRect { rect, .. } if (rect.height - 3.0).abs() < 0.01))
                .collect();
            assert_eq!(fills.len(), 2, "进度轨道 + 填充");
            match fills[1]
            {
                DrawCommand::FillRoundRect { rect, .. } => assert!((rect.width - layout.progress.width * expected as f32).abs() < 0.01, "进度 {progress} 的填充宽应为 {expected} 倍"),
                _ => unreachable!(),
            }
        }
    }

    /// 降级：无曲目显示「未在播放」且不画艺人；空闲态画 1 DIP 基线而不是柱。
    #[test]
    fn empty_and_idle_states_degrade()
    {
        let content = Rect::new(0.0, 0.0, WIDE_WIDTH, WIDE_HEIGHT);
        let wide = layout(MusicLayoutKind::Wide, content, true);
        let text = Color::rgba(0xE8, 0xEF, 0xF0, 0xFF);
        let empty = MusicView::default();
        assert!(empty.is_empty());
        let list = draw(&empty, &wide, text, text, true);
        let texts: Vec<String> = list
            .commands()
            .iter()
            .filter_map(|command| match command
            {
                DrawCommand::Text { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(texts, vec!["未在播放".to_string()], "无曲目只画固定文案");
        // 有曲目但暂停 → 基线（1 DIP 高、30% 色）。
        let mut paused = view("歌", "艺人", vec![10; 24]);
        paused.playback = "paused".into();
        assert!(paused.is_idle());
        let list = draw(&paused, &wide, text, text, true);
        let baseline = list.commands().iter().any(|command| matches!(command, DrawCommand::FillRoundRect { rect, .. } if (rect.height - 1.0).abs() < 0.01));
        assert!(baseline, "暂停时画 1 DIP 基线");
    }

    /// F1：判等通过才生成 DrawList —— 同一呈现目标连提三次，实际构造列表次数 = 1。
    #[test]
    fn presenter_builds_draw_list_only_on_change()
    {
        let mut presenter = MusicPresenter::default();
        let style = MusicStyle { text: Color::rgba(0xE8, 0xEF, 0xF0, 0xFF), fence_text: Color::rgba(0xD2, 0xE0, 0xE3, 0xFF), dark: true };
        let wide = layout(MusicLayoutKind::Wide, Rect::new(0.0, 0.0, WIDE_WIDTH, WIDE_HEIGHT), true);
        let data = view("歌", "艺人", vec![10; 24]);
        let build = |view: &MusicView, layout: &MusicLayout| {
            let list = draw(view, layout, style.text, style.fence_text, style.dark);
            list
        };
        // 同一呈现目标连提三次：只构造一次 DrawList。
        assert!(presenter.submit(&data, &wide, &style, || build(&data, &wide)).is_some());
        assert!(presenter.submit(&data, &wide, &style, || build(&data, &wide)).is_none());
        assert!(presenter.submit(&data, &wide, &style, || build(&data, &wide)).is_none());
        assert_eq!(presenter.built(), 1, "重复提交不得构造 DrawList");
        assert_eq!((presenter.draws(), presenter.skipped()), (1, 2));

        // 进度变化 / 频谱变化 / 样式变化 / 尺寸（布局）变化各触发一次构造。
        let mut progress = data.clone();
        progress.progress = Some(0.8);
        assert!(presenter.submit(&progress, &wide, &style, || build(&progress, &wide)).is_some());
        assert_eq!(presenter.built(), 2, "进度变化应构造一次");
        let mut bins = data.clone();
        bins.bins = vec![200; 24];
        assert!(presenter.submit(&bins, &wide, &style, || build(&bins, &wide)).is_some());
        assert_eq!(presenter.built(), 3, "频谱变化应构造一次");
        let mut light = style;
        light.dark = false;
        assert!(presenter.submit(&bins, &wide, &light, || build(&bins, &wide)).is_some());
        assert_eq!(presenter.built(), 4, "样式变化应构造一次");
        // 尺寸变化：同一档位换内容盒（布局矩形随之变化）。
        let wide_bigger = layout(MusicLayoutKind::Wide, Rect::new(0.0, 0.0, 500.0, 200.0), true);
        assert!(presenter.submit(&bins, &wide_bigger, &light, || build(&bins, &wide_bigger)).is_some());
        assert_eq!(presenter.built(), 5, "尺寸变化应构造一次");
    }

    /// F2：段数由配置输入 —— 配置 12 段 + 宽裕宽度 → 最终 DrawList 恰好 12 根柱。
    #[test]
    fn band_count_comes_from_config_and_is_capped_by_width()
    {
        let style = MusicStyle { text: Color::rgba(0xE8, 0xEF, 0xF0, 0xFF), fence_text: Color::rgba(0xD2, 0xE0, 0xE3, 0xFF), dark: true };
        let wide = layout(MusicLayoutKind::Wide, Rect::new(0.0, 0.0, WIDE_WIDTH, WIDE_HEIGHT), true);
        let count_bars = |view: &MusicView| {
            let list = draw(view, &wide, style.text, style.fence_text, style.dark);
            list.commands()
                .iter()
                .filter(|command| matches!(command, DrawCommand::FillRoundRect { rect, .. } if (rect.height - 1.0).abs() > 0.01 && rect.width < 20.0))
                .count()
        };
        let mut configured = view("歌", "艺人", vec![128; 48]);
        configured.bands = Some(12);
        assert_eq!(count_bars(&configured), 12, "配置 12 段时恰好 12 根柱（不硬编码 24）");
        // 缺省（fixture）才回落到 24。
        let mut fallback = configured.clone();
        fallback.bands = None;
        assert_eq!(count_bars(&fallback), 24, "缺省配置回落到 24");
        // 宽度窄时限幅：段数 = min(配置 24, floor(条带宽/3))。
        let narrow = layout(MusicLayoutKind::Wide, Rect::new(0.0, 0.0, 120.0, 148.0), true);
        let mut narrow_view = view("歌", "艺人", vec![128; 48]);
        narrow_view.bands = Some(24);
        let list = draw(&narrow_view, &narrow, style.text, style.fence_text, style.dark);
        let bars = list
            .commands()
            .iter()
            .filter(|command| matches!(command, DrawCommand::FillRoundRect { rect, .. } if (rect.height - 1.0).abs() > 0.01 && rect.width < 20.0))
            .count();
        let expected = 24usize.min((narrow.spectrum.width / SPECTRUM_MIN_STEP).floor() as usize);
        assert!(expected < 24, "该宽度应真的触发限幅（条带宽 {:.0}）", narrow.spectrum.width);
        assert_eq!(bars, expected, "段数取配置 ∩ floor(条带宽/3)");
    }

    /// F3：阈值以最先出界元素为准，且被拒盒不产出任何矩形。
    #[test]
    fn thresholds_reject_first_overflowing_element()
    {
        // 紧凑态：开频谱 ≥ 61、关频谱 ≥ 43。
        assert!(layout_checked(MusicLayoutKind::Compact, Rect::new(0.0, 0.0, 168.0, 61.0), true).is_ok(), "开频谱 61 应放行");
        assert!(layout_checked(MusicLayoutKind::Compact, Rect::new(0.0, 0.0, 168.0, 60.0), true).is_err(), "开频谱 60 应拒绝");
        assert!(layout_checked(MusicLayoutKind::Compact, Rect::new(0.0, 0.0, 168.0, 43.0), false).is_ok(), "关频谱 43 应放行");
        assert!(layout_checked(MusicLayoutKind::Compact, Rect::new(0.0, 0.0, 168.0, 42.0), false).is_err(), "关频谱 42 应拒绝");
        // 宽布局 ≥ 61；宽度必须大于两侧留白 + 内容。
        assert!(layout_checked(MusicLayoutKind::Wide, Rect::new(0.0, 0.0, 396.0, 61.0), true).is_ok());
        assert!(layout_checked(MusicLayoutKind::Wide, Rect::new(0.0, 0.0, 396.0, 60.0), true).is_err());
        assert!(layout_checked(MusicLayoutKind::Compact, Rect::new(0.0, 0.0, 3.0, 18.0), true).is_err(), "Compact 3×18 必须拒绝");
        assert!(layout_checked(MusicLayoutKind::Wide, Rect::new(0.0, 0.0, 3.0, 61.0), true).is_err(), "Wide 3×61 必须拒绝");
        // 被拒盒不产出矩形：`layout_checked` 返回 Err，调用方拿不到 layout，自然不画任何矩形。
        assert!(layout_checked(MusicLayoutKind::Wide, Rect::new(0.0, 0.0, 3.0, 61.0), true).is_err());
    }

    /// 缩段必须取峰值：`[0,255,0,255]` → 2 段应为 `[255,255]`；索引采样会得到 `[0,0]`（反例）。
    #[test]
    fn resample_takes_peak_when_shrinking()
    {
        let bins = vec![0u8, 255, 0, 255];
        assert_eq!(resample_bins(&bins, 2), vec![255, 255], "缩段取组内峰值");
        // 反例：直接按索引取样会漏掉峰值。
        let by_index: Vec<u8> = (0..2).map(|index| bins[index * bins.len() / 2]).collect();
        assert_eq!(by_index, vec![0, 0], "索引采样会漏峰（这就是不能用的写法）");
        assert_ne!(resample_bins(&bins, 2), by_index);
        // 边界：0 段 / 空输入 → 空；等长与放大也返回 count 段。
        assert!(resample_bins(&bins, 0).is_empty());
        assert!(resample_bins(&[], 4).is_empty());
        assert_eq!(resample_bins(&bins, 4).len(), 4);
        assert_eq!(resample_bins(&bins, 2).len(), 2);
    }

    /// 最小尺寸：空盒与过小的盒都拒绝；正常尺寸放行。
    #[test]
    fn layout_rejects_empty_and_tiny_boxes()
    {
        let empty = layout_checked(MusicLayoutKind::Wide, Rect::new(0.0, 0.0, 0.0, 0.0), true);
        assert!(empty.is_err(), "空盒必须拒绝");
        assert!(empty.unwrap_err().contains("太小"));
        let too_narrow = layout_checked(MusicLayoutKind::Compact, Rect::new(0.0, 0.0, 2.0, 40.0), true);
        assert!(too_narrow.is_err(), "宽 < 3 必须拒绝");
        let too_short = layout_checked(MusicLayoutKind::Compact, Rect::new(0.0, 0.0, 150.0, 17.0), true);
        assert!(too_short.is_err(), "紧凑高 < 18 必须拒绝");
        let wide_short = layout_checked(MusicLayoutKind::Wide, Rect::new(0.0, 0.0, 396.0, 60.0), true);
        assert!(wide_short.is_err(), "宽布局高 < 61 必须拒绝");
        let ok = layout_checked(MusicLayoutKind::Compact, Rect::new(0.0, 0.0, COMPACT_WIDTH, COMPACT_HEIGHT), true);
        assert!(ok.is_ok(), "168x112 应放行");
        assert_eq!(ok.unwrap().spectrum.height, 18.0);
    }

    /// 四类元素都在内容盒内：文字（标题/艺人）、进度轨道、进度填充、频谱柱。
    #[test]
    fn all_elements_stay_inside_content_box()
    {
        for (kind, content) in [
            (MusicLayoutKind::Wide, Rect::new(10.0, 20.0, WIDE_WIDTH, WIDE_HEIGHT)),
            (MusicLayoutKind::Compact, Rect::new(10.0, 20.0, COMPACT_WIDTH, COMPACT_HEIGHT)),
        ]
        {
            let layout = layout_checked(kind, content, true).expect("正常尺寸应放行");
            for (label, rect) in [("title", layout.title), ("artist", layout.artist), ("progress", layout.progress), ("spectrum", layout.spectrum)]
            {
                assert!(rect.x >= content.x - 0.01 && rect.y >= content.y - 0.01, "{label} 左/上越界：{rect:?}");
                assert!(rect.right() <= content.right() + 0.01, "{label} 右越界：{rect:?}");
                assert!(rect.bottom() <= content.bottom() + 0.01, "{label} 下越界：{rect:?}");
            }
            for bar in spectrum_bars(layout.spectrum, SPECTRUM_BANDS)
            {
                assert!(bar.x >= layout.spectrum.x - 0.01 && bar.right() <= layout.spectrum.right() + 0.01, "柱越界：{bar:?}");
                assert!(bar.bottom() <= layout.spectrum.bottom() + 0.01);
            }
        }
        // 过小盒被拒绝 ⇒ 根本不会产出越界矩形。
        assert!(layout_checked(MusicLayoutKind::Wide, Rect::new(0.0, 0.0, 396.0, 60.0), true).is_err());
    }

    /// 行盒取舍：行盒高 = 字号 13（`2/3` 由相邻行盒之间显式留出 `2 DIP`）。
    #[test]
    fn line_box_equals_font_size()
    {
        let wide = layout(MusicLayoutKind::Wide, Rect::new(0.0, 0.0, WIDE_WIDTH, WIDE_HEIGHT), true);
        assert_eq!(wide.title.height, LINE_HEIGHT, "行盒 = 13（1lh）");
        assert_eq!(wide.artist.y - wide.title.bottom(), 2.0, "行间距 2 DIP 由行盒之间显式留出");
    }
}

/// 一次音乐格诊断的报告（自包含，不读用户数据、不写盘）。
#[derive(Clone, Debug, serde::Serialize)]
pub struct MusicReport
{
    pub pid: u32,
    /// 有曲目：宽/紧凑两套布局的元素矩形与绘制统计。
    pub with_track: Vec<String>,
    /// 无曲目：降级文案与基线。
    pub without_track: Vec<String>,
    pub draws_with_track: usize,
    pub skipped_with_track: usize,
    pub exit_code: i32,
}

/// 跑一次音乐格诊断：只摆数据、算布局、生成绘制列表并统计，不建窗、不接管桌面。
pub fn run_music() -> Result<MusicReport>
{
    let dark = true;
    let text = Color::rgba(0xE8, 0xEF, 0xF0, 0xFF);
    let fence_text = Color::rgba(0xD2, 0xE0, 0xE3, 0xFF);
    let style = MusicStyle { text, fence_text, dark };
    let mut with_track = Vec::new();
    let playing = MusicView { title: "晴天".into(), artist: "周杰伦".into(), playback: "playing".into(), progress: Some(0.42), bins: (0..SPECTRUM_BANDS).map(|index| (index as u8).wrapping_mul(11)).collect(), muted: false, bands: None };
    let mut presenter = MusicPresenter::default();
    for (kind, content) in [
        (MusicLayoutKind::Wide, Rect::new(0.0, 0.0, WIDE_WIDTH, WIDE_HEIGHT)),
        (MusicLayoutKind::Compact, Rect::new(0.0, 0.0, COMPACT_WIDTH, COMPACT_HEIGHT)),
    ]
    {
        let layout = layout_checked(kind, content, true).map_err(|reason| windows::core::Error::new(windows::Win32::Foundation::E_FAIL, reason))?;
        // 只在判等通过时才生成绘制列表（不再在 submit 之前无条件 draw）。
        let list = presenter.submit(&playing, &layout, &style, || draw(&playing, &layout, text, fence_text, dark));
        let changed = list.is_some();
        let commands = list.as_ref().map(|list| list.commands().len()).unwrap_or(0);
        with_track.push(format!(
            "{:?}: content={:.0}x{:.0} artwork=({:.0},{:.0},{:.0}x{:.0}) title=({:.0},{:.0},{:.0}x{:.0}) spectrum=({:.0},{:.0},{:.0}x{:.0}) bars={} commands={} changed={changed}",
            kind,
            content.width,
            content.height,
            layout.artwork.x,
            layout.artwork.y,
            layout.artwork.width,
            layout.artwork.height,
            layout.title.x,
            layout.title.y,
            layout.title.width,
            layout.title.height,
            layout.spectrum.x,
            layout.spectrum.y,
            layout.spectrum.width,
            layout.spectrum.height,
            spectrum_bars(layout.spectrum, SPECTRUM_BANDS).len(),
            commands
        ));
    }
    // 同一份「数据 + 布局」连提两次：第一次绘制、第二次跳过（一次更新 → 一次绘制）。
    let compact_layout = layout_checked(MusicLayoutKind::Compact, Rect::new(0.0, 0.0, COMPACT_WIDTH, COMPACT_HEIGHT), true).map_err(|reason| windows::core::Error::new(windows::Win32::Foundation::E_FAIL, reason))?;
    let drawn_again = presenter.submit(&playing, &compact_layout, &style, || draw(&playing, &compact_layout, text, fence_text, dark)).is_some();
    let skipped_again = presenter.submit(&playing, &compact_layout, &style, || draw(&playing, &compact_layout, text, fence_text, dark)).is_none();
    with_track.push(format!("重复提交同一「数据+布局」：再绘制={drawn_again} 第二次跳过={skipped_again}"));
    let mut without_track = Vec::new();
    let empty = MusicView::default();
    for (kind, content) in [
        (MusicLayoutKind::Wide, Rect::new(0.0, 0.0, WIDE_WIDTH, WIDE_HEIGHT)),
        (MusicLayoutKind::Compact, Rect::new(0.0, 0.0, COMPACT_WIDTH, COMPACT_HEIGHT)),
    ]
    {
        let layout = layout_checked(kind, content, true).map_err(|reason| windows::core::Error::new(windows::Win32::Foundation::E_FAIL, reason))?;
        let list = draw(&empty, &layout, text, fence_text, dark);
        let texts: Vec<String> = list
            .commands()
            .iter()
            .filter_map(|command| match command
            {
                DrawCommand::Text { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect();
        let baseline = list.commands().iter().any(|command| matches!(command, DrawCommand::FillRoundRect { rect, .. } if (rect.height - 1.0).abs() < 0.01));
        without_track.push(format!("{:?}: texts={texts:?} baseline={baseline} commands={}", kind, list.commands().len()));
    }
    Ok(MusicReport {
        pid: std::process::id(),
        with_track,
        without_track,
        draws_with_track: presenter.draws(),
        skipped_with_track: presenter.skipped(),
        exit_code: 0,
    })
}
