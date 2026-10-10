pub mod credentials;
mod provider;

use credentials::Credentials;
use provider::{City, Failure, Provider};
use serde::{Deserialize, Serialize};
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
    window_epoch: u64,        // 窗口销毁后拒绝尚未开始的排队命令。
    revision: u64,            // 会话递增身份。
    session: Option<Session>, // 关闭页面后清理。
    busy: bool,               // 包括取消后仍在收尾的 HTTP 请求。
    retry_at: i64,            // 不因换草稿绕过供应商限流。
}

/// 天气设置权威状态；无轮询、天气缓存或城市落盘。
pub struct WeatherState
{
    target: String,      // Windows Credential Manager 的配置归属键。
    inner: Mutex<Inner>, // 串行提交与会话状态。
}

impl WeatherState
{
    /// 在命令进入阻塞队列前记录当前窗口代次。
    pub fn window_epoch(&self) -> Result<u64, String> { Ok(self.inner.lock().map_err(|_| "天气设置状态不可用")?.window_epoch) }
    /// 使用本次启动的配置归属创建空会话。
    pub fn new(directory: &std::path::Path) -> Self { Self { target: credentials::target(directory), inner: Mutex::new(Inner::default()) } }

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
        let configured = credentials::configured(&self.target)?; // 不向前端回读账户或私钥。
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
                credentials::clear(&self.target)?;
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
                credentials::save(&self.target, &session.credentials)?;
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
}
