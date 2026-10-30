use std::time::Duration;

use serde_json::{Value, json};

/// 宿主固定的本机服务探活地址：端口写死，不进命令入参也不进插件设置。
const ASTRBOT_START_TIME_URL: &str = "http://127.0.0.1:6185/api/stat/start-time";

/// 探活专用的请求超时，登录接口才需要更长等待。
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// 超时后按规格重试一次的总次数。
const PROBE_ATTEMPTS: u32 = 2;

/// 单次探测的原始观察结果，判定逻辑只依赖它，便于脱离网络测试。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Observation
{
    /// 拿到 200 且结构与类型都符合预期。
    Running(i64),
    /// 连接被拒，说明服务未运行。
    Refused,
    /// 返回 401，说明被其它机制挡住。
    Unauthorized,
    /// 返回其它非成功状态码。
    StatusFailed,
    /// 200 但 JSON 结构或字段类型不符。
    Malformed,
    /// 请求超时。
    TimedOut,
    /// 其它传输层错误。
    TransportFailed,
}

/// 能力入参允许的服务枚举；新增软件只增加变体与 URL 常量。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProbeService
{
    /// AstrBot 控制台。
    AstrBot,
}

/// 探活结论，只保留规格定义的四档。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProbeLevel
{
    /// 服务正在运行。
    Ok,
    /// 探测被拒或协议异常。
    Warn,
    /// 服务未运行。
    Off,
    /// 不确定，不能显示成离线。
    Unknown,
}

/// 把能力入参的服务名解析成枚举，其它值一律是参数错误。
pub fn parse_probe_service(value: &str) -> Result<ProbeService, String>
{
    match value
    {
        "astrbot" => Ok(ProbeService::AstrBot),
        _ => Err("不支持的服务标识".to_string()),
    }
}

/// 执行一次服务探活，返回固定的四档状态；任何异常都归入 unknown，不向上抛错。
pub fn probe(payload: &Value) -> Value
{
    let Ok(service) = payload.get("service").and_then(Value::as_str).map(parse_probe_service).unwrap_or_else(|| Err("不支持的服务标识".to_string()))
    else
    {
        return probe_result(ProbeLevel::Unknown, None);
    };
    let observation = match service
    {
        ProbeService::AstrBot => observe(ASTRBOT_START_TIME_URL, PROBE_TIMEOUT),
    }; // 单次探测的原始观察，超时已在内部重试。
    let (level, start_time) = judge(observation);
    probe_result(level, start_time)
}

/// 按规格把观察结果映射成四档状态；超时归 unknown，不能显示成离线。
pub fn judge(observation: Observation) -> (ProbeLevel, Option<i64>)
{
    match observation
    {
        Observation::Running(start_time) => (ProbeLevel::Ok, Some(start_time)),
        Observation::Refused => (ProbeLevel::Off, None),
        Observation::Unauthorized | Observation::Malformed | Observation::StatusFailed => (ProbeLevel::Warn, None),
        Observation::TimedOut | Observation::TransportFailed => (ProbeLevel::Unknown, None),
    }
}

/// 生成返回结构：非 ok 时不带启动时间，uptime 交给前端现算。
fn probe_result(level: ProbeLevel, start_time: Option<i64>) -> Value
{
    let checked_at = chrono::Utc::now().timestamp(); // 判定时刻，前端据此判断数据已过期。
    let start_time = start_time.filter(|_| level == ProbeLevel::Ok);
    json!({ "level": probe_level_name(level), "startTime": start_time, "checkedAt": checked_at })
}

/// 四档状态在契约里的固定名称，其它值一律不对外暴露。
fn probe_level_name(level: ProbeLevel) -> &'static str
{
    match level
    {
        ProbeLevel::Ok => "ok",
        ProbeLevel::Warn => "warn",
        ProbeLevel::Off => "off",
        ProbeLevel::Unknown => "unknown",
    }
}

/// 探测指定地址，超时后按规格重试一次；正式调用只用固定常量地址。
fn observe(url: &str, timeout: Duration) -> Observation
{
    // 探活专用客户端：不设 https_only、不启用 cookie jar、不带 Origin 等浏览器头，也不复用天气的客户端。
    let client = match reqwest::blocking::Client::builder().timeout(timeout).build()
    {
        Ok(client) => client,
        Err(error) =>
        {
            crate::log_warn!("AstrBot 探活客户端创建失败：{error}");
            return Observation::TransportFailed;
        }
    };
    let mut observation = Observation::TransportFailed; // 最近一次观察结果。
    for attempt in 1..=PROBE_ATTEMPTS
    {
        observation = observe_once(&client, url);
        if observation != Observation::TimedOut
        {
            break;
        }
        crate::log_debug!("AstrBot 探活超时，第 {attempt} 次，准备重试");
    }
    observation
}

/// 发一次请求并根据状态码与响应体得出观察结果。
fn observe_once(client: &reqwest::blocking::Client, url: &str) -> Observation
{
    let response = match client.get(url).send()
    {
        Ok(response) => response,
        Err(error) => return classify_request_error(&error),
    };
    let status = response.status();
    if status == reqwest::StatusCode::UNAUTHORIZED
    {
        return Observation::Unauthorized;
    }
    if !status.is_success()
    {
        crate::log_debug!("AstrBot 探活返回非成功状态：{status}");
        return Observation::StatusFailed;
    }
    let body = match response.json::<Value>()
    {
        Ok(body) => body,
        Err(_) => return Observation::Malformed,
    };
    if body.get("status").and_then(Value::as_str) != Some("ok")
    {
        return Observation::Malformed;
    }
    match body.pointer("/data/start_time").and_then(Value::as_f64).map(|seconds| seconds.trunc() as i64)
    {
        Some(start_time) if start_time > 0 => Observation::Running(start_time),
        _ => Observation::Malformed,
    }
}

/// 区分连接被拒、超时和其它传输错误。
fn classify_request_error(error: &reqwest::Error) -> Observation
{
    if error.is_timeout()
    {
        return Observation::TimedOut;
    }
    if error.is_connect()
    {
        // 连接阶段失败里，被拒才是「未运行」，其它（解析、网络不可达）仍属不确定。
        return if has_connection_refused(error as &dyn std::error::Error) { Observation::Refused } else { Observation::TransportFailed };
    }
    Observation::TransportFailed
}

/// 沿 source 链找连接被拒的 io 错误；reqwest 与 hyper 之间还有多层包装，不能只看第一层。
fn has_connection_refused(error: &dyn std::error::Error) -> bool
{
    let mut current = Some(error); // 当前正在检查的错误。
    while let Some(error) = current
    {
        if error.downcast_ref::<std::io::Error>().is_some_and(|io| io.kind() == std::io::ErrorKind::ConnectionRefused)
        {
            return true;
        }
        current = error.source();
    }
    false
}

#[cfg(test)]
#[path = "probe.test.rs"]
mod tests;
