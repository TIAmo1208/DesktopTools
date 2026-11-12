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
    reported_position_ms: Option<f64>, // 上一事件上报的公开 Position，用于识别零时间轴跳转。
    source: String,                    // 当前会话的播放源身份，歌词源按它切换。
    derived_duration_ms: Option<f64>,  // 歌词源给出的时长，仅补播放器不报时长时的量程。
}
static DETAILS: OnceLock<Mutex<Details>> = OnceLock::new();
fn details_state() -> &'static Mutex<Details> { DETAILS.get_or_init(|| Mutex::new(Details::default())) }
static REQUESTED: Mutex<Option<u64>> = Mutex::new(None); // 已发起取词的代次，关闭或丢弃后可重新请求。

/// 时间轴缺失：网易云等只发元数据的播放器把 end 与 LastUpdatedTime 都报为 0。
fn zero_timeline(end_time_ticks: &str, last_updated_time_ticks: &str) -> bool { end_time_ticks == "0" && last_updated_time_ticks == "0" }

/// 按锚点投影位置：时长未知（0）时不夹紧，避免零时长把播放位置钉在 0；已知时长仍夹在轨道内。
fn projected_position(position_ms: f64, duration_ms: f64, elapsed_ms: f64, advancing: bool) -> f64
{
    let value = position_ms + if advancing { elapsed_ms.max(0.0) } else { 0.0 };
    if duration_ms > 0.0 { value.clamp(0.0, duration_ms) } else { value.max(0.0) }
}

/// 零时间轴在事件间延续本地外推锚点：公开 Position 变化视为跳转锚点（含跳回 0）；暂停冻结在投影位置，恢复从冻结位置继续；切歌或真实时间轴直接采用上报值。
fn carry_zero_timeline(previous: Option<&Timeline>, previous_reported_ms: Option<f64>, elapsed_ms: f64, mut incoming: Timeline, changed: bool) -> Timeline
{
    if changed || !zero_timeline(&incoming.end_time_ticks, &incoming.last_updated_time_ticks)
    {
        return incoming;
    }
    // 网易云等 Position 恒 0 的播放器无法检测拖动跳转；只有相对上一事件变化时才采纳为锚点。
    if let Some(previous_reported_ms) = previous_reported_ms
    {
        if (incoming.position_ms - previous_reported_ms).abs() > 0.5
        {
            return incoming;
        }
    }
    if let Some(previous) = previous
    {
        incoming.position_ms = projected_position(previous.position_ms, previous.duration_ms, elapsed_ms, previous.advancing);
    }
    incoming
}

/// 读取缓存时重投影，恢复页面不会从过期的位置继续。
pub fn snapshot() -> serde_json::Value
{
    let state = details_state().lock().unwrap_or_else(|e| e.into_inner());
    let mut timeline = state.timeline.clone();
    if let Some(value) = &mut timeline
    {
        value.duration_ms = effective_duration(value.duration_ms, state.derived_duration_ms);
        let elapsed = state.sampled.map(|v| v.elapsed().as_secs_f64() * 1000.0).unwrap_or_default();
        value.position_ms = projected_position(value.position_ms, value.duration_ms, elapsed, value.advancing);
    }
    serde_json::json!({ "timeline": timeline, "lyrics": state.lyrics })
}

/// 读取线程写入新锚点，切歌立即清除旧歌词并递增请求代次。
pub fn update(timeline: Option<Timeline>, source: String)
{
    let key = timeline.as_ref().map(|v| format!("{}\0{}\0{}\0{}\0{}", source, v.session_id, v.title, v.artist, v.end_time_ticks)).unwrap_or_default();
    let mut state = details_state().lock().unwrap_or_else(|e| e.into_inner());
    let changed = state.key != key;
    let elapsed = state.sampled.map(|v| v.elapsed().as_secs_f64() * 1000.0).unwrap_or_default();
    let previous = state.timeline.clone();
    let previous_reported = state.reported_position_ms;
    let reported = timeline.as_ref().map(|value| value.position_ms);
    if changed
    {
        state.generation += 1;
        state.key = key;
        state.lyrics.clear();
        state.derived_duration_ms = None;
    }
    state.timeline = timeline.map(|incoming| carry_zero_timeline(previous.as_ref(), previous_reported, elapsed, incoming, changed));
    state.reported_position_ms = reported;
    state.source = source;
    state.sampled = Some(Instant::now());
}

/// 「显示歌词」关闭时清空缓存并停止取词；状态未就绪时按默认开启处理。
fn lyrics_enabled(app: &tauri::AppHandle) -> bool
{
    use tauri::Manager;
    app.try_state::<crate::app::settings::SettingsState>().and_then(|state| state.preferences().ok()).map(|preferences| preferences.music_lyrics_enabled).unwrap_or(true)
}

/// 一次调试轮询的结果：不适用、读到位置、读取失败。
#[derive(Clone, Copy, PartialEq)]
enum CdpPoll
{
    Skipped,
    Read,
    Failed,
}

/// 网易云调试端口轮询：仅在网易云零时间轴会话生效；成功以真实位置重锚并推送，失败保持本地外推。
fn poll_netease_position(app: &tauri::AppHandle) -> CdpPoll
{
    let (source, zero_timeline_session) = {
        let state = details_state().lock().unwrap_or_else(|e| e.into_inner());
        (state.source.clone(), state.timeline.as_ref().is_some_and(|timeline| timeline.end_time_ticks == "0" && timeline.last_updated_time_ticks == "0"))
    };
    if !is_netease_source(&source) || !zero_timeline_session
    {
        return CdpPoll::Skipped; // 不是网易云零时间轴会话，不进入退避。
    }
    let Some(reading) = crate::platform::windows::netease_cdp::read_position()
    else
    {
        return CdpPoll::Failed; // 端口未开或页面未就绪，交给本地锚点继续。
    };
    let mut state = details_state().lock().unwrap_or_else(|e| e.into_inner());
    if state.source != source
    {
        return CdpPoll::Read;
    }
    let details = &mut *state; // 拆成字段借用，位置与时长可以同时读写。
    let Some(timeline) = details.timeline.as_mut()
    else
    {
        return CdpPoll::Read;
    };
    // 暂停或页面位置未变时不重复广播；时长变化同样需要推送。
    let position_changed = (timeline.position_ms - reading.position_ms).abs() >= 20.0;
    let duration_changed = reading.duration_ms > 0.0 && (details.derived_duration_ms.unwrap_or(0.0) - reading.duration_ms).abs() >= 20.0;
    if !position_changed && !duration_changed
    {
        return CdpPoll::Read;
    }
    timeline.position_ms = reading.position_ms;
    if reading.duration_ms > 0.0
    {
        details.derived_duration_ms = Some(reading.duration_ms);
    }
    details.sampled = Some(Instant::now()); // 以调试端口读数为新锚点，前端继续本地投影。
    drop(state);
    let _ = app.emit(EVENT, snapshot());
    CdpPoll::Read
}

/// 后台轮询调试端口，前端只被动接收推送；端口长期未开时退避到五秒。
pub fn start_cdp_poller(app: &tauri::AppHandle)
{
    static STARTED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    if STARTED.set(()).is_err()
    {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let mut failures = 0u32;
        let mut last: Option<CdpPoll> = None;
        loop
        {
            std::thread::sleep(if failures >= 3 { Duration::from_secs(5) } else { Duration::from_secs(1) });
            let outcome = poll_netease_position(&app);
            // 只在可用性变化时记录，避免每秒刷日志。
            if matches!(outcome, CdpPoll::Read | CdpPoll::Failed) && last != Some(outcome)
            {
                crate::log_debug!("歌词跟随：网易云调试端口{}", if outcome == CdpPoll::Read { "已接管播放位置" } else { "不可用，回退本地外推" });
            }
            last = Some(outcome);
            failures = if outcome == CdpPoll::Failed { failures.saturating_add(1) } else { 0 };
        }
    });
}

/// 每代至多取词一次，网络不阻塞媒体线程；迟到响应不得污染下一首。
pub fn publish(app: &tauri::AppHandle)
{
    start_cdp_poller(app); // 位置轮询独立于取词开关，关闭歌词也要跟进度。
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
        state.derived_duration_ms = None;
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
    let source = state.source.clone(); // 取词线程按播放源切换歌词源，不能借用正在释放的锁内引用。
    let mut requested = REQUESTED.lock().unwrap_or_else(|e| e.into_inner());
    if *requested == Some(generation)
    {
        return;
    }
    *requested = Some(generation);
    drop(state);
    let app = app.clone();
    std::thread::spawn(move || {
        let fetched = fetch_lyrics(&timeline, &source).unwrap_or_default();
        crate::log_debug!("歌词：来源={}，行数={}，时长毫秒={:?}", source, fetched.lines.len(), fetched.duration_ms); // 只记数量与时长，不记正文。
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
        let FetchedLyrics { lines, duration_ms } = fetched;
        state.lyrics = lines;
        state.derived_duration_ms = duration_ms;
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

/// 查询用标题：先统一简体再取主标题；仅测试直接使用，生产走 `query_title_stripped`。
#[cfg(test)]
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

/// 取词结果：歌词行与歌词源给出的曲目时长（仅补播放器不报时长时的量程）。
#[derive(Default)]
struct FetchedLyrics
{
    lines: Vec<LyricLine>,
    duration_ms: Option<f64>,
}

/// 播放器已上报的真实时长优先；不报时用歌词源时长兜底，仍未知保持 0。
fn effective_duration(declared_ms: f64, derived_ms: Option<f64>) -> f64
{
    if declared_ms.is_finite() && declared_ms > 0.0 { declared_ms } else { derived_ms.filter(|value| value.is_finite() && *value > 0.0).unwrap_or(0.0) }
}

/// 歌词入口：按当前播放源选择歌词源；未登记来源与取词失败都回退 LRCLIB。
fn fetch_lyrics(timeline: &Timeline, source: &str) -> Result<FetchedLyrics, Box<dyn std::error::Error>>
{
    if timeline.title.trim().is_empty() || timeline.artist.trim().is_empty()
    {
        return Ok(FetchedLyrics::default());
    }
    let client = reqwest::blocking::Client::builder().user_agent("DesktopTools/0.4.2 (music lyrics)").timeout(Duration::from_secs(10)).build()?;
    // 网易云只发元数据、不填时间轴，中文曲库在 LRCLIB 覆盖偏低；按其公开 web 接口取词，失败再回退。
    if is_netease_source(source)
    {
        match fetch_netease_lyrics(&client, timeline)
        {
            Ok(Some((lines, duration_ms))) if !lines.is_empty() => return Ok(FetchedLyrics { lines, duration_ms: Some(duration_ms) }),
            Ok(_) =>
            {}
            Err(error) => crate::log_debug!("歌词：网易云接口失败，回退 LRCLIB，原因={}", error),
        }
    }
    Ok(FetchedLyrics { lines: fetch_lrclib_lyrics(&client, timeline)?, duration_ms: None })
}

/// LRCLIB：按标题与艺人匹配；有时长取 ±2 秒内最接近者，无时长只接受唯一解析结构。
fn fetch_lrclib_lyrics(client: &reqwest::blocking::Client, timeline: &Timeline) -> Result<Vec<LyricLine>, Box<dyn std::error::Error>>
{
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
            let lines = match_lyrics(request_candidates(client, &params)?, &timeline.title, &timeline.artist, duration);
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
        candidates.extend(request_candidates(client, &params)?);
    }
    Ok(match_lyrics(candidates, &timeline.title, &timeline.artist, 0.0))
}

/// 网易云的播放源身份；判定与调试端口模块共用。
fn is_netease_source(source: &str) -> bool { crate::platform::windows::netease_cdp::is_netease_source(source) }

#[derive(Deserialize)]
struct NeteaseSearch
{
    result: Option<NeteaseSearchResult>,
}

#[derive(Deserialize)]
struct NeteaseSearchResult
{
    songs: Option<Vec<NeteaseSong>>,
}

#[derive(Deserialize)]
struct NeteaseSong
{
    id: i64,
    name: String,
    #[serde(default)]
    artists: Vec<NeteaseArtist>,
    #[serde(default)]
    duration: Option<f64>, // 搜索结果自带的毫秒时长，播放器不报时长时补量程。
}

#[derive(Deserialize)]
struct NeteaseArtist
{
    name: String,
}

#[derive(Deserialize)]
struct NeteaseLyric
{
    lrc: Option<NeteaseLrc>,
}

#[derive(Deserialize)]
struct NeteaseLrc
{
    lyric: String,
}

/// 在网易云搜索结果里按标题与艺人收敛候选；同名不同艺人不猜，交给回退源。
fn select_netease_songs<'a>(songs: &'a [NeteaseSong], title: &str, artist: &str) -> Vec<&'a NeteaseSong>
{
    let target_title = normalize_title(title);
    let target_artist = normalize_artist(artist);
    if target_title.is_empty() || target_artist.is_empty()
    {
        return vec![];
    }
    songs
        .iter()
        .filter(|song| {
            if normalize_title(&song.name) != target_title
            {
                return false;
            }
            let names = song.artists.iter().map(|artist| artist.name.as_str()).collect::<Vec<_>>().join("/");
            let candidate = normalize_artist(&names);
            candidate == target_artist || (target_artist.chars().count() >= 2 && candidate.contains(target_artist.as_str())) || (candidate.chars().count() >= 2 && target_artist.contains(candidate.as_str()))
        })
        .collect()
}

/// 读取有界响应体，避免不可信响应占用内存。
fn read_body(response: reqwest::blocking::Response) -> Result<String, Box<dyn std::error::Error>>
{
    use std::io::Read;
    let mut body = String::new();
    response.take(2 * 1024 * 1024).read_to_string(&mut body)?;
    Ok(body)
}

/// 网易云公开 web 接口：先搜索候选，再按 id 取同步歌词；不登录、不解密。
fn fetch_netease_lyrics(client: &reqwest::blocking::Client, timeline: &Timeline) -> Result<Option<(Vec<LyricLine>, f64)>, Box<dyn std::error::Error>>
{
    let keyword = format!("{} {}", to_simplified(timeline.title.trim()), to_simplified(timeline.artist.trim()));
    let response = client
        .get("https://music.163.com/api/search/get")
        .header(reqwest::header::REFERER, "https://music.163.com/")
        .query(&[("s", keyword.as_str()), ("type", "1"), ("limit", "10")])
        .send()?
        .error_for_status()?;
    let search: NeteaseSearch = serde_json::from_str(&read_body(response)?)?;
    let Some(songs) = search.result.and_then(|result| result.songs)
    else
    {
        return Ok(None);
    };
    // 同名同艺人常有多个版本，第一条可能无词；按搜索顺序取到第一条有同步词的为止，最多 5 条。
    for song in select_netease_songs(&songs, &timeline.title, &timeline.artist).into_iter().take(5)
    {
        let response = client
            .get("https://music.163.com/api/song/lyric")
            .header(reqwest::header::REFERER, "https://music.163.com/")
            .query(&[("id", song.id.to_string()), ("lv", "1".to_string()), ("tv", "-1".to_string())])
            .send()?
            .error_for_status()?;
        let lyric: NeteaseLyric = serde_json::from_str(&read_body(response)?)?;
        let lines = parse_lrc(&to_simplified(&lyric.lrc.map(|lrc| lrc.lyric).unwrap_or_default()));
        if !lines.is_empty()
        {
            return Ok(Some((lines, song.duration.unwrap_or(0.0))));
        }
    }
    Ok(None)
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

    /// 生成一条时间轴事实，用于零时间轴延续与真实时间轴隔离的断言。
    fn timeline(session: &str, title: &str, end: &str, updated: &str, position_ms: f64, duration_ms: f64, advancing: bool) -> Timeline { Timeline { session_id: session.into(), title: title.into(), artist: "人".into(), position_ticks: "0".into(), end_time_ticks: end.into(), last_updated_time_ticks: updated.into(), playback_status: if advancing { 4 } else { 5 }, position_ms, duration_ms, advancing } }

    #[test]
    fn zero_timeline_projection_uses_local_clock_and_freezes_on_pause()
    {
        assert!(zero_timeline("0", "0"));
        assert!(!zero_timeline("0", "134000000000000000"));
        assert!(!zero_timeline("10000000", "0"));
        // 时长未知时不夹紧，避免零时长把位置钉在 0；已知时长仍夹在轨道内。
        assert_eq!(projected_position(0.0, 0.0, 1_500.0, true), 1_500.0);
        assert_eq!(projected_position(0.0, 0.0, 1_500.0, false), 0.0);
        assert_eq!(projected_position(9_000.0, 10_000.0, 2_000.0, true), 10_000.0);

        // 零时间轴事件间延续本地锚点：暂停冻结在投影位置，恢复后从冻结位置继续。
        let playing = timeline("1", "歌", "0", "0", 0.0, 0.0, true);
        let paused = carry_zero_timeline(Some(&playing), Some(0.0), 1_500.0, timeline("1", "歌", "0", "0", 0.0, 0.0, false), false);
        assert!(!paused.advancing);
        assert_eq!(paused.position_ms, 1_500.0);
        let resumed = carry_zero_timeline(Some(&paused), Some(0.0), 900.0, timeline("1", "歌", "0", "0", 0.0, 0.0, true), false);
        assert!(resumed.advancing);
        assert_eq!(resumed.position_ms, 1_500.0);
        // 公开 Position 相对上一事件变化（best-effort 跳转锚点）时重置本地锚点；网易云恒 0 时只能继续外推。
        let seeked = carry_zero_timeline(Some(&resumed), Some(0.0), 900.0, timeline("1", "歌", "0", "0", 7_000.0, 0.0, true), false);
        assert_eq!(seeked.position_ms, 7_000.0);
        // 跳回 0 同样是公开 Position 变化，按新位置重置而不是继续外推。
        assert_eq!(carry_zero_timeline(Some(&seeked), Some(7_000.0), 900.0, timeline("1", "歌", "0", "0", 0.0, 0.0, true), false).position_ms, 0.0);
        // 切歌重置为 0；真实时间轴直接采用上报位置，不被本地锚点污染。
        assert_eq!(carry_zero_timeline(Some(&resumed), Some(0.0), 900.0, timeline("2", "新歌", "0", "0", 0.0, 0.0, true), true).position_ms, 0.0);
        assert_eq!(carry_zero_timeline(Some(&resumed), Some(0.0), 900.0, timeline("3", "真实", "10000000", "134000000000000000", 3_000.0, 1_000.0, true), true).position_ms, 3_000.0);
        // 真实时间轴在同曲重读时不被本地锚点改写。
        assert_eq!(carry_zero_timeline(Some(&resumed), Some(0.0), 900.0, timeline("1", "歌", "10000000", "134000000000000000", 3_000.0, 1_000.0, true), false).position_ms, 3_000.0);
    }

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

#[cfg(test)]
mod netease_tests
{
    use super::*;

    /// 只有网易云的播放源身份走专用歌词源，其余来源走通用兜底。
    #[test]
    fn netease_source_detection_covers_cloudmusic_only()
    {
        assert!(is_netease_source("cloudmusic.exe"));
        assert!(is_netease_source("CloudMusic.EXE"));
        assert!(!is_netease_source("QQMusic.exe"));
        assert!(!is_netease_source("Spotify.exe"));
        assert!(!is_netease_source(""));
    }

    /// 搜索候选按标题与艺人收敛；同名同艺人的多个版本都保留，交给取词循环逐个尝试。
    #[test]
    fn netease_song_selection_keeps_all_matching_versions()
    {
        let song = |id: i64, name: &str, artist: &str, duration: Option<f64>| NeteaseSong { id, name: name.into(), artists: vec![NeteaseArtist { name: artist.into() }], duration };
        let songs = vec![
            song(1, "晴天", "周杰伦", Some(269_117.0)),
            song(2, "晴天 (Live)", "周杰伦", Some(300_000.0)),
            song(3, "晴天", "孙燕姿", Some(200_000.0)),
            song(4, "晴天", "周杰伦", Some(236_818.0)),
        ];
        let ids = |title: &str, artist: &str| select_netease_songs(&songs, title, artist).iter().map(|song| song.id).collect::<Vec<_>>();
        assert_eq!(ids("晴天", "周杰伦"), vec![1, 4]);
        assert_eq!(ids("晴天 (Live)", "周杰伦"), vec![2]);
        assert_eq!(ids("晴天", "孙燕姿"), vec![3]);
        assert!(ids("晴天", "张学友").is_empty());
        assert_eq!(ids("晴天（电视剧片尾曲）", "周杰伦"), vec![1, 4]);
    }

    /// 播放器不报时长时用歌词源时长兜底；已上报的真实时长优先，非法值不参与。
    #[test]
    fn derived_duration_only_fills_missing_timeline()
    {
        assert_eq!(effective_duration(0.0, Some(236_818.0)), 236_818.0);
        assert_eq!(effective_duration(100_000.0, Some(236_818.0)), 100_000.0);
        assert_eq!(effective_duration(0.0, Some(0.0)), 0.0);
        assert_eq!(effective_duration(0.0, None), 0.0);
        assert_eq!(effective_duration(0.0, Some(f64::NAN)), 0.0);
    }
}
