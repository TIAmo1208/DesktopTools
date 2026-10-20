pub(crate) mod activity;
pub mod credentials;
mod foreca;
mod provider;
pub mod source;

/// 按来源分派的天气请求入口；两个来源各自实现，不共用字段表。
enum Backend
{
    Qweather(Provider),
    Foreca(foreca::Foreca),
}

impl Backend
{
    /// 按凭据结构创建对应来源的实现。
    fn new(credentials: Arc<Credentials>, cancelled: Arc<AtomicBool>) -> Result<Self, Failure>
    {
        Ok(match &*credentials
        {
            Credentials::Qweather(value) => Backend::Qweather(Provider::new(Arc::new(value.clone()), cancelled)?),
            Credentials::Foreca(value) => Backend::Foreca(foreca::Foreca::new(Arc::new(value.clone()), cancelled)?),
        })
    }

    /// 手动搜索位置，结果只保留会话内存。
    fn search(&self, query: &str) -> Result<Vec<City>, Failure>
    {
        match self
        {
            Backend::Qweather(provider) => provider.search(query),
            Backend::Foreca(foreca) => foreca.search(query),
        }
    }

    /// 按来源声明的能力分别验证，检查项名称由来源决定。
    fn validate(&self, city: &City) -> Vec<Result<(), Failure>>
    {
        match self
        {
            Backend::Qweather(provider) => provider.validate(city),
            Backend::Foreca(foreca) => foreca.validate(city),
        }
    }

    /// 存活探测按来源只请求一次当前天气，用于切换前确认可用性。
    fn probe(&self, city: &City) -> Result<(), Failure>
    {
        match self
        {
            Backend::Qweather(provider) => provider.probe(city),
            Backend::Foreca(foreca) => foreca.probe(city),
        }
    }

    /// 只请求过期模块，部分成功也返回每个失败模块的请求策略。
    fn forecast_modules(&self, city: &City, modules: &[&str], is_active: impl Fn() -> bool) -> (serde_json::Value, std::collections::BTreeMap<String, Failure>)
    {
        match self
        {
            Backend::Qweather(provider) => provider.forecast_modules(city, modules, is_active),
            Backend::Foreca(foreca) => foreca.forecast_modules(city, modules, is_active),
        }
    }
}

use credentials::Credentials;
pub use provider::City;
use provider::{Failure, Provider};
use serde::{Deserialize, Serialize};
pub use source::Source;
use std::collections::BTreeMap;
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
    SaveCity
    {
        session_id: u64,
        city_id: String,
    },
    Cancel
    {
        session_id: u64,
    },
    Probe
    {
        source: Source,
    },
    Remove
    {
        source: Source,
    },
}

#[cfg(test)]
#[path = "mod.test.rs"]
mod tests;

/// 仅包含非敏感状态、临时城市及能力检查结果。
#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot
{
    session_id: Option<u64>,        // Rust 权威会话标识。
    cities: Vec<City>,              // 仅搜索命令返回临时结果。
    checks: Vec<Check>,             // 每项 failure 为 None 表示通过，空数组表示未验证。
    pub sources: Vec<SourceStatus>, // 设置页列表中的已配置来源，未配置来源不出现在列表。
}

/// 已配置来源的列表状态；只保留非敏感字段。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceStatus
{
    source: Source,                     // 来源标识。
    address: String,                    // 列表副标题与编辑回填使用的服务地址。
    identity: BTreeMap<String, String>, // 编辑模式可回填的非敏感字段。
    availability: Availability,         // 最近一次请求结果。
    cities: Vec<City>,                  // 该来源已添加的城市列表，随来源状态给前端。
}

/// 来源可用状态；未确认表示已配置但本次启动尚未请求过。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Availability
{
    #[default]
    Unconfirmed,
    Available,
    Unavailable,
}

/// 一项能力验证结果；名称随草稿来源，失败时保留脱敏说明。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Check
{
    label: String,            // 由来源决定的验证项名称。
    failure: Option<Failure>, // None 表示该项通过。
}

/// 每个来源的完整验证项；首项城市搜索由搜索结果本身判定，不单独发起请求。
fn check_labels(source: Source) -> &'static [&'static str]
{
    match source
    {
        Source::Qweather => &["城市搜索", "当前天气", "24 小时预报", "7 天预报", "来源与详情链接"],
        Source::Foreca => &["位置搜索", "当前天气", "24 小时预报", "7 天预报"],
    }
}

/// 存活探测的固定位置；探测只确认凭据可用于当前天气，不代表其余模块能力。
/// 这里的 `id` 不是真实位置标识，只用于占位：Foreca 的探测按经纬度寻址，改成按 `id` 寻址会让“尚未添加城市”时永远探测失败。
fn probe_city() -> City { City { id: "probe".into(), name: "北京".into(), adm1: String::new(), adm2: String::new(), country: String::new(), lat: "39.90".into(), lon: "116.40".into(), tz: "Asia/Shanghai".into(), fx_link: String::new() } }

/// 探测优先使用该来源已添加的第一座城市，列表为空才退回固定占位位置。
fn probe_city_from(cities: Vec<City>) -> City { cities.into_iter().next().unwrap_or_else(probe_city) }

/// 编辑态留空的敏感字段沿用已保存值；只在草稿字段为空时补齐，不覆盖用户新输入，明文不离开 Rust。
fn merge_saved_secrets(draft: Credentials, stored: Option<Credentials>) -> Credentials
{
    match (draft, stored)
    {
        (Credentials::Qweather(mut draft), Some(Credentials::Qweather(stored))) if draft.private_key.is_empty() =>
        {
            draft.private_key = stored.private_key.clone(); // 带 Drop 的类型不能移动字段，克隆值由草稿持有。
            Credentials::Qweather(draft)
        }
        (Credentials::Foreca(mut draft), Some(Credentials::Foreca(stored))) if draft.api_key.is_empty() =>
        {
            draft.api_key = stored.api_key.clone();
            Credentials::Foreca(draft)
        }
        (draft, _) => draft,
    }
}

/// 一份草稿承载本次编辑的凭据与临时搜索结果；保存不再以能力验证为门槛。
struct Session
{
    id: u64,                       // 防止旧页面和迟到回调覆盖新会话。
    credentials: Arc<Credentials>, // 不出现在普通快照中。
    cancelled: Arc<AtomicBool>,    // 后台请求检查取消标记。
    cities: Vec<City>,             // 不持久化的 GeoAPI 选择范围。
}

/// 同一设置窗口的短期草稿状态与来源可用状态。
#[derive(Default)]
struct Inner
{
    window_epoch: u64,                                              // 窗口销毁后拒绝尚未开始的排队命令。
    revision: u64,                                                  // 会话递增身份。
    session: Option<Session>,                                       // 关闭页面后清理。
    busy: bool,                                                     // 包括取消后仍在收尾的 HTTP 请求。
    retry_at: BTreeMap<Source, i64>,                                // 每个来源各自的限流等待，不因换草稿或换来源绕过。
    configurations: BTreeMap<Source, SourceConfiguration>,          // 每个来源各自的配置代次与取消标记。
    availability: std::collections::BTreeMap<Source, Availability>, // 每个来源最近一次请求结果。
}

/// 状态锁内的记账方法：只在已持有守卫时调用，因此这些方法不再自己加锁。
impl Inner
{
    /// 该来源的配置代次与取消标记。
    fn configuration(&mut self, source: Source) -> SourceConfiguration
    {
        let configuration = self.configurations.entry(source).or_default(); // 缺失来源按首次配置处理。
        SourceConfiguration { revision: configuration.revision, cancelled: Arc::clone(&configuration.cancelled) }
    }

    /// 作废该来源在途请求的旧配置结果，其他来源不受影响。
    fn cancel_configuration(&mut self, source: Source)
    {
        let configuration = self.configurations.entry(source).or_default();
        configuration.cancelled.store(true, Ordering::Release);
        configuration.cancelled = Arc::new(AtomicBool::new(false));
        configuration.revision += 1;
    }

    /// 记录一次实际请求结果，供设置页列表显示可用状态。
    fn observe(&mut self, source: Source, failure: Option<Failure>) { self.availability.insert(source, if failure.is_some() { Availability::Unavailable } else { Availability::Available }); }
}

/// 该来源是否仍处在服务端要求的等待期内；等待只约束发起请求的那个来源。
/// 单独取映射而不取整个 `Inner`，以便与已被借用的 `session` 字段并存。
fn retry_gate(retry_at: &BTreeMap<Source, i64>, source: Source) -> Result<(), String>
{
    let retry_at = retry_at.get(&source).copied().unwrap_or_default();
    if retry_at > chrono::Utc::now().timestamp()
    {
        return Err(format!("{} 服务端要求等待至 {} 后重试", source.key(), retry_at));
    }
    Ok(())
}

/// 一个来源的配置代次与在途请求身份；保存或清除只作废该来源的结果。
#[derive(Default)]
struct SourceConfiguration
{
    revision: u64,              // 该来源配置变化后的递增身份。
    cancelled: Arc<AtomicBool>, // 该来源已保存配置请求共用的取消标记。
}

/// 天气设置权威状态；城市与天气缓存均独立于工作区布局。
pub struct WeatherState
{
    namespace: String,                    // 凭据记录名前缀，固定为应用级记录；仅测试使用独立前缀。
    directory: PathBuf,                   // 天气配置目录，来源相关路径均按此目录派生。
    forecast_cache_path: PathBuf,         // 按配置指纹与城市标识保存的整包预报缓存路径。
    forecast_guard: Mutex<()>,            // 防止相同启动内的重复天气请求。
    forecast_retry: Mutex<ForecastRetry>, // 每个来源每个城市的短期退避。
    inner: Mutex<Inner>,                  // 串行提交、会话状态与按来源分开的配置代次。
}

/// 每个来源的城市列表独立成文件，不与其他来源共用路径。
impl Source
{
    /// 按当前配置目录派生该来源的城市列表文件。
    fn city_path(self, directory: &std::path::Path) -> PathBuf { directory.join(format!("weather-cities-{}.json", self.key())) }
}

/// 运行期状态按来源索引一次请求的最小键，同名城市跨来源互不覆盖。
fn retry_key(source: Source, city_id: &str, module: &str) -> String { format!("{}:{}:{}", source.key(), city_id, module) }

impl WeatherState
{
    /// 在命令进入阻塞队列前记录当前窗口代次。
    pub fn window_epoch(&self) -> Result<u64, String> { Ok(self.inner.lock().map_err(|_| "天气设置状态不可用")?.window_epoch) }
    /// 使用本次启动的配置目录创建空会话；来源全部由各入口显式传入。
    pub fn new(directory: &std::path::Path) -> Self { Self { namespace: credentials::target(Source::Qweather), directory: directory.to_path_buf(), forecast_cache_path: directory.join("weather-forecast.json"), forecast_guard: Mutex::new(()), forecast_retry: Mutex::new(ForecastRetry::default()), inner: Mutex::new(Inner::default()) } }

    /// 该来源在 Windows 凭据存储中的记录名称；和风沿用既有应用级记录。
    fn record(&self, source: Source) -> String { credentials::record_name(&self.namespace, source) } // 与凭据读写共用同一处取名，避免两侧各写一遍。

    /// 读取该来源的配置代次与取消标记；锁中毒时继续使用已有值。
    /// 只在未持有状态锁时调用；已持锁的位置直接用 `Inner::configuration`，避免同一线程二次加锁。
    fn configuration(&self, source: Source) -> SourceConfiguration
    {
        let mut inner = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        inner.configuration(source)
    }

    /// 作废该来源在途请求的旧配置结果，其他来源不受影响。
    fn cancel_configuration(&self, source: Source) { self.inner.lock().unwrap_or_else(|error| error.into_inner()).cancel_configuration(source) }

    /// 清空该来源全部城市的退避；其他来源的同名城市不受影响。
    fn clear_retry(&self, source: Source)
    {
        let ids = self.cities(source).unwrap_or_default().into_iter().map(|city| city.id).collect::<Vec<_>>(); // 只有该来源列表里的城市属于该来源。
        let Ok(mut retry) = self.forecast_retry.lock()
        else
        {
            return;
        };
        retry.modules.retain(|key, _| !ids.iter().any(|id| key.starts_with(&format!("{}:{}:", source.key(), id))));
    }

    /// 记录一次实际请求结果，供设置页列表显示可用状态。
    fn observe(&self, source: Source, failure: Option<Failure>) { self.inner.lock().unwrap_or_else(|error| error.into_inner()).observe(source, failure) }

    /// 设置页列表：只返回本机已保存凭据的来源，并附带编辑模式可回填的非敏感字段与该来源的城市列表。
    fn sources(&self, availability: &std::collections::BTreeMap<Source, Availability>) -> Result<Vec<SourceStatus>, String>
    {
        let mut list = Vec::new();
        for source in Source::ALL
        {
            // 一次凭据读取同时判定“是否已配置”并给出地址与回填字段：分两次读会出现前一次成功、后一次失败时的半残行。
            let credentials = match credentials::load(source, &self.record(source))
            {
                Ok(Some(credentials)) => credentials,
                Ok(None) =>
                {
                    continue; // 确实没有配置该来源。
                }
                Err(error) =>
                {
                    // 读取失败只影响本次列表，不能让状态查询整体失败；记日志以便区分“没配置”和“读不到”。
                    crate::log_warn!("天气来源列表：读取 {} 凭据失败，本次不列出该来源：{error}", source.key());
                    continue;
                }
            };
            // 地址与回填字段按该来源自己的凭据取值，不借用另一来源的内容。
            let (address, identity) = match (source, &credentials)
            {
                (Source::Qweather, Credentials::Qweather(value)) => (value.host.clone(), [("host", &value.host), ("developerId", &value.developer_id), ("projectId", &value.project_id), ("credentialId", &value.credential_id)].into_iter().map(|(key, field)| (key.to_string(), field.clone())).collect()),
                (Source::Foreca, _) => (foreca::ADDRESS.to_string(), Default::default()), // 该来源的地址固定，没有可回填字段。
                (Source::Qweather, _) => (String::new(), Default::default()),             // 结构不匹配时不显示其他来源的地址。
            };
            list.push(SourceStatus { source, address, identity, availability: availability.get(&source).copied().unwrap_or_default(), cities: self.cities(source)? });
        }
        Ok(list)
    }

    /// 读取该来源的用户城市列表；按城市标识去重，后写入的覆盖先写入的。
    pub fn cities(&self, source: Source) -> Result<Vec<City>, String>
    {
        let path = source.city_path(&self.directory); // 每个来源只读写自己的文件。
        let stored = crate::persistence::load_json::<Vec<City>>(&path).map_err(|error| format!("天气城市读取失败: {error}"))?.unwrap_or_default();
        Ok(stored.into_iter().fold(Vec::new(), |mut cities, city| {
            cities.retain(|existing: &City| existing.id != city.id);
            cities.push(city);
            cities
        }))
    }

    /// 按标识从该来源的城市列表解析城市；外部只提交两个标识，坐标与链接不在信任范围内。
    fn city(&self, source: Source, city_id: &str) -> Result<City, String> { self.cities(source)?.into_iter().find(|city| city.id == city_id).ok_or("该城市不在当前来源的城市列表中，请重新选择".into()) }

    /// 解析城市并按该来源规则校验；验证命令与添加命令共用同一条信任边界。
    pub fn validated_city(&self, source: Source, city_id: &str) -> Result<City, String>
    {
        let city = self.city(source, city_id)?;
        validate_city_for_source(source, &city)?;
        Ok(city)
    }

    /// 按格子绑定的来源与城市标识读取缓存；城市对象由运行期状态从该来源列表解析。
    pub fn forecast_for_city(&self, source: Source, city_id: &str) -> Result<serde_json::Value, String> { self.forecast(source, self.city(source, city_id)?) }

    /// 按来源验证候选城市：搜索命中不能证明可用的来源用已保存凭据真实请求一次当前天气。
    pub fn validate_for_source(&self, source: Source, city: &City) -> Result<(), String>
    {
        validate_city_for_source(source, city)?;
        if source.search_hit_proves_weather()
        {
            return Ok(()); // 搜索命中即可用的来源只校验字段与链接，不额外消耗一次请求。
        }
        self.probe_saved(source, city)
    }

    /// 用该来源已保存凭据请求一次当前天气；失败写回可用状态与等待时间，不改变城市列表。
    fn probe_saved(&self, source: Source, city: &City) -> Result<(), String>
    {
        retry_gate(&self.inner.lock().map_err(|_| "天气设置状态不可用")?.retry_at, source)?; // 该来源的等待期先于任何请求。
        let configuration = self.configuration(source); // 本次验证所属的配置代次。
        let credentials = credentials::load(source, &self.record(source))?.ok_or("该来源尚未配置凭据")?;
        let result = Backend::new(Arc::new(credentials), configuration.cancelled).and_then(|backend| backend.probe(city));
        if self.configuration(source).revision != configuration.revision
        {
            return Err("天气配置已改变，旧城市验证已取消".into()); // 配置更换后的结果不作为本次结论。
        }
        self.observe(source, result.as_ref().err().cloned()); // 与列表探测共用可用状态记账。
        if let Some(retry_at) = result.as_ref().err().and_then(|failure| failure.retry_at)
        {
            self.record_retry(source, retry_at); // 服务端要求等待时同样约束该来源的后续请求。
        }
        result.map_err(|failure| failure.message)
    }

    /// 把城市写入该来源的列表，同一标识重复添加不产生重复项。
    pub fn save_city(&self, source: Source, city: City) -> Result<City, String>
    {
        validate_city_for_source(source, &city)?;
        let mut cities = self.cities(source)?; // 保留该来源已有城市，只替换同标识项。
        cities.retain(|existing| existing.id != city.id);
        cities.push(city.clone());
        crate::persistence::save_json_atomic(&source.city_path(&self.directory), &cities).map_err(|error| format!("天气城市保存失败: {error}"))?;
        Ok(city)
    }

    /// 按标识从该来源的城市列表移除一项并返回更新后的完整列表；正在使用该城市的格子保留自己的配置。
    pub fn remove_city(&self, source: Source, city_id: &str) -> Result<Vec<City>, String>
    {
        let mut cities = self.cities(source)?; // 只读写该来源自己的文件。
        let before = cities.len(); // 删除前的项数，用于区分标识不存在。
        cities.retain(|city| city.id != city_id);
        if cities.len() == before
        {
            return Err("该城市不在当前来源的城市列表中，请刷新后重试".into());
        }
        crate::persistence::save_json_atomic(&source.city_path(&self.directory), &cities).map_err(|error| format!("天气城市保存失败: {error}"))?;
        Ok(cities)
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
        let mut snapshot = Snapshot { sources: self.sources(&inner.availability)?, ..Default::default() }; // 本次命令结果。
        match command
        {
            Command::Status | Command::Cancel { .. } => return Ok(snapshot),
            Command::Begin { credentials } =>
            {
                let source = credentials.source(); // 草稿来源决定合并哪一条已保存记录。
                let stored = credentials::load(source, &self.record(source)).ok().flatten(); // 凭据读取失败不阻断编辑，留空的敏感字段仍会被格式校验拒绝。
                let credentials = merge_saved_secrets(credentials, stored); // 编辑态留空表示沿用已保存值。
                provider::check_credentials(&credentials)?;
                if let Some(session) = inner.session.take()
                {
                    session.cancelled.store(true, Ordering::Release);
                }
                inner.revision += 1;
                let id = inner.revision; // 每次草稿使用新身份。
                inner.session = Some(Session { id, credentials: Arc::new(credentials), cancelled: Arc::new(AtomicBool::new(false)), cities: Vec::new() });
                snapshot.session_id = Some(id);
                return Ok(snapshot);
            }
            Command::Remove { source } =>
            {
                credentials::clear(&self.record(source))?; // 只清除该来源的记录。
                inner.availability.remove(&source); // 没有凭据时不再报告旧可用状态。
                if let Some(session) = inner.session.take()
                {
                    session.cancelled.store(true, Ordering::Release);
                }
                drop(inner);
                self.clear_retry(source); // 该来源的退避不再适用。
                self.cancel_configuration(source); // 只作废该来源在途请求的旧配置结果。
                let inner = self.inner.lock().map_err(|_| "天气设置状态不可用")?;
                snapshot.sources = self.sources(&inner.availability)?;
                crate::log_info!("天气设置：已删除来源凭据，来源={}", source.key());
                return Ok(snapshot);
            }
            Command::Save { session_id } =>
            {
                // 保存只要求草稿身份有效：是否可用由来源可用状态与城市列表体现，不以能力验证为门槛。
                let session = inner.session.as_ref().filter(|s| s.id == session_id && !s.cancelled.load(Ordering::Acquire)).ok_or("验证会话已失效，请重新编辑")?;
                if inner.busy
                {
                    return Err("天气验证仍在执行".into());
                }
                let source = session.credentials.source(); // 记录名跟随草稿来源，不写入其他来源的记录。
                credentials::save(&self.record(source), &session.credentials)?;
                inner.availability.remove(&source); // 新配置尚未请求过，回到未确认状态。
                if let Some(session) = inner.session.take()
                {
                    session.cancelled.store(true, Ordering::Release);
                }
                drop(inner);
                self.clear_retry(source); // 新配置经过完整验证后只恢复该来源的请求。
                self.cancel_configuration(source); // 只作废该来源在途请求的旧配置结果。
                let inner = self.inner.lock().map_err(|_| "天气设置状态不可用")?;
                snapshot.sources = self.sources(&inner.availability)?;
                crate::log_info!("天气设置：验证通过的凭据已安全保存");
                return Ok(snapshot);
            }
            Command::Search { .. } | Command::Validate { .. } | Command::Probe { .. } | Command::SaveCity { .. } =>
            {}
        }
        if inner.busy
        {
            return Err("已有天气请求正在执行，请稍后重试".into());
        }
        if let Command::Probe { source } = &command
        {
            let source = *source; // 探测目标与草稿会话无关。
            let credentials = credentials::load(source, &self.record(source))?.ok_or("该来源尚未配置凭据")?; // 只用已保存配置探测。
            let configuration = inner.configuration(source); // 本函数已持有状态锁，通过 Inner 的记账方法取值，不会二次加锁。
            let city = probe_city_from(self.cities(source)?); // 有城市时用该来源的第一座城市探测，未添加城市时退回固定位置。
            retry_gate(&inner.retry_at, source)?; // 只有该来源被限流时不发起新请求，其他来源不受影响。
            inner.busy = true;
            drop(inner);
            crate::log_debug!("天气设置：开始探测来源可用性，来源={}，城市={}", source.key(), city.name);
            let result = Backend::new(Arc::new(credentials), configuration.cancelled).and_then(|backend| backend.probe(&city)); // 一次当前天气请求。
            let mut inner = self.inner.lock().map_err(|_| "天气设置状态不可用")?;
            inner.busy = false;
            if inner.configurations.entry(source).or_default().revision == configuration.revision
            {
                inner.observe(source, result.as_ref().err().cloned()); // 与搜索、验证共用同一份可用状态记账。
            }
            if let Some(retry_at) = result.as_ref().err().and_then(|failure| failure.retry_at)
            {
                let current = inner.retry_at.get(&source).copied().unwrap_or_default();
                inner.retry_at.insert(source, current.max(retry_at)); // 探测收到的等待时间同样约束该来源后续请求。
            }
            snapshot.sources = self.sources(&inner.availability)?;
            result.map_err(|failure| failure.message)?;
            return Ok(snapshot);
        }
        // 会话命令把验证通过的城市写入该来源的城市列表，不改变草稿与凭据。
        if let Command::SaveCity { session_id, city_id } = &command
        {
            let session = inner.session.as_ref().filter(|s| s.id == *session_id && !s.cancelled.load(Ordering::Acquire)).ok_or("验证会话已失效，请重新选择城市")?; // 身份校验先于写入。
            let source = session.credentials.source(); // 写入本次草稿所属来源的列表，不使用其他来源的记录。
            let city = session.cities.iter().find(|city| &city.id == city_id).cloned().ok_or("请选择当前搜索结果中的城市")?; // 只接受该草稿本次搜索返回的城市。
            drop(inner);
            let saved = self.save_city(source, city)?;
            let inner = self.inner.lock().map_err(|_| "天气设置状态不可用")?;
            snapshot.sources = self.sources(&inner.availability)?;
            snapshot.cities = vec![saved]; // 前端按本次写入结果更新展开区。
            crate::log_info!("天气设置：验证通过的城市已写入来源列表，来源={}", source.key());
            return Ok(snapshot);
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
        let source = inner.session.as_ref().filter(|s| s.id == session_id).map(|session| session.credentials.source()).ok_or("验证会话已失效，请重新选择城市")?; // 先按只读借用取出来源，才能随后按来源查限流等待。
        retry_gate(&inner.retry_at, source)?; // 限流等待按来源生效，另一个来源仍可正常搜索与验证。
        let session = inner.session.as_mut().filter(|s| s.id == session_id).ok_or("验证会话已失效，请重新选择城市")?; // 身份校验先于请求。
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
        let result = Backend::new(credentials, cancelled).map(|backend| {
            if let Some(query) = query
            {
                (backend.search(query.trim()), Vec::new())
            }
            else
            {
                (Ok(Vec::new()), backend.validate(city.as_ref().expect("验证命令已经检查城市")))
            }
        }); // 每次动作只有一个后台任务。
        let mut inner = self.inner.lock().map_err(|_| "天气设置状态不可用")?; // 提交时重新核验会话。
        inner.busy = false;
        let (cities, checks) = result.map_err(|failure| failure.message)?; // 客户端创建失败也释放互斥。
        for failure in cities.as_ref().err().into_iter().chain(checks.iter().filter_map(|r| r.as_ref().err()))
        {
            if let Some(retry_at) = failure.retry_at
            {
                let current = inner.retry_at.get(&source).copied().unwrap_or_default();
                inner.retry_at.insert(source, current.max(retry_at)); // 只推迟该来源，不阻塞其他来源。
            }
        }
        let session = inner.session.as_mut().filter(|s| s.id == session_id && !s.cancelled.load(Ordering::Acquire)).ok_or("验证已取消或草稿已改变")?; // 拒绝取消后的结果。
        let cities = cities.map_err(|failure| failure.message)?; // 搜索失败不保留旧城市集合。
        if !checks.is_empty()
        {
            // 网络验证项加首项搜索构成本来源的完整清单；项数与 provider 实际返回的数量必须相等，否则标签会与结果错位。
            let labels = check_labels(source);
            let failures = std::iter::once(None).chain(checks.into_iter().map(Result::err)).collect::<Vec<_>>();
            debug_assert_eq!(labels.len(), failures.len(), "验证项标签数量与请求项数量不一致");
            snapshot.checks = labels.iter().zip(failures).map(|(label, failure)| Check { label: (*label).to_string(), failure }).collect();
        }
        else
        {
            session.cities = cities.clone();
        }
        snapshot.cities = cities;
        snapshot.session_id = Some(session_id);
        crate::log_debug!("天气设置：会话结果已提交");
        Ok(snapshot)
    }

    /// 页面按格子绑定的来源只读取缓存；网络更新由共享活动调度器负责。
    pub fn forecast(&self, source: Source, city: City) -> Result<serde_json::Value, String>
    {
        validate_city(&city)?;
        let _guard = self.forecast_guard.lock().map_err(|_| "天气请求状态不可用")?; // 与调度器磁盘写入串行。
        let _configuration = self.inner.lock().map_err(|_| "天气设置状态不可用")?;
        let credentials = credentials::load(source, &self.record(source))?; // 只使用本次请求来源的凭据。
        let key = credentials.as_ref().map(credentials::cache_identity).transpose()?;
        let cached = self.read_cached_forecast(&city, key.as_deref())?;
        Ok(display_forecast(cached.unwrap_or_else(|| serde_json::json!({"city": city})), chrono::Utc::now().timestamp_millis()))
    }

    /// 调度器按活动状态刷新，网络期间每个模块开始前重新核对活动条件。
    fn refresh(&self, source: Source, city: City, is_active: impl Fn() -> bool) -> Result<serde_json::Value, String>
    {
        let _forecast_guard = self.forecast_guard.lock().map_err(|_| "天气请求状态不可用")?;
        let configuration = self.inner.lock().map_err(|_| "天气设置状态不可用")?; // 该来源的配置代次与本次缓存读取、凭据获取使用同一个短事务。
        let credentials = credentials::load(source, &self.record(source))?; // 无凭据时仅允许展示期限内的旧缓存。
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
            crate::log_debug!("天气组件：使用本地预报缓存，来源={}，城市={}", source.key(), city.id);
            return Ok(display_forecast(cached.expect("新鲜模块缓存已存在"), chrono::Utc::now().timestamp_millis()));
        }
        let mut deferred_errors = serde_json::Map::new(); // 受限模块保留失败状态，其他模块仍可刷新。
        {
            let retry = self.forecast_retry.lock().map_err(|_| "天气请求状态不可用")?;
            let now = chrono::Utc::now().timestamp(); // 每次读取只判断一次退避时间。
            stale_modules.retain(|module| {
                if let Some(state) = retry.modules.get(&retry_key(source, &city.id, module)).filter(|state| state.retry_at.is_none_or(|time| time > now))
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
        let revision = configuration.configurations.get(&source).map_or(0, |value| value.revision); // 网络开始时该来源的配置身份，其他来源变化不作废本次请求。
        let cancelled = configuration.configurations.get(&source).map_or_else(|| Arc::new(AtomicBool::new(false)), |value| Arc::clone(&value.cancelled)); // 只有该来源的保存或清除会取消本次请求。
        drop(configuration);
        let (mut fresh, failures) = Backend::new(Arc::new(credentials), cancelled).map_err(|failure| failure.message)?.forecast_modules(&city, &stale_modules, &is_active); // 网络期间释放配置锁。
        self.observe(source, failures.values().next().cloned()); // 已保存配置的实际请求结果更新列表可用状态。
        let configuration = self.inner.lock().map_err(|_| "天气设置状态不可用")?; // 验证与回写持有同一把锁，消除检查后切换配置的竞态。
        if configuration.configurations.get(&source).map_or(0, |value| value.revision) != revision || !is_active()
        {
            return Err("天气配置已改变，旧预报已取消".into());
        }
        {
            let mut retry = self.forecast_retry.lock().map_err(|_| "天气请求状态不可用")?; // 成功与失败按来源、城市、模块独立更新。
            let now = chrono::Utc::now().timestamp();
            for module in &stale_modules
            {
                retry.record(retry_key(source, &city.id, module), failures.get(*module), now);
            }
        }
        fresh["moduleErrors"].as_object_mut().expect("Provider 始终返回模块错误表").extend(deferred_errors);
        let mut forecast = merge_forecast(cached, fresh, &city); // 请求耗时也计入磁盘保留期限。
        forecast["sourceKey"] = serde_json::json!(source_key); // 仅磁盘持有归属，返回快照会移除此字段。
        prune_forecast(&mut forecast, chrono::Utc::now().timestamp_millis());
        self.write_cached_forecast(&forecast)?;
        Ok(display_forecast(forecast, chrono::Utc::now().timestamp_millis()))
    }

    /// 读取该来源该城市的模块截止时间，既有请求串行锁保护集合读改写。
    fn next_refresh(&self, source: Source, city: &City, value: &serde_json::Value) -> Result<Option<i64>, String>
    {
        if !credentials::configured(&self.record(source))?
        {
            return Ok(None);
        }
        let retry = self.forecast_retry.lock().map_err(|_| "天气请求状态不可用")?;
        let now = chrono::Utc::now().timestamp_millis(); // 错误初始化也采用有界重试，避免忙循环。
        Ok([("current", 30), ("hourly", 60), ("daily", 180)]
            .into_iter()
            .filter_map(|(module, minutes)| {
                if let Some(state) = retry.modules.get(&retry_key(source, &city.id, module))
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

    /// 记录服务端要求的最早重试时间，只推迟该来源。
    fn record_retry(&self, source: Source, retry_at: i64)
    {
        if let Ok(mut inner) = self.inner.lock()
        {
            let current = inner.retry_at.get(&source).copied().unwrap_or_default();
            inner.retry_at.insert(source, current.max(retry_at));
        }
    }

    /// 使用该来源已保存凭据搜索当前会话城市；结果只返回给内存中的天气格子。
    pub fn search_saved(&self, source: Source, query: String) -> Result<Vec<City>, String>
    {
        let query = query.trim();
        if query.is_empty() || query.len() > 200
        {
            return Err("请输入不超过 200 字节的城市名称".into());
        }
        retry_gate(&self.inner.lock().map_err(|_| "天气设置状态不可用")?.retry_at, source)?; // 真实搜索入口同样遵守该来源的等待期。
        let configuration = self.configuration(source); // 城市结果与发起搜索的该来源已保存配置绑定。
        let credentials = credentials::load(source, &self.record(source))?.ok_or("尚未配置天气凭据")?;
        let revision = configuration.revision; // 本次搜索期间的该来源配置身份。
        let result = Backend::new(Arc::new(credentials), configuration.cancelled).map_err(|failure| failure.message)?.search(query);
        if self.configuration(source).revision != revision
        {
            return Err("天气配置已改变，旧城市搜索已取消".into()); // 清除或替换该来源凭据后不向页面返回旧城市结果。
        }
        self.observe(source, result.as_ref().err().cloned()); // 实际搜索结果更新列表可用状态。
        if let Some(retry_at) = result.as_ref().err().and_then(|failure| failure.retry_at)
        {
            self.record_retry(source, retry_at); // 服务端要求等待时，后续搜索与探测都先被拒绝。
        }
        result.map_err(|failure| failure.message)
    }
}

#[cfg(test)]
impl WeatherState
{
    /// 测试专用构造：使用独立记录名前缀，不读写用户真实凭据；来源全部由入口传入。
    pub fn for_test(directory: &std::path::Path, namespace: &str) -> Self
    {
        let mut state = Self::new(directory);
        state.namespace = namespace.to_string();
        state
    }

    /// 测试读取该来源当前的配置代次，用于核对保存与清除的隔离范围。
    fn config_revision(&self, source: Source) -> u64 { self.inner.lock().unwrap().configurations.get(&source).map_or(0, |value| value.revision) }

    /// 测试读取该来源最近添加的城市；列表为空表示尚未选择。
    pub fn saved_city(&self, source: Source) -> Result<Option<City>, String> { Ok(self.cities(source)?.pop()) }
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

/// 校验各来源共用的城市规则；详情链接规则由各来源实现。
pub(crate) fn validate_city(city: &City) -> Result<(), String>
{
    let lat = city.lat.parse::<f64>().map_err(|_| "城市坐标无效")?;
    let lon = city.lon.parse::<f64>().map_err(|_| "城市坐标无效")?;
    if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) || city.tz.parse::<chrono_tz::Tz>().is_err() || city.id.is_empty() || city.name.is_empty() || [&city.id, &city.name, &city.adm1, &city.adm2, &city.country, &city.fx_link].iter().any(|value| value.len() > 1024)
    {
        return Err("城市数据无效".into());
    }
    Ok(())
}

/// 按来源校验城市边界：公共规则之后跟随该来源的详情链接规则。
pub(crate) fn validate_city_for_source(source: Source, city: &City) -> Result<(), String>
{
    validate_city(city)?;
    match source
    {
        Source::Qweather => provider::validate_city_detail(city).map_err(str::to_string),
        Source::Foreca => Ok(()), // 该来源没有城市天气网页，空链接合法。
    }
}
