//! 音乐第二类事实：时间轴与逐句歌词，不参与媒体属性收敛。
use serde::{Deserialize, Serialize};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::Emitter;

pub const EVENT: &str = "music-timeline-changed";

/// 原始 ticks 用十进制字符串保留精度；投影与时长使用毫秒。
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Timeline
{
    pub session_id: String,
    pub title: String,
    pub artist: String,
    pub position_ticks: String,
    pub end_time_ticks: String,
    pub last_updated_time_ticks: String,
    pub playback_status: i32,
    pub position_ms: f64,
    pub duration_ms: f64,
    pub advancing: bool,
}

/// 同一时间戳的歌词合并为一行。
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LyricLine
{
    pub start_ms: u64,
    pub text: String,
}

#[derive(Default)]
struct Details
{
    timeline: Option<Timeline>,
    sampled: Option<Instant>,
    key: String,
    generation: u64,
    lyrics: Vec<LyricLine>,
}
static DETAILS: OnceLock<Mutex<Details>> = OnceLock::new();
fn details_state() -> &'static Mutex<Details> { DETAILS.get_or_init(|| Mutex::new(Details::default())) }

/// 读取缓存时重投影，恢复页面不会从过期的位置继续。
pub fn snapshot() -> serde_json::Value
{
    let state = details_state().lock().unwrap_or_else(|e| e.into_inner());
    let mut timeline = state.timeline.clone();
    if let Some(value) = &mut timeline
    {
        if value.advancing
        {
            value.position_ms = (value.position_ms + state.sampled.map(|v| v.elapsed().as_secs_f64() * 1000.0).unwrap_or_default()).min(value.duration_ms);
        }
    }
    serde_json::json!({ "timeline": timeline, "lyrics": state.lyrics })
}

/// 读取线程写入新锚点，切歌立即清除旧歌词并递增请求代次。
pub fn update(timeline: Option<Timeline>)
{
    let key = timeline.as_ref().map(|v| format!("{}\0{}\0{}\0{}", v.session_id, v.title, v.artist, v.end_time_ticks)).unwrap_or_default();
    let mut state = details_state().lock().unwrap_or_else(|e| e.into_inner());
    if state.key != key
    {
        state.generation += 1;
        state.key = key;
        state.lyrics.clear();
    }
    state.timeline = timeline;
    state.sampled = Some(Instant::now());
}

/// 每代至多取词一次，网络不阻塞媒体线程；迟到响应不得污染下一首。
pub fn publish(app: &tauri::AppHandle)
{
    static REQUESTED: Mutex<Option<u64>> = Mutex::new(None);
    let _ = app.emit(EVENT, snapshot());
    let state = details_state().lock().unwrap_or_else(|e| e.into_inner());
    let Some(timeline) = state.timeline.clone()
    else
    {
        return;
    };
    let generation = state.generation;
    let mut requested = REQUESTED.lock().unwrap_or_else(|e| e.into_inner());
    if *requested == Some(generation)
    {
        return;
    }
    *requested = Some(generation);
    drop(state);
    let app = app.clone();
    std::thread::spawn(move || {
        let lyrics = fetch_lyrics(&timeline).unwrap_or_default();
        let mut state = details_state().lock().unwrap_or_else(|e| e.into_inner());
        if state.generation != generation
        {
            return;
        }
        state.lyrics = lyrics;
        drop(state);
        let _ = app.emit(EVENT, snapshot());
    });
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Candidate
{
    track_name: String,
    artist_name: String,
    duration: f64,
    synced_lyrics: Option<String>,
}

/// 标题、艺人及 ±2 秒内唯一版本才可同步，不猜测多版本。
fn match_lyrics(candidates: Vec<Candidate>, title: &str, artist: &str, duration: f64) -> Vec<LyricLine>
{
    let mut matches = candidates.into_iter().filter(|v| v.track_name.trim().eq_ignore_ascii_case(title.trim()) && v.artist_name.trim().eq_ignore_ascii_case(artist.trim()) && v.duration.is_finite() && (v.duration - duration).abs() <= 2.0);
    let first = matches.next();
    if matches.next().is_some()
    {
        return vec![];
    }
    first.and_then(|v| v.synced_lyrics).map(|v| parse_lrc(&v)).unwrap_or_default()
}
fn fetch_lyrics(timeline: &Timeline) -> Result<Vec<LyricLine>, Box<dyn std::error::Error>>
{
    if timeline.title.trim().is_empty() || timeline.artist.trim().is_empty() || timeline.duration_ms <= 0.0
    {
        return Ok(vec![]);
    }
    let client = reqwest::blocking::Client::builder().user_agent("DesktopTools/0.4.2 (music lyrics)").timeout(Duration::from_secs(10)).build()?;
    let response = client.get("https://lrclib.net/api/search").query(&[("track_name", &timeline.title), ("artist_name", &timeline.artist)]).send()?.error_for_status()?;
    // 限制不可信响应体，不记录歌词正文。
    use std::io::Read;
    let mut body = String::new();
    response.take(2 * 1024 * 1024).read_to_string(&mut body)?;
    Ok(match_lyrics(serde_json::from_str(&body)?, &timeline.title, &timeline.artist, timeline.duration_ms / 1000.0))
}

/// 支持多标签与小数秒，忽略元信息、无标签行与非法时间。
fn parse_lrc(text: &str) -> Vec<LyricLine>
{
    let mut lines = std::collections::BTreeMap::<u64, Vec<String>>::new();
    for line in text.lines()
    {
        let mut rest = line.trim();
        let mut times = vec![];
        while let Some(tag) = rest.strip_prefix('[').and_then(|v| v.split_once(']'))
        {
            if let Some((minutes, seconds)) = tag.0.split_once(':')
            {
                if let (Ok(minutes), Ok(seconds)) = (minutes.parse::<u32>(), seconds.parse::<f64>())
                {
                    if seconds.is_finite() && (0.0..60.0).contains(&seconds)
                    {
                        times.push((minutes as u64 * 60_000) + (seconds * 1000.0).round() as u64);
                    }
                }
            }
            rest = tag.1;
        }
        let text = rest.trim();
        if text.is_empty()
        {
            continue;
        }
        for time in times
        {
            let group = lines.entry(time).or_default();
            if !group.iter().any(|v| v == text)
            {
                group.push(text.to_string());
            }
        }
    }
    lines.into_iter().map(|(start_ms, text)| LyricLine { start_ms, text: text.join(" / ") }).collect()
}

#[cfg(test)]
mod tests
{
    use super::*;
    #[test]
    fn music_lrc_and_matching()
    {
        let lines = parse_lrc("无标签\n[00:02.50][00:01]甲\n[00:01.000]乙\n[00:99]错\n[ar:名字]元信息");
        assert_eq!(lines, vec![LyricLine { start_ms: 1000, text: "甲 / 乙".into() }, LyricLine { start_ms: 2500, text: "甲".into() }]);
        let candidate = |duration| Candidate { track_name: "歌".into(), artist_name: "人".into(), duration, synced_lyrics: Some("[00:01]词".into()) };
        assert_eq!(match_lyrics(vec![candidate(102.0)], "歌", "人", 100.0).len(), 1);
        assert!(match_lyrics(vec![candidate(102.01)], "歌", "人", 100.0).is_empty());
        assert!(match_lyrics(vec![candidate(100.0), candidate(101.0)], "歌", "人", 100.0).is_empty());
    }
}
