//! 诊断首帧的只读内容。读取权威用例，不启动刷新调度，不持有条目正文或凭据。
use super::{
    draw::{Color, DrawCommand, DrawList, Rect, TextAlign},
    host::{DemoReport, run_scene},
    workspace_scene::{config_dir, draw_with_content, read_scene_from},
};
use crate::{
    collect,
    contracts::{WorkspaceSnapshot, settings::ApplicationPreferences},
    weather::{Source, WeatherState},
};
use chrono::{Local, TimeZone, Timelike};
use serde_json::Value;
use std::{collections::HashMap, path::Path, time::Duration};

/// 适配现有墙钟事件 Utc::now() 的本地文本展示；zh-CN 数字、AM/PM 和秒显示与 LocalClock 相同。
fn clock_text<T: TimeZone>(ms: Option<i64>, zone: &T, hour12: bool, seconds: bool) -> String
{
    let Some(t) = ms.filter(|&ms| ms != 0).and_then(|ms| zone.timestamp_millis_opt(ms).single())
    else
    {
        return "--:--:--".into();
    };
    let prefix = if hour12 { if t.hour() < 12 { "上午" } else { "下午" } } else { "" };
    let h = if hour12 { t.hour12().1 } else { t.hour() };
    let mut out = format!("{prefix}{h:02}:{:02}", t.minute());
    if seconds
    {
        out.push_str(&format!(":{:02}", t.second()));
    }
    out
}

/// 已解析的显示字段；绝不把无当前缓存当作实时天气。
#[derive(Debug)]
struct Weather
{
    city: String,
    temperature: String,
    condition: String,
}
impl Weather
{
    fn empty(city: String) -> Self { Self { city, temperature: "--".into(), condition: "当前天气暂不可用".into() } }
    fn read(state: &WeatherState, config: &Value) -> Self
    {
        let Some(source) = config.get("weatherSource").and_then(|v| serde_json::from_value::<Source>(v.clone()).ok())
        else
        {
            return Self::empty("未选择天气来源".into());
        };
        let Some(id) = config["weatherCityId"].as_str()
        else
        {
            return Self::empty("未选择城市".into());
        };
        let Ok(city) = state.validated_city(source, id)
        else
        {
            return Self::empty("城市暂不可用".into());
        };
        let mut w = Self::empty(city.name.clone());
        if let Ok(v) = state.forecast(source, city)
        {
            if let Some(t) = v.pointer("/current/temperature/value").and_then(Value::as_f64)
            {
                w.temperature = format!("{}°", (t + 0.5).floor() as i32);
            }
            if let Some(t) = v.pointer("/current/condition/text").and_then(Value::as_str).filter(|s| !s.is_empty()).or_else(|| v.pointer("/moduleErrors/current").and_then(Value::as_str).filter(|s| !s.is_empty()))
            {
                w.condition = t.into();
            }
        }
        w
    }
}

pub(super) struct Content
{
    clock: String,
    weather: HashMap<String, Weather>,
    collect: String,
}
impl Content
{
    fn read(dir: &Path, s: &WorkspaceSnapshot, p: &ApplicationPreferences) -> Self
    {
        let w = WeatherState::new(dir);
        let weather = s.components.iter().filter(|c| c.component_id == "official.weather").map(|c| (c.instance_id.clone(), Weather::read(&w, &c.state))).collect();
        let count = collect::list_existing(dir).ok().map(|items| items.len());
        let collect = count.map_or_else(|| "收集项暂不可用".into(), |n| format!("{n} 项"));
        Self { clock: clock_text(Some(crate::tools::time::host_unix_millis()), &Local, p.clock_hour12, p.clock_show_seconds), weather, collect }
    }
    /// 每条行框受格子内容内边距与剩余高度双重限制。只读诊断不绘制任何列表项。
    pub(super) fn draw(&self, list: &mut DrawList, component: &str, id: &str, r: Rect, header: f32, color: Color)
    {
        let area = Rect::new(r.x + 10., r.y + header + 6., (r.width - 20.).max(0.), (r.height - header - 12.).max(0.));
        if area.width <= 0. || area.height <= 0.
        {
            return;
        }
        list.push(DrawCommand::PushClip { rect: area });
        let mut row = |label: &str, y: f32, size: f32, weight: u16, align: TextAlign| {
            let h = (area.height - y).min(size * 1.5);
            if h > 0.
            {
                list.push(DrawCommand::Caption { rect: Rect::new(area.x, area.y + y, area.width, h), text: label.into(), size, weight, color, align });
            }
        };
        match component
        {
            "official.local-clock" => row(&self.clock, ((area.height - 30.) / 2.).max(0.), 20., 400, TextAlign::Center),
            "official.collect-box" => row(&self.collect, 0., 13., 400, TextAlign::Start),
            "official.weather" =>
            {
                if let Some(w) = self.weather.get(id)
                {
                    // 摘要上下两行：城市与天气状态各有完整行框；温度在左侧独立占一列。
                    let temp_w = (area.width * 0.4).min(78.);
                    let gap = 8f32.min((area.width - temp_w) / 2.);
                    let txt_x = area.x + temp_w + gap;
                    let txt_w = area.width - temp_w - gap;
                    let line_h = (area.height / 2.).min(19.5);
                    let temp_h = area.height.min(26.);
                    let top = area.y;
                    for (label, rect, size) in [(&w.temperature, Rect::new(area.x, area.y, temp_w, temp_h), 26.), (&w.city, Rect::new(txt_x, top, txt_w, line_h), 13.), (&w.condition, Rect::new(txt_x, top + line_h, txt_w, line_h), 13.)]
                    {
                        list.push(DrawCommand::Caption { rect, text: label.clone(), size, weight: 400, color, align: TextAlign::Start });
                    }
                }
            }
            _ =>
            {}
        }
        list.push(DrawCommand::PopClip);
    }
}

/// 真实快照仅在这个显式入口内启用时钟和展开收集箱；样例分支完全不读取真实数据。
pub fn run_content(example: bool) -> windows::core::Result<DemoReport>
{
    if example
    {
        return run_scene(Duration::from_secs(8), |_, _, _, _| {
            let mut l = DrawList::new();
            let r = Rect::new(36., 36., 164., 90.);
            l.push(DrawCommand::FillRoundRect { rect: r, radius: 4., color: Color(0x202628ff) });
            for (y, label) in [(6., "截断样例 · 非真实天气"), (34., "示例市超长中文地点 · District 123456789 · 城市名称超长截断验证")]
            {
                l.push(DrawCommand::Caption { rect: Rect::new(r.x + 10., r.y + y, r.width - 20., 24.), text: label.into(), size: 13., weight: 400, color: Color(0xe8eff0ff), align: TextAlign::Start });
            }
            l
        }, None);
    }
    let dir = config_dir()?;
    let (mut scene, mut preferences) = read_scene_from(&dir)?;
    let content = Content::read(&dir, &scene, &preferences);
    preferences.clock_enabled = true;
    for c in &scene.components
    {
        if c.component_id == "official.collect-box"
        {
            if let Some(a) = scene.tile_appearances.get_mut(&format!("component:{}", c.instance_id))
            {
                a.collapsed = false;
            }
        }
    }
    let monitors = crate::platform::windows::enumerate_desktop_monitors().map_err(|s| windows::core::Error::new(windows::Win32::Foundation::E_FAIL, s))?;
    let remap = single_screen_projection(&scene, &monitors);
    if let Some((old, new)) = &remap
    {
        eprintln!("内容诊断：已重映射 {old} → {new}，仅内存显示，不更改布局");
    }
    run_scene(Duration::from_secs(8), |g, m, _, _| {
        let id = remap.as_ref().filter(|(_, new)| new == &m.id).map_or(m.id.as_str(), |(old, _)| old.as_str());
        draw_with_content(&scene, &preferences, id, g, Some(&content), None)
    }, None)
}

/// 不推断更换显示器的原因；只有三个目标均存在、同属一个旧屏并且边界可原样绘制时才接受别名。
fn single_screen_projection(s: &WorkspaceSnapshot, monitors: &[crate::contracts::DesktopHostMonitor]) -> Option<(String, String)>
{
    let [m] = monitors
    else
    {
        return None;
    };
    if m.dpi == 0
    {
        return None;
    }
    let (w, h) = (m.work_width as f64 * 96. / m.dpi as f64, m.work_height as f64 * 96. / m.dpi as f64);
    let kinds = ["official.local-clock", "official.weather", "official.collect-box"];
    let mut seen = [false; 3];
    let mut old = None;
    for c in &s.components
    {
        let Some(k) = kinds.iter().position(|&id| id == c.component_id)
        else
        {
            continue;
        };
        seen[k] = true;
        let id = c.projection_monitor_id.as_ref().unwrap_or(&c.monitor_id);
        let r = c.projection_monitor_id.as_ref().and(c.projection_bounds).unwrap_or(c.bounds);
        if id.is_empty() || id == &m.id || old.is_some_and(|o| o != id) || !(r.left >= 0. && r.top >= 0. && r.width > 0. && r.height > 0. && r.left + r.width <= w && r.top + r.height <= h)
        {
            return None;
        }
        old = Some(id);
    }
    if !seen.into_iter().all(|b| b)
    {
        return None;
    }
    Some((old?.clone(), m.id.clone()))
}

#[cfg(test)]
mod tests
{
    use super::*;
    #[test]
    fn diagnostic_projection_rejects_ambiguous_or_offscreen_layout()
    {
        let bounds = serde_json::json!({"left":0,"top":0,"width":200,"height":90});
        let tiles: Vec<_> = ["official.local-clock", "official.weather", "official.collect-box"].into_iter().map(|id| serde_json::json!({"instanceId":id,"componentId":id,"monitorId":"old","bounds":bounds,"stateVersion":1,"state":{}})).collect();
        let mut scene: WorkspaceSnapshot = serde_json::from_value(serde_json::json!({"schemaVersion":1,"revision":1,"fences":[],"components":tiles,"unclassifiedItems":[]})).unwrap();
        let m = crate::contracts::DesktopHostMonitor { id: "new".into(), name: "display".into(), primary: true, x: 0, y: 0, width: 300, height: 190, work_x: 0, work_y: 0, work_width: 300, work_height: 190, dpi: 96, scale_factor: 1. };
        let one = std::slice::from_ref(&m);
        assert!(single_screen_projection(&scene, one).is_some());
        assert!(single_screen_projection(&scene, &[m.clone(), m.clone()]).is_none());
        scene.components[0].bounds.left = 250.;
        assert!(single_screen_projection(&scene, one).is_none());
        scene.components[0].bounds.left = 0.;
        scene.components[0].monitor_id = "elsewhere".into();
        assert!(single_screen_projection(&scene, one).is_none());
        scene.components.remove(0);
        assert!(single_screen_projection(&scene, one).is_none());
    }
    #[test]
    fn clock_matches_shared_intl_fixture()
    {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Case
        {
            name: String,
            ms: Option<i64>,
            hour12: bool,
            seconds: bool,
            zone: String,
            expected: String,
        }
        let cases: Vec<Case> = serde_json::from_str(include_str!("../../../src/features/tile-content/clock/clockText.golden.json")).unwrap();
        for c in cases
        {
            let zone = c.zone.parse::<chrono_tz::Tz>().unwrap();
            assert_eq!(clock_text(c.ms, &zone, c.hour12, c.seconds), c.expected, "{}", c.name);
        }
    }
    /// 人工温度只用于图元契约：最小内容高，窄/宽框都保留城市、温度、状况且不越界。
    #[test]
    fn compact_weather_preserves_status_and_bounds()
    {
        for width in [164., 649.]
        {
            let content = Content { clock: String::new(), collect: String::new(), weather: HashMap::from([("id".into(), Weather { city: "样例，非真实天气".repeat(4), temperature: "23°".into(), condition: "仅供测试".into() })]) };
            let r = Rect::new(0., 0., width, 96.);
            let mut l = DrawList::new();
            content.draw(&mut l, "official.weather", "id", r, 28., Color::TRANSPARENT);
            let DrawCommand::PushClip { rect: clip } = l.commands()[0]
            else
            {
                panic!("missing clip")
            };
            let rows: Vec<_> = l
                .commands()
                .iter()
                .filter_map(|c| {
                    if let DrawCommand::Caption { rect, text, size, weight, .. } = c
                    {
                        Some((rect, text, size, weight))
                    }
                    else
                    {
                        None
                    }
                })
                .collect();
            // 比较真实 DWrite 未裁切文本宽度与该天气图元的行框，而非只看指令条数。
            use windows::{Win32::Graphics::DirectWrite::*, core::w};
            let _apt = super::super::host::ComApartment::enter().unwrap();
            let factory: IDWriteFactory = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED) }.unwrap();
            let format = unsafe { factory.CreateTextFormat(w!("Microsoft YaHei UI"), None, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, 13., w!("zh-CN")) }.unwrap();
            let text: Vec<u16> = rows[1].1.encode_utf16().collect();
            let layout = unsafe { factory.CreateTextLayout(&text, &format, 4096., 100.) }.unwrap();
            let mut metrics = DWRITE_TEXT_METRICS::default();
            unsafe { layout.GetMetrics(&mut metrics) }.unwrap();
            assert_eq!(metrics.width > rows[1].0.width, width == 164., "box={width} text={} available={}", metrics.width, rows[1].0.width);
            assert_eq!(rows.len(), 3);
            for (rect, _, _, _) in &rows
            {
                assert!(rect.width > 0. && rect.height >= 13. && rect.x >= clip.x && rect.y >= clip.y && rect.bottom() <= clip.bottom() && rect.right() <= clip.right());
            }
            assert_eq!((rows[0].0.height, *rows[0].2, *rows[0].3), (26., 26., 400));
            assert_eq!(rows[2].1, "仅供测试");
            assert!(rows[2].0.y >= rows[1].0.bottom());
        }
    }
}
