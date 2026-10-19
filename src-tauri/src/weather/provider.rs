use super::credentials::{Credentials, ForecaCredentials, QweatherCredentials};
use chrono::{DateTime, Utc};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use reqwest::{Url, blocking::Client, header::HeaderMap};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    io::Read,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

/// 经校验的临时搜索结果，只保留在当前验证会话中。
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct City
{
    pub id: String,      // 供应商位置标识。
    pub name: String,    // 位置名称。
    pub adm1: String,    // 一级行政区。
    pub adm2: String,    // 二级行政区。
    pub country: String, // 国家。
    pub lat: String,     // 供应商纬度。
    pub lon: String,     // 供应商经度。
    pub tz: String,      // IANA 时区。
    pub fx_link: String, // 当前城市的详情链接。
}

/// 只返回经过归类的错误，不携带请求或原始响应。
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Failure
{
    pub message: String,       // 用户可见说明。
    pub retry_at: Option<i64>, // 服务端规定的最早重试时间。
    #[serde(skip)]
    pub retryable: bool, // 后端请求策略，不依赖用户可见错误文案。
}

impl From<&str> for Failure
{
    fn from(message: &str) -> Self { Self { message: message.into(), retry_at: None, retryable: false } }
}

/// 按 HTTP 状态码归类失败；只有限流与服务端错误可重试，402 额度不足不设置等待。
fn status_failure(status: u16) -> Failure
{
    let mut failure = match status
    {
        401 => Failure::from("凭据无效，请核对账户、私钥和设备时间"),
        402 => Failure::from("账户额度不足，请检查和风账户"),
        403 => Failure::from("接口权限不足"),
        429 => Failure::from("请求受到限流，请稍后重试"),
        _ => Failure::from("天气服务请求失败"),
    }; // 不向用户展示上游正文。
    failure.retryable = status == 429 || status >= 500;
    failure
}

/// 按来源校验凭据格式；格式通过不代表账户或接口可用。
pub fn check_credentials(credentials: &Credentials) -> Result<(), String>
{
    match credentials
    {
        Credentials::Qweather(value) => check_qweather(value),
        Credentials::Foreca(value) => check_foreca(value),
    }
}

/// Foreca 的 API key 是较长的签名令牌，只校验非空、长度边界与常见复制错误。
fn check_foreca(credentials: &ForecaCredentials) -> Result<(), String>
{
    if credentials.api_key.is_empty() || credentials.api_key.len() > 1024 || credentials.api_key.trim() != credentials.api_key
    {
        return Err("请填写完整的 Foreca API key，不要包含首尾空白".into());
    }
    if !credentials.api_key.bytes().all(|byte| byte.is_ascii_graphic()) || credentials.api_key.starts_with("Bearer")
    {
        return Err("API key 含空白或不可见字符，请重新从 Foreca 控制台复制完整内容".into());
    }
    Ok(())
}

/// 完整 Host 仅验证 DNS 语法和官方域名边界，不解释标签含义。
fn check_qweather(credentials: &QweatherCredentials) -> Result<(), String>
{
    let host = &credentials.host; // 保留控制台复制的完整主机名。
    if host.len() > 253 || host.split('.').any(|label| label.is_empty() || label.len() > 63 || label.starts_with('-') || label.ends_with('-') || !label.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-'))
    {
        return Err("API Host 格式无效，请仅填写主机名".into());
    }
    if !host.to_ascii_lowercase().ends_with(".qweatherapi.com")
    {
        return Err("API Host 必须为账户的 qweatherapi.com 子域名".into());
    }
    for id in [&credentials.developer_id, &credentials.project_id, &credentials.credential_id]
    {
        if id.is_empty() || id.len() > 64 || !id.bytes().all(|c| c.is_ascii_alphanumeric())
        {
            return Err("账户 ID 格式无效".into());
        }
    }
    if credentials.developer_id.len() != 10 || !credentials.developer_id.starts_with('Q')
    {
        return Err("开发者 ID 应为 Q 开头的 10 位字母或数字".into());
    }
    if credentials.private_key.len() > 1024
    {
        return Err("私钥内容过长".into());
    }
    token(credentials).map(|_| ())
}

/// 按官方要求提前 30 秒签发 15 分钟令牌，仅在 Rust 内使用。
fn token(credentials: &QweatherCredentials) -> Result<String, String>
{
    let key = EncodingKey::from_ed_pem(credentials.private_key.as_bytes()).map_err(|_| "私钥必须为 PKCS#8 Ed25519 PEM")?; // 库负责 ASN.1 和算法校验。
    let mut header = Header::new(Algorithm::EdDSA); // 不加入无关 JWT 字段。
    header.typ = None;
    header.kid = Some(credentials.credential_id.clone());
    let issued = Utc::now().timestamp() - 30; // 容忍小幅设备时钟误差。
    jsonwebtoken::encode(&header, &json!({"iss":credentials.developer_id,"sub":credentials.project_id,"iat":issued,"exp":issued+900}), &key).map_err(|_| "私钥签名失败，请核对 Ed25519 私钥".into())
}

/// 单次验证使用一个 HTTP 客户端，不常驻线程或自动轮询。
pub struct Provider
{
    client: Client,                        // TLS、超时和重定向策略。
    credentials: Arc<QweatherCredentials>, // 当前会话凭据。
    cancelled: Arc<AtomicBool>,            // 离开、取消或字段更换时停止后续请求。
}

impl Provider
{
    /// 在阻塞工作线程创建客户端，禁止凭据随重定向泄露。
    pub fn new(credentials: Arc<QweatherCredentials>, cancelled: Arc<AtomicBool>) -> Result<Self, Failure>
    {
        let client = Client::builder().https_only(true).redirect(reqwest::redirect::Policy::none()).connect_timeout(Duration::from_secs(8)).timeout(Duration::from_secs(20)).gzip(true).build().map_err(|_| Failure::from("天气网络客户端初始化失败"))?;
        Ok(Self { client, credentials, cancelled })
    }

    /// 有界读取解压后的 JSON；供应商错误正文不进入日志或前端。
    fn request(&self, path: &str, query: &[(&str, &str)]) -> Result<Value, Failure>
    {
        if self.cancelled.load(Ordering::Acquire)
        {
            return Err("验证已取消".into());
        }
        let jwt = token(&self.credentials).map_err(|_| Failure::from("JWT 签名失败，请核对私钥与设备时间"))?; // 请求时签发，不缓存令牌。
        let mut response = self.client.get(format!("https://{}{path}", self.credentials.host)).query(query).bearer_auth(jwt).send().map_err(|_| Failure { retryable: true, ..Failure::from("网络不可用或请求超时，未完成验证") })?; // 不输出 reqwest 错误中的 URL。
        let status = response.status().as_u16(); // HTTP 错误分类。
        if status != 200
        {
            let mut failure = status_failure(status);
            crate::log_debug!("天气请求：和风返回状态码 {status}"); // 只记录状态码，不记录凭据或响应正文。
            if status == 429 || status == 503
            {
                failure.retry_at = retry_at(response.headers()); // 与 Foreca 使用同一条等待判定。
            }
            return Err(failure);
        }
        let mut bytes = Vec::new(); // 解压后的响应上限 512 KiB。
        response.by_ref().take(524_289).read_to_end(&mut bytes).map_err(|_| Failure { retryable: true, ..Failure::from("天气响应读取失败") })?;
        if bytes.len() > 524_288
        {
            return Err("天气响应超过大小限制".into());
        }
        if self.cancelled.load(Ordering::Acquire)
        {
            return Err("验证已取消".into());
        }
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| Failure::from("天气响应格式无效"))?; // 解析结构后再验证业务字段。
        if let Some(code) = value.get("code").and_then(Value::as_str)
        {
            match code
            {
                "200" =>
                {}
                "404" => return Err("没有找到城市".into()),
                "401" => return Err("凭据无效，请核对账户、私钥和设备时间".into()),
                "402" => return Err("账户额度不足，请检查和风账户".into()),
                "403" => return Err("接口权限不足".into()),
                "429" =>
                {
                    let mut failure = Failure::from("请求受到限流，请稍后重试");
                    failure.retryable = true;
                    failure.retry_at = retry_at(response.headers());
                    return Err(failure);
                }
                _ => return Err("城市搜索服务请求失败".into()),
            }
        }
        Ok(value)
    }

    /// 手动搜索，不保存 GeoAPI 响应。
    pub fn search(&self, query: &str) -> Result<Vec<City>, Failure>
    {
        let response = self.request("/geo/v2/city/lookup", &[("location", query), ("number", "10"), ("lang", "zh")])?; // 同一账户官方 Host。
        let cities: Vec<City> = serde_json::from_value(response.get("location").cloned().ok_or("城市数据缺失")?).map_err(|_| Failure::from("城市数据格式无效"))?; // 只取已定义字段。
        if cities.len() > 10
        {
            return Err("城市结果数量无效".into());
        }
        for city in &cities
        {
            let lat = city.lat.parse::<f64>().map_err(|_| Failure::from("城市坐标无效"))?; // 纬度。
            let lon = city.lon.parse::<f64>().map_err(|_| Failure::from("城市坐标无效"))?; // 经度。
            if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) || city.tz.parse::<chrono_tz::Tz>().is_err() || city.id.is_empty() || city.name.is_empty() || [&city.id, &city.name, &city.adm1, &city.adm2, &city.country, &city.fx_link].iter().any(|s| s.len() > 1024)
            {
                return Err("城市数据无效".into());
            }
        }
        Ok(cities)
    }

    /// 存活探测只请求一次当前天气，不复用完整能力验证的其余接口。
    pub fn probe(&self, city: &City) -> Result<(), Failure>
    {
        let position = format!("{:.2}/{:.2}", city.lat.parse::<f64>().unwrap_or_default(), city.lon.parse::<f64>().unwrap_or_default()); // 官方最多两位坐标。
        let value = self.request(&format!("/weather/v1/current/{position}"), &[("lang", "zh")])?;
        validate_weather("current", &value, city, Utc::now()).map_err(Failure::from)
    }

    /// 分别验证三个接口与来源，不将 HTTP 成功当作完整能力。
    pub fn validate(&self, city: &City) -> Vec<Result<(), Failure>>
    {
        let position = format!("{:.2}/{:.2}", city.lat.parse::<f64>().unwrap_or_default(), city.lon.parse::<f64>().unwrap_or_default()); // 官方最多两位坐标。
        let mut sources = valid_detail(city); // 来源与详情独立报告。
        let mut results = Vec::new(); // 当前、小时、每日的独立结果。
        for (module, query) in [("current", vec![("lang", "zh")]), ("hourly", vec![("hours", "24"), ("lang", "zh")]), ("daily", vec![("days", "7"), ("lang", "zh")])]
        {
            let result = self.request(&format!("/weather/v1/{module}/{position}"), &query).and_then(|value| {
                sources &= value.pointer("/metadata/attributions").and_then(Value::as_array).is_some_and(|items| !items.is_empty() && items.iter().all(|v| v.as_str().is_some_and(|s| !s.trim().is_empty())));
                validate_weather(module, &value, city, Utc::now()).map_err(Failure::from)
            }); // 坏数据不能通过保存门槛。
            if result.is_err()
            {
                sources = false;
            }
            let stop = result.as_ref().err().is_some_and(|e| e.retry_at.is_some() || e.message.contains("限流") || e.message.contains("额度") || e.message.contains("凭据无效") || e.message.contains("取消")); // 不继续消耗已失效账户的请求。
            results.push(result);
            if stop
            {
                while results.len() < 3
                {
                    results.push(Err("未验证".into()));
                }
                break;
            }
        }
        results.push(if sources { Ok(()) } else { Err("来源或当前城市详情链接未通过核验".into()) });
        results
    }

    /// 只请求指定的天气模块，允许调用方复用仍有效的本地模块缓存。
    pub fn forecast_modules(&self, city: &City, modules: &[&str], is_active: impl Fn() -> bool) -> (Value, std::collections::BTreeMap<String, Failure>)
    {
        let position = format!("{:.2}/{:.2}", city.lat.parse::<f64>().unwrap_or_default(), city.lon.parse::<f64>().unwrap_or_default());
        let fetched_at = Utc::now().timestamp_millis();
        let mut errors = serde_json::Map::new();
        let mut module_fetched_at = serde_json::Map::new();
        let mut failures = std::collections::BTreeMap::new(); // 部分成功也必须保留每个失败模块的请求策略。
        // 桌面读取与设置验证共用结构和时间窗校验，坏数据不获得成功时间戳。
        let mut request_module = |name: &str, path: String, query: &[(&str, &str)]| match (if is_active()
        {
            self.request(&path, query)
        }
        else
        {
            Err(Failure::from("天气实例已停止活动"))
        })
        .and_then(|value| {
            validate_weather(name, &value, city, Utc::now()).map_err(Failure::from)?;
            Ok(value)
        })
        {
            Ok(value) =>
            {
                module_fetched_at.insert(name.to_string(), json!(fetched_at));
                Some(value)
            }
            Err(failure) =>
            {
                errors.insert(name.to_string(), json!(failure.message));
                failures.insert(name.to_string(), failure);
                None
            }
        };
        let current = modules.contains(&"current").then(|| request_module("current", format!("/weather/v1/current/{position}"), &[("lang", "zh")])).flatten();
        let hourly = modules.contains(&"hourly").then(|| request_module("hourly", format!("/weather/v1/hourly/{position}"), &[("hours", "24"), ("lang", "zh")])).flatten();
        let daily = modules.contains(&"daily").then(|| request_module("daily", format!("/weather/v1/daily/{position}"), &[("days", "7"), ("lang", "zh")])).flatten();
        let attributions = current.as_ref().and_then(|value| value.pointer("/metadata/attributions")).cloned().unwrap_or_else(|| json!([]));
        (serde_json::json!({ "city": city, "current": current, "hourly": hourly, "daily": daily, "moduleErrors": errors, "moduleFetchedAt": module_fetched_at, "attributions": attributions, "fetchedAt": fetched_at }), failures)
    }
}

/// 解析服务端要求的最早重试时间。
pub(super) fn retry_at(headers: &HeaderMap) -> Option<i64> { headers.get("retry-after").and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<i64>().ok().filter(|seconds| *seconds >= 0).and_then(|seconds| Utc::now().timestamp().checked_add(seconds)).or_else(|| DateTime::parse_from_rfc2822(v).ok().map(|time| time.timestamp()))) }

/// 详情 URL 必须属于和风且包含选定的位置 ID。
fn valid_detail(city: &City) -> bool { Url::parse(&city.fx_link).ok().is_some_and(|url| url.scheme() == "https" && matches!(url.host_str(), Some("www.qweather.com" | "qweather.com")) && url.username().is_empty() && url.password().is_none() && url.port().is_none() && url.path().starts_with("/weather/") && url.path().ends_with(&format!("-{}.html", city.id))) }

/// 和风的城市详情链接规则；由天气模块按来源在验证与保存入口调用。
pub(super) fn validate_city_detail(city: &City) -> Result<(), &'static str>
{
    if valid_detail(city)
    {
        return Ok(());
    }
    Err("城市详情链接无效")
}

/// 验证连续小时和城市本地日期，跨夏令时按真实 UTC 小时及本地日历分别处理。
pub(super) fn validate_weather(module: &str, value: &Value, city: &City, now: DateTime<Utc>) -> Result<(), &'static str>
{
    let temperature = |v: &Value| v.get("value").and_then(Value::as_f64).is_some_and(f64::is_finite) && v.get("unit").and_then(Value::as_str) == Some("°C"); // 明确摄氏单位。
    let condition = |v: &Value| ["text", "code"].iter().all(|key| v.get(key).and_then(Value::as_str).is_some_and(|s| !s.is_empty())); // 内容所需字段。
    let time = |v: &Value, key| v.get(key).and_then(Value::as_str).and_then(|s| DateTime::parse_from_rfc3339(s).ok().or_else(|| DateTime::parse_from_str(s, "%Y-%m-%dT%H:%M%#z").ok())); // 官方示例允许省略秒。
    match module
    {
        "current" if temperature(&value["temperature"]) && condition(&value["condition"]) => Ok(()),
        "hourly" =>
        {
            let items = value["hours"].as_array().filter(|v| v.len() == 24).ok_or("小时预报不足 24 小时")?; // 不补齐缺项。
            let first = time(&items[0], "forecastTime").ok_or("小时预报时间无效")?; // 检查首小时边界。
            if first.timestamp() <= now.timestamp() - 3600 || first.timestamp() > now.timestamp() + 3600
            {
                return Err("小时预报时间窗口无效");
            }
            for (index, item) in items.iter().enumerate()
            {
                if time(item, "forecastTime").is_none_or(|t| t.timestamp() != first.timestamp() + index as i64 * 3600) || !temperature(&item["temperature"]) || !condition(&item["condition"])
                {
                    return Err("小时预报数据缺失或不连续");
                }
            }
            Ok(())
        }
        "daily" =>
        {
            let zone = city.tz.parse::<chrono_tz::Tz>().map_err(|_| "城市时区无效")?; // 使用城市 IANA 时区处理夏令时。
            let today = now.with_timezone(&zone).date_naive(); // 含今天的七天窗口。
            let items = value["days"].as_array().filter(|v| v.len() == 7).ok_or("日预报不足 7 天")?; // 不复制旧日数据。
            for (index, item) in items.iter().enumerate()
            {
                let start = time(item, "forecastStartTime").ok_or("日预报时间无效")?.with_timezone(&zone); // 供应商 UTC 转城市本地日历。
                let end = time(item, "forecastEndTime").ok_or("日预报时间无效")?.with_timezone(&zone); // 日期边界可能相差 23 或 25 小时。
                if start.date_naive() != today + chrono::Days::new(index as u64) || end.date_naive() != start.date_naive() + chrono::Days::new(1) || !temperature(&item["temperatureMax"]) || !temperature(&item["temperatureMin"]) || !condition(&item["daytime"]["condition"]) || !condition(&item["nighttime"]["condition"])
                {
                    return Err("日预报未覆盖含今天的完整 7 天或数据缺失");
                }
            }
            Ok(())
        }
        _ => Err("当前天气数据缺失或单位无效"),
    }
}

#[cfg(test)]
#[path = "provider.test.rs"]
mod tests;
