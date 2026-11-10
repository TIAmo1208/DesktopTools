//! 音乐第二类事实：时间轴与逐句歌词，不参与媒体属性收敛。
use crate::platform::windows::chinese::{to_simplified, to_traditional};
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
static REQUESTED: Mutex<Option<u64>> = Mutex::new(None); // 已发起取词的代次，关闭或丢弃后可重新请求。

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

/// 「显示歌词」关闭时清空缓存并停止取词；状态未就绪时按默认开启处理。
fn lyrics_enabled(app: &tauri::AppHandle) -> bool
{
    use tauri::Manager;
    app.try_state::<crate::app::settings::SettingsState>().and_then(|state| state.preferences().ok()).map(|preferences| preferences.music_lyrics_enabled).unwrap_or(true)
}

/// 每代至多取词一次，网络不阻塞媒体线程；迟到响应不得污染下一首。
pub fn publish(app: &tauri::AppHandle)
{
    if !lyrics_enabled(app)
    {
        // 关闭取词：清空歌词并让在途请求失效，但时间轴仍要持续广播。
        let mut state = details_state().lock().unwrap_or_else(|e| e.into_inner());
        let mut requested = REQUESTED.lock().unwrap_or_else(|e| e.into_inner());
        if !state.lyrics.is_empty() || requested.is_some()
        {
            state.generation += 1; // 迟到响应不得写回关闭后的缓存。
        }
        state.lyrics.clear();
        *requested = None; // 重新开启后允许再次取词。
        drop(requested);
        drop(state);
        let _ = app.emit(EVENT, snapshot());
        return;
    }
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
        if !lyrics_enabled(&app)
        {
            // 关闭期间丢弃的结果必须释放该代请求，重新开启后才会再次取词。
            let mut requested = REQUESTED.lock().unwrap_or_else(|e| e.into_inner());
            if *requested == Some(generation)
            {
                *requested = None;
            }
            return;
        }
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

/// 匹配用标题：统一简体、去掉括号说明与标点空白，忽略大小写。
fn normalize_title(text: &str) -> String
{
    let simplified = to_simplified(text);
    let mut depth = 0u32;
    let mut base = String::new();
    for character in simplified.chars()
    {
        match character
        {
            '(' | '（' | '[' | '【' => depth += 1,
            ')' | '）' | ']' | '】' => depth = depth.saturating_sub(1),
            _ if depth == 0 => base.push(character),
            _ =>
            {}
        }
    }
    base.chars().filter(|character| character.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// 匹配用艺人：统一简体，只保留字母数字，忽略空格与标点。
fn normalize_artist(text: &str) -> String { to_simplified(text).chars().filter(|character| character.is_alphanumeric()).flat_map(char::to_lowercase).collect() }

/// 去重歌词：统一简体并忽略空白。
fn normalize_lyrics(text: &str) -> String { to_simplified(text).chars().filter(|character| !character.is_whitespace()).collect() }

/// 标题归一后相等，艺人归一后相等或互相包含，才视为同一首。
fn candidate_matches(candidate: &Candidate, title: &str, artist: &str) -> bool
{
    let title_matches = normalize_title(&candidate.track_name) == title;
    let candidate_artist = normalize_artist(&candidate.artist_name);
    let artist_matches = candidate_artist == artist || (artist.chars().count() >= 2 && candidate_artist.contains(artist)) || (candidate_artist.chars().count() >= 2 && artist.contains(candidate_artist.as_str()));
    title_matches && artist_matches
}

/// 标题、艺人一致且在 ±2 秒内的候选里取时长最接近者；只有真正并列且歌词一致才接受，不猜版本。
fn match_lyrics(candidates: Vec<Candidate>, title: &str, artist: &str, duration: f64) -> Vec<LyricLine>
{
    if !duration.is_finite()
    {
        return vec![];
    }
    let target_title = normalize_title(title);
    let target_artist = normalize_artist(artist);
    if target_title.is_empty() || target_artist.is_empty()
    {
        return vec![];
    }
    // 先收集合格候选、时间差与用于去重的简体歌词。
    let matches: Vec<(&str, f64, String)> = candidates
        .iter()
        .filter_map(|candidate| {
            if !candidate_matches(candidate, &target_title, &target_artist) || !candidate.duration.is_finite()
            {
                return None;
            }
            let lyrics = candidate.synced_lyrics.as_deref()?;
            let distance = (candidate.duration - duration).abs();
            (distance <= 2.0).then(|| (lyrics, distance, normalize_lyrics(lyrics)))
        })
        .collect();
    // 只接受真正最接近的距离，不引入未声明的容差。
    let Some(best) = matches.iter().map(|(_, distance, _)| *distance).min_by(|a, b| a.partial_cmp(b).unwrap())
    else
    {
        return vec![];
    };
    let mut variants: Vec<(&str, &str)> = matches.iter().filter(|(_, distance, _)| *distance == best).map(|(lyrics, _, normalized)| (*lyrics, normalized.as_str())).collect();
    variants.sort_unstable_by(|a, b| a.1.cmp(b.1));
    variants.dedup_by(|a, b| a.1 == b.1);
    // 唯一最佳或并列候选歌词一致时可用，不同歌词视为多版本保持空态。
    if variants.len() == 1 { parse_lrc(&to_simplified(variants[0].0)) } else { vec![] }
}

/// 查询 LRCLIB 候选，限制不可信响应体，不记录歌词正文。
fn request_candidates(client: &reqwest::blocking::Client, params: &[(&str, &str)]) -> Result<Vec<Candidate>, Box<dyn std::error::Error>>
{
    let response = client.get("https://lrclib.net/api/search").query(params).send()?.error_for_status()?;
    use std::io::Read;
    let mut body = String::new();
    response.take(2 * 1024 * 1024).read_to_string(&mut body)?;
    Ok(serde_json::from_str(&body)?)
}

fn fetch_lyrics(timeline: &Timeline) -> Result<Vec<LyricLine>, Box<dyn std::error::Error>>
{
    if timeline.title.trim().is_empty() || timeline.artist.trim().is_empty() || timeline.duration_ms <= 0.0
    {
        return Ok(vec![]);
    }
    let client = reqwest::blocking::Client::builder().user_agent("DesktopTools/0.4.2 (music lyrics)").timeout(Duration::from_secs(10)).build()?;
    let simplified_title = to_simplified(&timeline.title);
    let simplified_artist = to_simplified(&timeline.artist);
    let traditional_title = to_traditional(&timeline.title);
    let traditional_artist = to_traditional(&timeline.artist);
    let duration = timeline.duration_ms / 1000.0;
    // 播放器可能给简体、歌词库可能存繁体；带艺人与仅曲名都尝试，任一得到同步词即返回。
    for (title, artist) in [(&simplified_title, Some(&simplified_artist)), (&traditional_title, Some(&traditional_artist)), (&simplified_title, None), (&traditional_title, None)]
    {
        let mut params = vec![("track_name", title.as_str())];
        if let Some(artist) = artist
        {
            params.push(("artist_name", artist.as_str()));
        }
        let lines = match_lyrics(request_candidates(&client, &params)?, &timeline.title, &timeline.artist, duration);
        if !lines.is_empty()
        {
            return Ok(lines);
        }
    }
    Ok(vec![])
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

    /// 生成一条指定时长与同步词的候选，默认标题、艺人与待匹配参数一致。
    fn candidate(duration: f64, lyrics: Option<&str>) -> Candidate { Candidate { track_name: "歌".into(), artist_name: "人".into(), duration, synced_lyrics: lyrics.map(str::to_string) } }

    #[test]
    fn music_lyrics_match_simplified_and_traditional()
    {
        assert_eq!(to_simplified("後會無期"), "后会无期");
        assert_eq!(to_traditional("后会无期"), "後會無期");
        // 播放器简体、歌词库繁体时仍能匹配，并把歌词统一为简体。
        let candidate = Candidate { track_name: "後會無期".into(), artist_name: "G.E.M. 鄧紫棋".into(), duration: 223.0, synced_lyrics: Some("[00:07.91] 當一艘船沉入海底".into()) };
        let lines = match_lyrics(vec![candidate], "后会无期", "G.E.M.邓紫棋", 223.015);
        assert_eq!(lines, vec![LyricLine { start_ms: 7910, text: "当一艘船沉入海底".into() }]);
        // 括号说明不参与标题匹配，艺人为包含关系。
        let suffixed = Candidate { track_name: "後會無期 (電影同名主題歌)".into(), artist_name: "鄧紫棋".into(), duration: 223.04, synced_lyrics: Some("[00:07.89] 當一艘船沉入海底".into()) };
        assert_eq!(match_lyrics(vec![suffixed], "后会无期", "G.E.M.邓紫棋", 223.0).len(), 1);
        // 不同歌手的同名词不匹配。
        let other = Candidate { track_name: "後會無期".into(), artist_name: "孫露".into(), duration: 223.0, synced_lyrics: Some("[00:01] 詞".into()) };
        assert!(match_lyrics(vec![other], "后会无期", "G.E.M.邓紫棋", 223.0).is_empty());
    }

    #[test]
    fn music_lrc_and_matching()
    {
        let lines = parse_lrc("无标签\n[00:02.50][00:01]甲\n[00:01.000]乙\n[00:99]错\n[ar:名字]元信息");
        assert_eq!(lines, vec![LyricLine { start_ms: 1000, text: "甲 / 乙".into() }, LyricLine { start_ms: 2500, text: "甲".into() }]);
        // 四条候选都落在 ±2 秒内时取时长最接近的一条，而不是直接空态。
        let lines = match_lyrics(vec![candidate(298.840816, Some("[00:01]甲")), candidate(297.24, Some("[00:02]乙")), candidate(297.0, Some("[00:03]丙")), candidate(296.7158329, Some("[00:04]丁"))], "歌", "人", 297.24);
        assert_eq!(lines, vec![LyricLine { start_ms: 2000, text: "乙".into() }]);
        // ±2 秒容差边界可用，超出容差、无同步词与非有限时长不参与。
        assert_eq!(match_lyrics(vec![candidate(102.0, Some("[00:01]词"))], "歌", "人", 100.0).len(), 1);
        assert!(match_lyrics(vec![candidate(102.01, Some("[00:01]词"))], "歌", "人", 100.0).is_empty());
        assert!(match_lyrics(vec![candidate(100.0, None)], "歌", "人", 100.0).is_empty());
        assert!(match_lyrics(vec![candidate(f64::NAN, Some("[00:01]词"))], "歌", "人", 100.0).is_empty());
        // 标题或艺人不一致时不猜测同名版本。
        assert!(match_lyrics(vec![Candidate { track_name: "另一首".into(), ..candidate(100.0, Some("[00:01]词")) }], "歌", "人", 100.0).is_empty());
        assert!(match_lyrics(vec![Candidate { artist_name: "另一人".into(), ..candidate(100.0, Some("[00:01]词")) }], "歌", "人", 100.0).is_empty());
        // 同一距离的不同歌词视为多版本，保持空态；一致歌词没有分歧，可以接受。
        assert!(match_lyrics(vec![candidate(100.0, Some("[00:01]甲")), candidate(100.0, Some("[00:02]乙"))], "歌", "人", 100.0).is_empty());
        assert_eq!(match_lyrics(vec![candidate(100.0, Some("[00:01]甲")), candidate(100.0, Some("[00:01]甲"))], "歌", "人", 100.0).len(), 1);
        // 接近但不相等的时间差不算并列，按真正最接近者取词，且结果与候选顺序无关。
        let near = match_lyrics(vec![candidate(100.005, Some("[00:01]甲")), candidate(100.0, Some("[00:02]乙"))], "歌", "人", 100.0);
        assert_eq!(near, vec![LyricLine { start_ms: 2000, text: "乙".into() }]);
        let reversed = match_lyrics(vec![candidate(100.0, Some("[00:02]乙")), candidate(100.005, Some("[00:01]甲"))], "歌", "人", 100.0);
        assert_eq!(reversed, near);
    }
}
