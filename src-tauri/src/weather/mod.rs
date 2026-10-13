pub(crate) mod activity;
pub mod credentials;
mod provider;

use credentials::Credentials;
pub use provider::City;
use provider::{Failure, Provider};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

/// 设置页可提交的天气用例；保存命令不接受前端验证标记。
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", rename_all_fields = "camelCase")]
pub enum Command
{
    Status,
    Begin
    {
        credentials: Credentials,
    },
    Search
    {
        session_id: u64,
        query: String,
    },
    Validate
    {
        session_id: u64,
        city_id: String,
    },
    Save
    {
        session_id: u64,
    },
    Cancel
    {
        session_id: u64,
    },
    Clear,
}

#[cfg(test)]
#[path = "mod.test.rs"]
mod tests;

/// 仅包含非敏感状态、临时城市及能力检查结果。
#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot
{
    configured: bool,             // 当前配置存在安全存储记录。
    session_id: Option<u64>,      // Rust 权威会话标识。
    cities: Vec<City>,            // 仅搜索命令返回临时结果。
    checks: Vec<Option<Failure>>, // 每项 None 表示通过，空数组表示未验证。
}

/// 一份草稿只关联一个搜索结果集合和一份验证结果。
struct Session
{
    id: u64,                       // 防止旧页面和迟到回调覆盖新会话。
    credentials: Arc<Credentials>, // 不出现在普通快照中。
    cancelled: Arc<AtomicBool>,    // 后台请求检查取消标记。
    cities: Vec<City>,             // 不持久化的 GeoAPI 选择范围。
    verified: bool,                // 仅完整验证可以设置。
}

/// 同一设置窗口的短期草稿状态。
#[derive(Default)]
struct Inner
{
    window_epoch: u64,                        // 窗口销毁后拒绝尚未开始的排队命令。
    revision: u64,                            // 会话递增身份。
    session: Option<Session>,                 // 关闭页面后清理。
    busy: bool,                               // 包括取消后仍在收尾的 HTTP 请求。
    retry_at: i64,                            // 不因换草稿绕过供应商限流。
    configuration_revision: u64,              // 保存或清除成功后拒绝旧配置的网络结果。
    configuration_cancelled: Arc<AtomicBool>, // 所有已保存配置请求共用取消身份。
}

/// 天气设置权威状态；城市与天气缓存均独立于工作区布局。
pub struct WeatherState
{
    target: String,                       // Windows Credential Manager 的配置归属键。
    city_path: PathBuf,                   // 当前选中城市的独立缓存路径。
    forecast_cache_path: PathBuf,         // 当前城市整包预报缓存路径。
    forecast_guard: Mutex<()>,            // 防止相同启动内的重复天气请求。
    forecast_retry: Mutex<ForecastRetry>, // 当前天气自动请求的短期退避。
    inner: Mutex<Inner>,                  // 串行提交与会话状态。
}

impl WeatherState
{
    /// 在命令进入阻塞队列前记录当前窗口代次。
    pub fn window_epoch(&self) -> Result<u64, String> { Ok(self.inner.lock().map_err(|_| "天气设置状态不可用")?.window_epoch) }
    /// 使用本次启动的配置归属创建空会话。
    pub fn new(directory: &std::path::Path) -> Self { Self { target: credentials::target(directory), city_path: directory.join("weather-city.json"), forecast_cache_path: directory.join("weather-forecast.json"), forecast_guard: Mutex::new(()), forecast_retry: Mutex::new(ForecastRetry::default()), inner: Mutex::new(Inner::default()) } }

    /// 读取当前选中的城市；搜索结果不进入该文件。
    pub fn saved_city(&self) -> Result<Option<City>, String>
    {
        crate::persistence::load_json(&self.city_path).map_err(|error| format!("天气城市读取失败: {error}"))?.map_or(Ok(None), |city| {
            validate_city(&city)?;
            Ok(Some(city))
        })
    }

    /// 保存当前选中的城市，供后续启动恢复天气请求。
    pub fn save_city(&self, city: City) -> Result<City, String>
    {
        validate_city(&city)?;
        crate::persistence::save_json_atomic(&self.city_path, &city).map_err(|error| format!("天气城市保存失败: {error}"))?;
        Ok(city)
    }

    /// 窗口销毁时使在途请求失效，并释放私钥草稿。
    pub fn cancel_all(&self)
    {
        if let Ok(mut inner) = self.inner.lock()
        {
            inner.window_epoch += 1;
            if let Some(session) = inner.session.take()
            {
                session.cancelled.store(true, Ordering::Release);
            }
        }
    }

    /// 在阻塞线程执行用例；网络期间不持有状态锁。
    pub fn execute(&self, command: Command, window_epoch: u64) -> Result<Snapshot, String>
    {
        let mut inner = self.inner.lock().map_err(|_| "天气设置状态不可用")?; // 只锁短事务。
        if inner.window_epoch != window_epoch
        {
            return Err("设置窗口已关闭，命令已取消".into());
        }
        // 取消先释放内存草稿，凭据状态读取失败不得阻止在途请求失效。
        if let Command::Cancel { session_id } = &command
        {
            if inner.session.as_ref().is_some_and(|s| s.id == *session_id)
            {
                if let Some(session) = inner.session.take()
                {
                    session.cancelled.store(true, Ordering::Release);
                }
            }
        }
        let configured = matches!(&command, Command::Status | Command::Begin { .. } | Command::Save { .. }).then(|| credentials::configured(&self.target)).transpose()?.unwrap_or(false); // 搜索、验证和清除不因状态读取失败而阻塞。
        let mut snapshot = Snapshot { configured, ..Default::default() }; // 本次命令结果。
        match command
        {
            Command::Status | Command::Cancel { .. } => return Ok(snapshot),
            Command::Begin { credentials } =>
            {
                provider::check_credentials(&credentials)?;
                if let Some(session) = inner.session.take()
                {
                    session.cancelled.store(true, Ordering::Release);
                }
                inner.revision += 1;
                let id = inner.revision; // 每次草稿使用新身份。
                inner.session = Some(Session { id, credentials: Arc::new(credentials), cancelled: Arc::new(AtomicBool::new(false)), cities: Vec::new(), verified: false });
                snapshot.session_id = Some(id);
                return Ok(snapshot);
            }
            Command::Clear =>
            {
                let mut retry = self.forecast_retry.lock().map_err(|_| "天气请求状态不可用")?; // 凭据清除成功后才释放旧配置的退避。
                credentials::clear(&self.target)?;
                *retry = ForecastRetry::default();
                inner.configuration_cancelled.store(true, Ordering::Release);
                inner.configuration_cancelled = Arc::new(AtomicBool::new(false));
                inner.configuration_revision += 1;
                if let Some(session) = inner.session.take()
                {
                    session.cancelled.store(true, Ordering::Release);
                }
                snapshot.configured = false;
                crate::log_info!("天气设置：已清除本机凭据");
                return Ok(snapshot);
            }
            Command::Save { session_id } =>
            {
                let session = inner.session.as_ref().filter(|s| s.id == session_id && s.verified && !s.cancelled.load(Ordering::Acquire)).ok_or("请先完成全部能力验证")?; // 不信任客户端状态。
                if inner.busy
                {
                    return Err("天气验证仍在执行".into());
                }
                let mut retry = self.forecast_retry.lock().map_err(|_| "天气请求状态不可用")?; // 新配置经过完整验证后恢复请求，失败则保留旧退避。
                credentials::save(&self.target, &session.credentials)?;
                *retry = ForecastRetry::default();
                inner.configuration_cancelled.store(true, Ordering::Release);
                inner.configuration_cancelled = Arc::new(AtomicBool::new(false));
                inner.configuration_revision += 1;
                if let Some(session) = inner.session.take()
                {
                    session.cancelled.store(true, Ordering::Release);
                }
                snapshot.configured = true;
                crate::log_info!("天气设置：验证通过的凭据已安全保存");
                return Ok(snapshot);
            }
            Command::Search { .. } | Command::Validate { .. } =>
            {}
        }
        if inner.busy
        {
            return Err("已有天气请求正在执行，请稍后重试".into());
        }
        if inner.retry_at > chrono::Utc::now().timestamp()
        {
            return Err(format!("服务端要求等待至 {} 后重试", inner.retry_at));
        }
        let (session_id, query, city_id) = match command
        {
            Command::Search { session_id, query } => (session_id, Some(query), None),
            Command::Validate { session_id, city_id } => (session_id, None, Some(city_id)),
            _ => unreachable!(),
        }; // 网络用例参数。
        if query.as_ref().is_some_and(|q| q.trim().is_empty() || q.len() > 200)
        {
            return Err("请输入不超过 200 字节的城市名称".into());
        }
        let session = inner.session.as_mut().filter(|s| s.id == session_id).ok_or("验证会话已失效，请重新选择城市")?; // 身份校验先于请求。
        session.verified = false;
        let city = city_id.as_ref().map(|id| session.cities.iter().find(|city| &city.id == id).cloned().ok_or("请选择当前搜索结果中的城市")).transpose()?; // 不接受前端提供的坐标与链接。
        if query.is_some()
        {
            session.cities.clear();
        }
        let credentials = Arc::clone(&session.credentials); // 仅工作线程持有本次凭据。
        let cancelled = Arc::clone(&session.cancelled); // 会话替换后仍可取消旧任务。
        inner.busy = true;
        drop(inner);
        crate::log_debug!("天气设置：开始{}", if query.is_some() { "城市搜索" } else { "能力验证" });
        let result = Provider::new(credentials, cancelled).map(|provider| {
            if let Some(query) = query
            {
                (provider.search(query.trim()), Vec::new())
            }
            else
            {
                (Ok(Vec::new()), provider.validate(city.as_ref().expect("验证命令已经检查城市")))
            }
        }); // 每次动作只有一个后台任务。
        let mut inner = self.inner.lock().map_err(|_| "天气设置状态不可用")?; // 提交时重新核验会话。
        inner.busy = false;
        let (cities, checks) = result.map_err(|failure| failure.message)?; // 客户端创建失败也释放互斥。
        for failure in cities.as_ref().err().into_iter().chain(checks.iter().filter_map(|r| r.as_ref().err()))
        {
            if let Some(retry_at) = failure.retry_at
            {
                inner.retry_at = inner.retry_at.max(retry_at);
            }
        }
        let session = inner.session.as_mut().filter(|s| s.id == session_id && !s.cancelled.load(Ordering::Acquire)).ok_or("验证已取消或草稿已改变")?; // 拒绝取消后的结果。
        let cities = cities.map_err(|failure| failure.message)?; // 搜索失败不保留旧城市集合。
        if !checks.is_empty()
        {
            session.verified = checks.len() == 4 && checks.iter().all(Result::is_ok);
            snapshot.checks = std::iter::once(None).chain(checks.into_iter().map(Result::err)).collect();
        }
        else
        {
            session.cities = cities.clone();
        }
        snapshot.cities = cities;
        snapshot.session_id = Some(session_id);
        crate::log_debug!("天气设置：会话结果已提交，完整验证：{}", session.verified);
        Ok(snapshot)
    }

    /// 页面只读取缓存；网络更新由共享活动调度器负责。
    pub fn forecast(&self, city: City) -> Result<serde_json::Value, String>
    {
        validate_city(&city)?;
        let _guard = self.forecast_guard.lock().map_err(|_| "天气请求状态不可用")?; // 与调度器磁盘写入串行。
        let _configuration = self.inner.lock().map_err(|_| "天气设置状态不可用")?;
        let credentials = credentials::load(&self.target)?;
        let key = credentials.as_ref().map(credentials::cache_identity).transpose()?;
        let cached = self.read_cached_forecast(&city, key.as_deref())?;
        Ok(display_forecast(cached.unwrap_or_else(|| serde_json::json!({"city": city})), chrono::Utc::now().timestamp_millis()))
    }

    /// 调度器按活动状态刷新，网络期间每个模块开始前重新核对活动条件。
    fn refresh(&self, city: City, is_active: impl Fn() -> bool) -> Result<serde_json::Value, String>
    {
        let _forecast_guard = self.forecast_guard.lock().map_err(|_| "天气请求状态不可用")?;
        let configuration = self.inner.lock().map_err(|_| "天气设置状态不可用")?; // 配置切换与本次缓存读取、凭据获取使用同一个短事务。
        let credentials = credentials::load(&self.target)?; // 无凭据时仅允许展示期限内的旧缓存。
        let source_key = credentials.as_ref().map(credentials::cache_identity).transpose()?; // 当前配置对应的不可逆缓存归属。
        let cached = self.read_cached_forecast(&city, source_key.as_deref())?;
        let Some(credentials) = credentials
        else
        {
            return cached.map(|value| display_forecast(value, chrono::Utc::now().timestamp_millis())).ok_or_else(|| "尚未配置天气凭据".into());
        };
        let mut stale_modules = ["current", "hourly", "daily"].into_iter().filter(|module| !cached.as_ref().is_some_and(|value| module_fresh(value, module))).collect::<Vec<_>>();
        if stale_modules.is_empty()
        {
            crate::log_debug!("天气组件：使用本地预报缓存，城市={}", city.id);
            return Ok(display_forecast(cached.expect("新鲜模块缓存已存在"), chrono::Utc::now().timestamp_millis()));
        }
        let mut deferred_errors = serde_json::Map::new(); // 受限模块保留失败状态，其他模块仍可刷新。
        {
            let retry = self.forecast_retry.lock().map_err(|_| "天气请求状态不可用")?;
            let now = chrono::Utc::now().timestamp(); // 每次读取只判断一次退避时间。
            stale_modules.retain(|module| {
                if let Some(state) = retry.modules.get(&format!("{}:{module}", city.id)).filter(|state| state.retry_at.is_none_or(|time| time > now))
                {
                    deferred_errors.insert((*module).to_string(), serde_json::json!(state.message));
                    return false;
                }
                true
            });
        }
        if stale_modules.is_empty()
        {
            return Ok(display_forecast(merge_forecast(cached, serde_json::json!({ "moduleErrors": deferred_errors }), &city), chrono::Utc::now().timestamp_millis()));
        }
        provider::check_credentials(&credentials).map_err(|error| {
            crate::log_warn!("天气组件：凭据校验失败，原因={error}");
            error
        })?;
        let revision = configuration.configuration_revision; // 网络开始时的配置身份。
        let cancelled = Arc::clone(&configuration.configuration_cancelled); // 配置保存或清除会取消共享请求。
        drop(configuration);
        let (mut fresh, failures) = Provider::new(Arc::new(credentials), cancelled).map_err(|failure| failure.message)?.forecast_modules(&city, &stale_modules, &is_active); // 网络期间释放配置锁。
        let configuration = self.inner.lock().map_err(|_| "天气设置状态不可用")?; // 验证与回写持有同一把锁，消除检查后切换配置的竞态。
        if configuration.configuration_revision != revision || !is_active()
        {
            return Err("天气配置已改变，旧预报已取消".into());
        }
        {
            let mut retry = self.forecast_retry.lock().map_err(|_| "天气请求状态不可用")?; // 成功与失败按城市模块独立更新。
            let now = chrono::Utc::now().timestamp();
            for module in &stale_modules
            {
                retry.record(format!("{}:{module}", city.id), failures.get(*module), now);
            }
        }
        fresh["moduleErrors"].as_object_mut().expect("Provider 始终返回模块错误表").extend(deferred_errors);
        let mut forecast = merge_forecast(cached, fresh, &city); // 请求耗时也计入磁盘保留期限。
        forecast["sourceKey"] = serde_json::json!(source_key); // 仅磁盘持有归属，返回快照会移除此字段。
        prune_forecast(&mut forecast, chrono::Utc::now().timestamp_millis());
        self.write_cached_forecast(&forecast)?;
        Ok(display_forecast(forecast, chrono::Utc::now().timestamp_millis()))
    }

    /// 读取并清理全部位置，既有请求串行锁保护集合读改写。
    fn next_refresh(&self, city: &City, value: &serde_json::Value) -> Result<Option<i64>, String>
    {
        if !credentials::configured(&self.target)?
        {
            return Ok(None);
        }
        let retry = self.forecast_retry.lock().map_err(|_| "天气请求状态不可用")?;
        let now = chrono::Utc::now().timestamp_millis(); // 错误初始化也采用有界重试，避免忙循环。
        Ok([("current", 30), ("hourly", 60), ("daily", 180)]
            .into_iter()
            .filter_map(|(module, minutes)| {
                if let Some(state) = retry.modules.get(&format!("{}:{module}", city.id))
                {
                    return state.retry_at.map(|time| time.saturating_mul(1000).max(now + 1000));
                }
                let age_deadline = value["moduleFetchedAt"][module].as_i64().map(|time| time + minutes * 60_000).unwrap_or(now + 60_000);
                let window = match module
                {
                    "hourly" => value["hourly"]["hours"][0]["forecastTime"].as_str().and_then(|time| chrono::DateTime::parse_from_rfc3339(time).ok()).map(|time| time.timestamp_millis() + 3_600_000),
                    "daily" => value["daily"]["days"][0]["forecastEndTime"].as_str().and_then(|time| chrono::DateTime::parse_from_rfc3339(time).ok()).map(|time| time.timestamp_millis()),
                    _ => None,
                };
                Some(window.map_or(age_deadline, |time| time.min(age_deadline)).max(now + 1000))
            })
            .min())
    }

    /// 读取并清理全部位置，既有请求串行锁保护集合读改写。
    fn cached_forecasts(&self) -> Result<ForecastCache, String>
    {
        let mut cache = crate::persistence::load_json::<ForecastCache>(&self.forecast_cache_path).map_err(|error| format!("天气缓存读取失败: {error}"))?.unwrap_or_default(); // 缺少集合时视为无当前格式缓存，不迁移旧结构。
        let mut changed = false; // 没有清理内容时不写盘。
        let now = chrono::Utc::now().timestamp_millis();
        cache.entries.retain_mut(|value| {
            changed |= prune_forecast(value, now);
            let retained = ["current", "hourly", "daily"].iter().any(|module| value[*module].is_object());
            changed |= !retained;
            retained
        });
        if changed
        {
            crate::persistence::save_json_atomic(&self.forecast_cache_path, &cache).map_err(|error| format!("天气缓存清理失败: {error}"))?;
        }
        Ok(cache)
    }

    /// 每个配置和城市只保存最近成功模块，其他位置不因刷新被覆盖。
    fn write_cached_forecast(&self, forecast: &serde_json::Value) -> Result<(), String>
    {
        let mut cache = self.cached_forecasts()?; // 同时清理其他位置超过七天的数据。
        cache.entries.retain(|value| value["cityId"] != forecast["city"]["id"] || value["sourceKey"] != forecast["sourceKey"]);
        if ["current", "hourly", "daily"].iter().any(|module| forecast[*module].is_object())
        {
            let mut entry = forecast.clone(); // 天气索引只保留位置标识，不保存历史城市详情。
            entry["cityId"] = forecast["city"]["id"].clone();
            entry.as_object_mut().expect("预报合并结果必须为对象").remove("city");
            cache.entries.push(entry); // 最新写入位于末尾，清除凭据后可恢复最近配置的离线数据。
        }
        crate::persistence::save_json_atomic(&self.forecast_cache_path, &cache).map_err(|error| format!("天气缓存保存失败: {error}"))
    }

    /// 读取指定配置和城市；无凭据时只返回该城市最近写入的保留数据。
    fn read_cached_forecast(&self, city: &City, source_key: Option<&str>) -> Result<Option<serde_json::Value>, String>
    {
        Ok(self.cached_forecasts()?.entries.into_iter().rev().find(|value| value["cityId"].as_str() == Some(city.id.as_str()) && source_key.is_none_or(|key| value["sourceKey"].as_str() == Some(key))).map(|mut value| {
            value.as_object_mut().expect("缓存条目具有城市标识").remove("cityId");
            value["city"] = serde_json::to_value(city).expect("城市必须可序列化");
            value
        }))
    }

    /// 使用已保存凭据搜索当前会话城市；结果只返回给内存中的天气格子。
    pub fn search_saved(&self, query: String) -> Result<Vec<City>, String>
    {
        let query = query.trim();
        if query.is_empty() || query.len() > 200
        {
            return Err("请输入不超过 200 字节的城市名称".into());
        }
        let configuration = self.inner.lock().map_err(|_| "天气设置状态不可用")?; // 城市结果与发起搜索的已保存配置绑定。
        let credentials = credentials::load(&self.target)?.ok_or("尚未配置天气凭据")?;
        let revision = configuration.configuration_revision;
        let cancelled = Arc::clone(&configuration.configuration_cancelled);
        drop(configuration);
        let result = Provider::new(Arc::new(credentials), cancelled).map_err(|failure| failure.message)?.search(query);
        let configuration = self.inner.lock().map_err(|_| "天气设置状态不可用")?; // 清除或替换凭据后不向页面返回旧城市结果。
        if configuration.configuration_revision != revision
        {
            return Err("天气配置已改变，旧城市搜索已取消".into());
        }
        result.map_err(|failure| failure.message)
    }
}

/// 磁盘仅保存各配置各位置的最近成功结果，不保存 GeoAPI 搜索集合。
#[derive(Default, Deserialize, Serialize)]
struct ForecastCache
{
    #[serde(default)]
    entries: Vec<serde_json::Value>, // 模块内容继续复用已有供应商结构与清理规则。
}

#[derive(Default)]
struct ForecastRetry
{
    modules: std::collections::BTreeMap<String, ModuleRetry>, // 同城同模块共享退避，不影响其他位置。
}

/// 单模块最近失败的策略；无重试时间表示等待配置重新验证。
#[derive(Default)]
struct ModuleRetry
{
    failures: u8,          // 网络失败阶梯，成功后清除。
    retry_at: Option<i64>, // 服务端等待优先，不以本地上限截断。
    message: String,       // 展示脱敏失败说明。
}

impl ForecastRetry
{
    /// 合并单模块结果，不让其他模块成功清除其失败退避。
    fn record(&mut self, key: String, failure: Option<&Failure>, now: i64)
    {
        let Some(failure) = failure
        else
        {
            self.modules.remove(&key);
            return;
        };
        let state = self.modules.entry(key).or_default(); // 保留连续失败次数。
        let delay = [60, 120, 300, 900, 1800][state.failures.min(4) as usize]; // 1、2、5、15、30 分钟。
        state.failures = state.failures.saturating_add(1);
        state.retry_at = failure.retry_at.or_else(|| failure.retryable.then(|| now.saturating_add(delay)));
        state.message = failure.message.clone();
    }
}

/// 模块数据与独立时间戳共同决定有效期，不从整包时间推断缺失模块。
fn module_fresh(value: &serde_json::Value, module: &str) -> bool
{
    let lifetime = match module
    {
        "current" => 30 * 60 * 1000,
        "hourly" => 60 * 60 * 1000,
        "daily" => 3 * 60 * 60 * 1000,
        _ => return false,
    };
    let timestamp = value.get("moduleFetchedAt").and_then(|times| times.get(module)).and_then(serde_json::Value::as_i64); // 不为缺失模块借用其他模块的刷新时间。
    let now = chrono::Utc::now().timestamp_millis(); // 未来时间戳不可视为有效缓存。
    if !timestamp.is_some_and(|timestamp| timestamp <= now && now.saturating_sub(timestamp) < lifetime)
    {
        return false;
    }
    let Some(city) = value.get("city").and_then(|city| serde_json::from_value::<City>(city.clone()).ok())
    else
    {
        return false;
    }; // 缓存必须保留城市时区，才能核对含今天的完整窗口。
    provider::validate_weather(module, &value[module], &city, chrono::Utc::now()).is_ok()
}

/// 将新请求的模块和旧缓存合并，保留可用模块与独立错误。
fn merge_forecast(cached: Option<serde_json::Value>, fresh: serde_json::Value, city: &City) -> serde_json::Value
{
    let mut merged = cached.filter(|value| value.is_object()).unwrap_or_else(|| serde_json::json!({ "city": city }));
    let object = merged.as_object_mut().expect("天气缓存必须是对象"); // 沿用当前缓存结构，只替换实际成功的模块。
    let mut times = object.get("moduleFetchedAt").and_then(serde_json::Value::as_object).cloned().unwrap_or_default(); // 保留未刷新模块的时间。
    let mut errors = object.get("moduleErrors").and_then(serde_json::Value::as_object).cloned().unwrap_or_default(); // 只由对应模块的新结果解除失败。
    if let Some(incoming) = fresh.as_object()
    {
        for module in ["current", "hourly", "daily"]
        {
            if let Some(value) = incoming.get(module).filter(|value| value.is_object())
            {
                object.insert(module.to_string(), value.clone());
                times.remove(module);
                if let Some(timestamp) = fresh["moduleFetchedAt"][module].as_i64()
                {
                    times.insert(module.into(), serde_json::json!(timestamp));
                }
                errors.remove(module);
            }
            else if let Some(error) = fresh["moduleErrors"][module].as_str()
            {
                errors.insert(module.into(), serde_json::json!(error));
            }
        }
        if let Some(attributions) = incoming.get("attributions").filter(|value| value.as_array().is_some_and(|items| !items.is_empty()))
        {
            object.insert("attributions".into(), attributions.clone());
        }
    }
    object.insert("fetchedAt".into(), serde_json::json!(times.values().filter_map(serde_json::Value::as_i64).min()));
    object.insert("moduleFetchedAt".into(), serde_json::Value::Object(times));
    object.insert("moduleErrors".into(), serde_json::Value::Object(errors));
    object.insert("city".into(), serde_json::to_value(city).expect("城市必须可序列化"));
    merged
}

/// 只过滤返回快照，磁盘仍按七天期限保留最近成功结果。
fn display_forecast(mut value: serde_json::Value, now: i64) -> serde_json::Value
{
    if let Some(object) = value.as_object_mut()
    {
        object.remove("sourceKey"); // 配置指纹只用于后端缓存，不广播到页面。
    }
    let allowed = value["moduleFetchedAt"]["current"].as_i64().is_some_and(|timestamp| timestamp <= now && now.saturating_sub(timestamp) < 24 * 60 * 60 * 1000); // 当前天气展示期限独立于磁盘期限。
    if !allowed
    {
        if let Some(object) = value.as_object_mut()
        {
            object.remove("current");
            let errors = object.entry("moduleErrors").or_insert_with(|| serde_json::json!({})); // 保留现有网络失败原因。
            if !errors.is_object()
            {
                *errors = serde_json::json!({});
            }
            errors.as_object_mut().expect("错误表已归一化").entry("current").or_insert_with(|| serde_json::json!("当前天气已超过展示期限，暂不可用"));
        }
    }
    value
}

/// 读取及写入时清理超过七天的模块，不保留无法判断年龄的数据。
fn prune_forecast(value: &mut serde_json::Value, now: i64) -> bool
{
    let Some(object) = value.as_object_mut()
    else
    {
        return false;
    }; // 非对象缓存不参与模块清理。
    let mut changed = false; // 没有变化时避免磁盘写入。
    for module in ["current", "hourly", "daily"]
    {
        let retained = object.get("moduleFetchedAt").and_then(|times| times.get(module)).and_then(serde_json::Value::as_i64).is_some_and(|timestamp| timestamp <= now && now.saturating_sub(timestamp) < 7 * 24 * 60 * 60 * 1000) && object.get(module).is_some_and(serde_json::Value::is_object); // 数据与有效时间戳必须同时存在。
        if retained
        {
            continue;
        }
        changed |= object.remove(module).is_some();
        // 最近请求错误独立保留，清理旧数据不能把本次失败变成加载中。
        if let Some(entries) = object.get_mut("moduleFetchedAt").and_then(serde_json::Value::as_object_mut)
        {
            changed |= entries.remove(module).is_some();
        }
    }
    if changed
    {
        let oldest = object.get("moduleFetchedAt").and_then(serde_json::Value::as_object).and_then(|times| times.values().filter_map(serde_json::Value::as_i64).min()); // 汇总时间只使用保留的模块。
        object.insert("fetchedAt".into(), serde_json::json!(oldest));
        if oldest.is_none()
        {
            object.remove("attributions");
        }
    }
    changed
}

/// 校验城市缓存的边界，避免把任意前端对象写入磁盘。
pub(crate) fn validate_city(city: &City) -> Result<(), String>
{
    let lat = city.lat.parse::<f64>().map_err(|_| "城市坐标无效")?;
    let lon = city.lon.parse::<f64>().map_err(|_| "城市坐标无效")?;
    if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) || city.tz.parse::<chrono_tz::Tz>().is_err() || city.id.is_empty() || city.name.is_empty() || [&city.id, &city.name, &city.adm1, &city.adm2, &city.country, &city.fx_link].iter().any(|value| value.len() > 1024)
    {
        return Err("城市数据无效".into());
    }
    let url = reqwest::Url::parse(&city.fx_link).map_err(|_| "城市详情链接无效")?;
    if url.scheme() != "https" || !matches!(url.host_str(), Some("www.qweather.com" | "qweather.com")) || !url.username().is_empty() || url.password().is_some() || url.port().is_some() || !url.path().starts_with("/weather/") || !url.path().ends_with(&format!("-{}.html", city.id))
    {
        return Err("城市详情链接无效".into());
    }
    Ok(())
}
