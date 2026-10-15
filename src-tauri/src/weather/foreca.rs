use super::credentials::ForecaCredentials;
use super::provider::{City, Failure, retry_at, validate_weather};
use chrono::{NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::{
    io::Read,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

/// 服务地址由平台适配固定，不接受用户填写的地址。
pub(super) const ADDRESS: &str = "weatherapi.foreca.net";

/// 符号代码按昼夜、云量与降水强度解析，云量达到分档即视为非晴。
const PRECIPITATION_BASE: u32 = 300;

/// 单次验证使用一个 HTTP 客户端，不常驻线程或自动轮询。
pub struct Foreca
{
    client: Client,                      // TLS、超时和重定向策略。
    credentials: Arc<ForecaCredentials>, // 当前会话凭据。
    cancelled: Arc<AtomicBool>,          // 离开、取消或字段更换时停止后续请求。
}

impl Foreca
{
    /// 在阻塞工作线程创建客户端，禁止凭据随重定向泄露。
    pub fn new(credentials: Arc<ForecaCredentials>, cancelled: Arc<AtomicBool>) -> Result<Self, Failure>
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
        let mut response = self.client.get(format!("https://{ADDRESS}{path}")).query(query).bearer_auth(&self.credentials.api_key).send().map_err(|_| Failure { retryable: true, ..Failure::from("网络不可用或请求超时，未完成验证") })?; // 不输出 reqwest 错误中的 URL。
        let status = response.status().as_u16(); // HTTP 错误分类。
        if status != 200
        {
            let mut failure = match status
            {
                400 => Failure::from("服务未接受该请求参数，请更换位置搜索名称后重试"),
                401 => Failure::from("凭据无效，请核对 Foreca API key 是否完整且仍有效"),
                403 => Failure::from("接口权限不足，请核对该 key 的套餐是否包含位置搜索与预报接口"),
                404 => Failure::from("没有找到该位置，请更换搜索名称"),
                429 => Failure::from("请求受到限流，请稍后重试"),
                _ => Failure::from("天气服务请求失败"),
            }; // 不向用户展示上游正文。
            crate::log_debug!("天气请求：Foreca 返回状态码 {status}"); // 只记录状态码，不记录密钥或响应正文。
            failure.retryable = status == 429 || status >= 500;
            if status == 429 || status == 503
            {
                failure.retry_at = retry_at(response.headers());
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
        serde_json::from_slice(&bytes).map_err(|_| Failure::from("天气响应格式无效")) // 结构校验在归一化中进行。
    }

    /// 手动搜索，不保存位置检索结果。
    pub fn search(&self, query: &str) -> Result<Vec<City>, Failure>
    {
        let response = self.request(&format!("/api/v1/location/search/{query}"), &[("lang", "zh_CN")])?; // 中文位置名。
        let locations = response.get("locations").and_then(Value::as_array).ok_or("位置数据缺失")?;
        if locations.len() > 10
        {
            return Err("位置结果数量无效".into());
        }
        locations.iter().map(location).collect()
    }

    /// 存活探测只请求一次当前天气，不覆盖其他模块的缓存或失败状态。
    pub fn probe(&self, city: &City) -> Result<(), Failure>
    {
        let position = format!("{},{}", city.lon, city.lat); // 该来源接受“经度,纬度”作为位置。
        let value = self.request(&format!("{}/{position}", endpoint("current")), &[("lang", "zh"), ("dataset", "full")])?; // 与验证、桌面读取共用同一响应边界。
        validate_weather("current", &current(&value)?, city, Utc::now()).map_err(Failure::from)
    }

    /// 分别验证三个接口；归一化与时间窗校验与和风共用同一套规则。
    pub fn validate(&self, city: &City) -> Vec<Result<(), Failure>>
    {
        let position = position(city);
        let mut results = Vec::new(); // 当前、小时、每日的独立结果。
        for (module, query) in [("current", vec![]), ("hourly", vec![("periods", "24"), ("history", "0")]), ("daily", vec![("periods", "7")])]
        {
            let mut parameters = query;
            parameters.push(("lang", "zh"));
            parameters.push(("dataset", "full"));
            let result = self.request(&format!("{}/{position}", endpoint(module)), &parameters).and_then(|value| {
                let normalized = match module
                {
                    "current" => current(&value),
                    "hourly" => hourly(&value),
                    _ => daily(&value, city),
                }?;
                validate_weather(module, &normalized, city, Utc::now()).map_err(Failure::from)
            });
            let stop = result.as_ref().err().is_some_and(|e| e.retry_at.is_some() || e.message.contains("限流") || e.message.contains("凭据无效") || e.message.contains("取消")); // 不继续消耗已失效账户的请求。
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
        results
    }
    /// 只请求指定的天气模块，返回与和风一致的归一化结构。
    pub fn forecast_modules(&self, city: &City, modules: &[&str], is_active: impl Fn() -> bool) -> (Value, std::collections::BTreeMap<String, Failure>)
    {
        let position = position(city);
        let fetched_at = Utc::now().timestamp_millis();
        let mut errors = serde_json::Map::new();
        let mut module_fetched_at = serde_json::Map::new();
        let mut failures = std::collections::BTreeMap::new(); // 部分成功也必须保留每个失败模块的请求策略。
        // 桌面读取与设置验证共用归一化结构和时间窗校验，坏数据不获得成功时间戳。
        let mut request_module = |name: &str, query: Vec<(&str, &str)>, is_current: bool| {
            let mut parameters = query.clone();
            parameters.push(("lang", "zh"));
            parameters.push(("dataset", "full"));
            let result = if is_active()
            {
                self.request(&format!("{}/{position}", endpoint(name)), &parameters)
            }
            else
            {
                Err(Failure::from("天气实例已停止活动"))
            }
            .and_then(|value| {
                let normalized = if is_current
                {
                    current(&value)
                }
                else if name == "hourly"
                {
                    hourly(&value)
                }
                else
                {
                    daily(&value, city)
                }?;
                validate_weather(name, &normalized, city, Utc::now()).map_err(Failure::from)?;
                Ok(normalized)
            });
            match result
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
            }
        };
        let current_value = modules.contains(&"current").then(|| request_module("current", vec![], true)).flatten();
        let hourly_value = modules.contains(&"hourly").then(|| request_module("hourly", vec![("periods", "24"), ("history", "0")], false)).flatten();
        let daily_value = modules.contains(&"daily").then(|| request_module("daily", vec![("periods", "7")], false)).flatten();
        (json!({ "city": city, "current": current_value, "hourly": hourly_value, "daily": daily_value, "moduleErrors": errors, "moduleFetchedAt": module_fetched_at, "attributions": [], "fetchedAt": fetched_at }), failures)
    }
}

/// 该来源以位置标识作为请求参数，坐标仅在搜索时归一化。
fn position(city: &City) -> String { city.id.clone() }

/// 当前天气与其余预报模块使用不同地址，按来源声明的端点映射。
fn endpoint(module: &str) -> String
{
    match module
    {
        "current" => "/api/v1/current".to_string(),
        _ => format!("/api/v1/forecast/{module}"),
    }
}

/// 位置标识与坐标必须可解析，时区必须是 IANA 名称。
fn location(value: &Value) -> Result<City, Failure>
{
    let id = value.get("id").and_then(Value::as_i64).ok_or("位置标识缺失")?;
    let name = text(value, "name")?;
    let lat = value.get("lat").and_then(Value::as_f64).ok_or("位置坐标缺失")?;
    let lon = value.get("lon").and_then(Value::as_f64).ok_or("位置坐标缺失")?;
    let tz = text(value, "timezone")?;
    if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) || tz.parse::<Tz>().is_err()
    {
        return Err("位置数据无效".into());
    }
    Ok(City { id: id.to_string(), name, adm1: optional(value, "adminArea"), adm2: optional(value, "adminArea2"), country: optional(value, "country"), lat: format!("{lat:.2}"), lon: format!("{lon:.2}"), tz, fx_link: String::new() }) // 该来源没有城市天气网页。
}

/// 读取必填文本字段。
fn text(value: &Value, key: &str) -> Result<String, Failure> { value.get(key).and_then(Value::as_str).filter(|item| !item.trim().is_empty() && item.len() <= 1024).map(str::to_string).ok_or_else(|| Failure::from("位置或天气字段缺失")) }

/// 读取可缺省的文本字段。
fn optional(value: &Value, key: &str) -> String { value.get(key).and_then(Value::as_str).filter(|item| item.len() <= 1024).unwrap_or_default().to_string() }

/// 单位固定为摄氏，供应商缺测值按缺失处理。
fn temperature(value: &Value, key: &str) -> Result<Value, Failure>
{
    let number = value.get(key).and_then(Value::as_f64).filter(|item| item.is_finite()).ok_or("温度数据缺失")?;
    Ok(json!({ "value": number, "unit": "°C" }))
}

/// 符号代码与描述构成条件字段。
fn condition(value: &Value) -> Result<Value, Failure>
{
    let symbol = text(value, "symbol")?;
    Ok(json!({ "text": text(value, "symbolPhrase")?, "code": symbol_code(&symbol)?.to_string() }))
}

/// 符号代码为昼夜字母加三位数字；降水强度非零时映射到降水区间，便于统一图标判断。
fn symbol_code(symbol: &str) -> Result<u32, Failure>
{
    let digits = symbol.get(1..).ok_or("天气符号无效")?;
    if !matches!(symbol.as_bytes().first(), Some(b'd' | b'n')) || digits.len() != 3 || !digits.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err("天气符号无效".into());
    }
    let cloudiness = digits.as_bytes()[0] - b'0'; // 云量分档。
    let intensity = digits.as_bytes()[1] - b'0'; // 降水强度分档。
    let kind = digits.as_bytes()[2] - b'0'; // 降水类型分档。
    Ok(
        if intensity > 0
        {
            PRECIPITATION_BASE + u32::from(intensity) * 10 + u32::from(kind)
        }
        else
        {
            u32::from(cloudiness)
        },
    )
}

/// 当前天气归一化为共享结构。
pub(super) fn current(value: &Value) -> Result<Value, Failure>
{
    let item = value.get("current").ok_or("当前天气数据缺失")?;
    Ok(json!({ "temperature": temperature(item, "temperature")?, "condition": condition(item)? }))
}

/// 小时预报归一化为共享结构，保持供应商的时间戳与时区。
pub(super) fn hourly(value: &Value) -> Result<Value, Failure>
{
    let items = value.get("forecast").and_then(Value::as_array).ok_or("小时预报数据缺失")?;
    let hours = items
        .iter()
        .map(|item| {
            Ok(json!({
                "forecastTime": text(item, "time")?,
                "temperature": temperature(item, "temperature")?,
                "condition": condition(item)?,
            }))
        })
        .collect::<Result<Vec<_>, Failure>>()?;
    Ok(json!({ "hours": hours }))
}

/// 日预报归一化为共享结构，按城市时区补齐本地日起止边界。
pub(super) fn daily(value: &Value, city: &City) -> Result<Value, Failure>
{
    let zone = city.tz.parse::<Tz>().map_err(|_| Failure::from("城市时区无效"))?;
    let items = value.get("forecast").and_then(Value::as_array).ok_or("日预报数据缺失")?;
    let days = items
        .iter()
        .map(|item| {
            let date = NaiveDate::parse_from_str(&text(item, "date")?, "%Y-%m-%d").map_err(|_| Failure::from("日预报日期无效"))?;
            let start = midnight(&zone, date)?; // 供应商返回本地日历日期，不携带偏移。
            let end = midnight(&zone, date.succ_opt().ok_or("日预报日期无效")?)?; // 夏令时日期边界可能相差 23 或 25 小时。
            let condition = condition(item)?;
            Ok(json!({
                "forecastStartTime": start,
                "forecastEndTime": end,
                "temperatureMax": temperature(item, "maxTemp")?,
                "temperatureMin": temperature(item, "minTemp")?,
                "daytime": { "condition": condition.clone() }, // 供应商每天只有一个符号，代表白天 7 时至 18 时。
                "nighttime": { "condition": condition },
            }))
        })
        .collect::<Result<Vec<_>, Failure>>()?;
    Ok(json!({ "days": days }))
}

/// 该时区的本地零点；夏令时歧义时取第一个偏移。
fn midnight(zone: &Tz, date: NaiveDate) -> Result<String, Failure>
{
    let naive = date.and_hms_opt(0, 0, 0).ok_or("日预报日期无效")?;
    zone.from_local_datetime(&naive).earliest().map(|time| time.to_rfc3339()).ok_or_else(|| Failure::from("日预报日期无效"))
}

#[cfg(test)]
#[path = "foreca.test.rs"]
mod tests;
