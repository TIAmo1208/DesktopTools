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

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Candidate
{
    track_name: String,
    artist_name: String,
    duration: f64,
    synced_lyrics: Option<String>,
}

/// 版本标识：出现在括号说明或破折号副标题里时不能丢弃，否则会与录音室版混淆。
fn has_version_marker(text: &str) -> bool
{
    let simplified = to_simplified(text).to_lowercase();
    let words: Vec<&str> = simplified.split(|character: char| !character.is_alphanumeric()).filter(|word| !word.is_empty()).collect(); // 英文按整词判定，避免 delivery 命中 live。
    let english = ["live", "remix", "instrumental", "inst", "acoustic", "karaoke", "demo", "cover", "extended", "radio", "edit", "remaster", "remastered"]; // 改变演唱或时长，不能与录音室版互换。
    let chinese = ["现场", "演唱会", "伴奏", "纯音乐", "混音", "翻唱", "抖音", "加速", "降速", "女声", "男声", "片段", "试听"];
    english.iter().any(|marker| words.contains(marker)) || chinese.iter().any(|marker| simplified.contains(marker))
}

/// 明确的描述性副标题：影视、动画、游戏出处等，不承载版本差异，可以剥离。
fn is_descriptive_segment(text: &str) -> bool
{
    let simplified = to_simplified(text).to_lowercase();
    let words: Vec<&str> = simplified.split(|character: char| !character.is_alphanumeric()).filter(|word| !word.is_empty()).collect();
    let chinese = ["电视剧", "片尾曲", "片头曲", "主题曲", "主题歌", "插曲", "推广曲", "宣传曲", "电影", "剧场版", "原声", "原声带", "动画", "游戏", "预告", "片花", "角色歌", "印象曲"];
    let english = ["soundtrack", "ost", "theme"];
    chinese.iter().any(|marker| simplified.contains(marker)) || english.iter().any(|marker| words.contains(marker))
}

/// 按破折号切分标题；全角/长破折号直接分隔，半角连字符只在至少一侧有空白时才算分隔。
fn title_segments(text: &str) -> Vec<String>
{
    let characters: Vec<char> = text.chars().collect();
    let mut segments = Vec::new();
    let mut current = String::new();
    for (index, character) in characters.iter().enumerate()
    {
        let separator = matches!(character, '－' | '—' | '–') || (*character == '-' && ((index > 0 && characters[index - 1].is_whitespace()) || characters.get(index + 1).is_some_and(|next| next.is_whitespace())));
        if separator
        {
            segments.push(current.trim().to_string());
            current.clear();
        }
        else
        {
            current.push(*character);
        }
    }
    segments.push(current.trim().to_string());
    segments
}

/// 剥离副标题：`keep_version` 为真时只丢明确描述片段，版本标识与未知后缀都保留，避免把不同版次折叠。
fn strip_title_suffix(text: &str, keep_version: bool) -> String
{
    let segments: Vec<String> = title_segments(text).into_iter().filter(|segment| segment.chars().any(char::is_alphanumeric)).collect();
    if !keep_version
    {
        return segments.into_iter().next().unwrap_or_else(|| text.trim().to_string()); // 召回只取主标题。
    }
    let mut kept: Vec<String> = Vec::new();
    for segment in segments
    {
        if kept.is_empty()
        {
            kept.push(segment);
            continue;
        }
        if has_version_marker(&segment) || !is_descriptive_segment(&segment)
        {
            kept.push(segment); // 版本与未知后缀都保留，不猜它们与主标题等价。
        }
    }
    if kept.is_empty() { text.trim().to_string() } else { kept.join(" ") }
}

/// 匹配用标题：统一简体，去掉非版本括号说明与破折号副标题，保留版本标识，忽略标点空白。
fn normalize_title(text: &str) -> String
{
    let simplified = to_simplified(text);
    let mut depth = 0u32;
    let mut base = String::new();
    let mut bracket = String::new();
    for character in simplified.chars()
    {
        match character
        {
            '(' | '（' | '[' | '【' =>
            {
                depth += 1;
                if depth == 1
                {
                    bracket.clear();
                }
            }
            ')' | '）' | ']' | '】' =>
            {
                if depth == 1 && has_version_marker(&bracket)
                {
                    base.push(' ');
                    base.push_str(&bracket); // 版本标识参与匹配，不能随括号一起丢弃。
                }
                depth = depth.saturating_sub(1);
            }
            _ if depth > 0 => bracket.push(character),
            _ => base.push(character),
        }
    }
    strip_title_suffix(&base, true).chars().filter(|character| character.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// 召回用主标题：无条件去掉括号说明与破折号副标题，只提高查询命中，不参与版本判定。
fn query_title_stripped(text: &str) -> String
{
    let mut depth = 0u32;
    let mut base = String::new();
    for character in text.chars()
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
    strip_title_suffix(&base, false).trim().to_string()
}

/// 查询用标题：先统一简体再取主标题。
fn query_title(text: &str) -> String { query_title_stripped(&to_simplified(text)) }

/// 匹配用艺人：统一简体，只保留字母数字，忽略空格与标点。
fn normalize_artist(text: &str) -> String { to_simplified(text).chars().filter(|character| character.is_alphanumeric()).flat_map(char::to_lowercase).collect() }

/// 去重歌词：统一简体并忽略空白。
fn normalize_lyrics(text: &str) -> String { to_simplified(text).chars().filter(|character| !character.is_whitespace()).collect() }

/// 标题归一后相等；无时长时艺人必须严格相等，有时长时允许相等或互相包含。
fn candidate_matches(candidate: &Candidate, title: &str, artist: &str, strict_artist: bool) -> bool
{
    if normalize_title(&candidate.track_name) != title
    {
        return false;
    }
    let candidate_artist = normalize_artist(&candidate.artist_name);
    if candidate_artist == artist
    {
        return true;
    }
    if strict_artist
    {
        return false; // 没有时长时不再靠包含关系猜艺人。
    }
    (artist.chars().count() >= 2 && candidate_artist.contains(artist)) || (candidate_artist.chars().count() >= 2 && artist.contains(candidate_artist.as_str()))
}

/// 标题、艺人一致时优先取 ±2 秒内时长最接近者；播放器未提供时长时只接受唯一歌词，不猜版本。
fn match_lyrics(candidates: Vec<Candidate>, title: &str, artist: &str, duration: f64) -> Vec<LyricLine>
{
    let target_title = normalize_title(title);
    let target_artist = normalize_artist(artist);
    if target_title.is_empty() || target_artist.is_empty()
    {
        return vec![];
    }
    // 部分播放器（如网易云）不填 EndTime，时长不可用；此时没有版本判据，只有解析后的歌词结构唯一才接受。
    if !duration.is_finite() || duration <= 0.0
    {
        let mut accepted: Option<Vec<LyricLine>> = None; // 首个非空解析结果，后续必须与它完全一致。
        for candidate in candidates.iter().filter(|candidate| candidate_matches(candidate, &target_title, &target_artist, true))
        {
            let Some(lyrics) = candidate.synced_lyrics.as_deref()
            else
            {
                continue;
            };
            let lines = parse_lrc(&to_simplified(lyrics));
            if lines.is_empty()
            {
                continue;
            }
            match &accepted
            {
                None => accepted = Some(lines),
                Some(first) if *first == lines =>
                {}
                Some(_) => return vec![], // 已知冲突，不猜版本。
            }
        }
        return accepted.unwrap_or_default();
    }
    // 先收集合格候选、时间差与用于去重的简体歌词。
    let matches: Vec<(&str, f64, String)> = candidates
        .iter()
        .filter_map(|candidate| {
            if !candidate_matches(candidate, &target_title, &target_artist, false) || !candidate.duration.is_finite()
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
    if timeline.title.trim().is_empty() || timeline.artist.trim().is_empty()
    {
        return Ok(vec![]);
    }
    let client = reqwest::blocking::Client::builder().user_agent("DesktopTools/0.4.2 (music lyrics)").timeout(Duration::from_secs(10)).build()?;
    let simplified_title = to_simplified(&timeline.title);
    let simplified_artist = to_simplified(&timeline.artist);
    let traditional_title = to_traditional(&timeline.title);
    let traditional_artist = to_traditional(&timeline.artist);
    let duration = timeline.duration_ms / 1000.0;
    let has_duration = duration.is_finite() && duration > 0.0;
    // 播放器标题可能带副标题：主标题与原始标题都查询，去重后最多 6 个查询。
    let mut queries: Vec<(String, Option<String>)> = Vec::new();
    for (title, artist) in [
        (query_title_stripped(&simplified_title), Some(simplified_artist.clone())),
        (simplified_title.clone(), Some(simplified_artist.clone())),
        (query_title_stripped(&traditional_title), Some(traditional_artist.clone())),
        (traditional_title.clone(), Some(traditional_artist.clone())),
        (query_title_stripped(&simplified_title), None),
        (query_title_stripped(&traditional_title), None),
    ]
    {
        if title.trim().is_empty()
        {
            continue;
        }
        let query = (title, artist);
        if !queries.contains(&query)
        {
            queries.push(query); // 同一查询不重复请求。
        }
    }
    // 有时长：任一查询得到同步词即可返回，时长负责版本判定。
    if has_duration
    {
        for (title, artist) in &queries
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
        return Ok(vec![]);
    }
    // 无时长：先合并全部查询的候选，再统一判唯一，避免“先到的唯一”掩盖后续查询里的冲突。
    let mut candidates = Vec::new();
    for (title, artist) in &queries
    {
        let mut params = vec![("track_name", title.as_str())];
        if let Some(artist) = artist
        {
            params.push(("artist_name", artist.as_str()));
        }
        candidates.extend(request_candidates(&client, &params)?);
    }
    Ok(match_lyrics(candidates, &timeline.title, &timeline.artist, 0.0))
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
        // 不同歌手同名的歌曲不匹配。
        let other = Candidate { track_name: "後會無期".into(), artist_name: "孫露".into(), duration: 223.0, synced_lyrics: Some("[00:01] 詞".into()) };
        assert!(match_lyrics(vec![other], "后会无期", "G.E.M.邓紫棋", 223.0).is_empty());
    }

    #[test]
    fn music_lyrics_strip_dash_suffix_without_cutting_hyphenated_titles()
    {
        // 播放器把副标题/影视说明接在破折号后，仍按主标题匹配。
        let suffixed = Candidate { track_name: "後會無期 - 電影《後會無期》片尾曲".into(), artist_name: "鄧紫棋".into(), duration: 223.0, synced_lyrics: Some("[00:07.9] 當一艘船沉入海底".into()) };
        assert_eq!(match_lyrics(vec![suffixed], "后会无期", "G.E.M.邓紫棋", 223.0).len(), 1);
        // 半角连字符只在有空白时视为分隔，曲名中的连字符不切断。
        // 半角连字符只在有空白时视为分隔，曲名中的连字符不切断。
        assert_eq!(title_segments("G-Eazy - Me, Myself & I")[0], "G-Eazy");
        assert_eq!(normalize_title("G-Eazy"), "geazy");
    }

    #[test]
    fn music_title_normalization_keeps_versions_and_ignores_leading_dashes()
    {
        // 版本标识保留：Live/Remix/伴奏等不能与录音室版合并。
        assert_ne!(normalize_title("晴天 (Live)"), normalize_title("晴天"));
        assert_ne!(normalize_title("晴天 - Live"), normalize_title("晴天"));
        assert_eq!(normalize_title("晴天 (Live)"), normalize_title("晴天 - Live"));
        assert_ne!(normalize_title("晴天 (伴奏)"), normalize_title("晴天"));
        // 非版本括号说明与副标题仍被去掉。
        assert_eq!(normalize_title("晴天 (电视剧片尾曲)"), normalize_title("晴天"));
        // 未列出的版本后缀保留，不能被当作描述性副标题折叠。
        assert_ne!(normalize_title("晴天 - Radio Edit"), normalize_title("晴天"));
        // 多段副标题逐段判断：描述段剥离，版本段保留。
        assert_eq!(normalize_title("晴天 - Live - 电影片尾曲"), normalize_title("晴天 - Live"));
        assert_eq!(normalize_title("晴天 - 电视剧片尾曲"), normalize_title("晴天"));
        // 重制版也可能改变时长，归为版本标识；括号与破折号形式都保留。
        assert_ne!(normalize_title("晴天 - Remastered 2021"), normalize_title("晴天"));
        assert_ne!(normalize_title("晴天 (Remastered 2021)"), normalize_title("晴天"));
        // 前导或连续破折号没有主标题可截，不能把标题截成空。
        assert_eq!(normalize_title("-- Song"), "song");
        assert_eq!(normalize_title("——Song"), "song");
        // 查询用主标题无条件剥离副标题，仅用于召回。
        assert_eq!(query_title("晴天 - 电视剧片尾曲"), "晴天");
        assert_eq!(query_title("G-Eazy - Me, Myself & I"), "G-Eazy");
    }

    #[test]
    fn music_no_duration_requires_exact_artist_and_version()
    {
        // 无时长不再靠艺人包含关系猜；有时长时同一候选仍可接受。
        let other_artist = Candidate { track_name: "歌".into(), artist_name: "张杰伦".into(), duration: 100.0, synced_lyrics: Some("[00:01]甲".into()) };
        assert!(match_lyrics(vec![other_artist.clone()], "歌", "张杰", 0.0).is_empty());
        assert_eq!(match_lyrics(vec![other_artist], "歌", "张杰", 100.0).len(), 1);
        // 版本不同不共用歌词：现场版目标不接受录音室版同步词。
        let studio = Candidate { track_name: "晴天".into(), artist_name: "周杰伦".into(), duration: 0.0, synced_lyrics: Some("[00:01]甲".into()) };
        assert!(match_lyrics(vec![studio.clone()], "晴天 (Live)", "周杰伦", 0.0).is_empty());
        // 重制版同样归为版本标识：无时长不能拿录音室版歌词顶替。
        assert!(match_lyrics(vec![studio], "晴天 - Remastered 2021", "周杰伦", 0.0).is_empty());
    }

    #[test]
    fn music_no_duration_compares_parsed_lyrics()
    {
        // 去空白后文本相同，但一份是两行、一份是一行带时间戳正文，不能当作同一份歌词。
        assert!(match_lyrics(vec![candidate(0.0, Some("[00:01]甲\n[00:02]乙")), candidate(0.0, Some("[00:01]甲[00:02]乙"))], "歌", "人", 0.0).is_empty());
        // 结构完全相同的重复候选仍是唯一。
        assert_eq!(match_lyrics(vec![candidate(0.0, Some("[00:01]甲")), candidate(0.0, Some("[00:01]甲"))], "歌", "人", 0.0).len(), 1);
        // 控制字符不再充当结构键：同样的可见文本、不同的行结构仍判为冲突。
        assert!(match_lyrics(vec![candidate(0.0, Some("[00:01]甲\n[00:02]乙")), candidate(0.0, Some("[00:01]甲\u{1}2000:乙"))], "歌", "人", 0.0).is_empty());
    }

    #[test]
    fn music_lyrics_without_duration_require_unique_lyrics()
    {
        // 播放器不填时长（如网易云）时，标题与艺人一致且歌词唯一才接受。
        assert_eq!(match_lyrics(vec![candidate(0.0, Some("[00:01]唯一"))], "歌", "人", 0.0).len(), 1);
        assert!(match_lyrics(vec![candidate(0.0, Some("[00:01]甲")), candidate(0.0, Some("[00:02]乙"))], "歌", "人", 0.0).is_empty());
        // 多版本歌词一致时没有分歧，可以接受。
        assert_eq!(match_lyrics(vec![candidate(0.0, Some("[00:01]甲")), candidate(0.0, Some("[00:01]甲"))], "歌", "人", 0.0).len(), 1);
        // 非有限时长同样走无时长分支；无同步词仍不参与。
        assert_eq!(match_lyrics(vec![candidate(f64::NAN, Some("[00:01]唯一"))], "歌", "人", f64::NAN).len(), 1);
        assert!(match_lyrics(vec![candidate(0.0, None)], "歌", "人", 0.0).is_empty());
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
