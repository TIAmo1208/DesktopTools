use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Condvar, LazyLock, Mutex};
use std::thread;
use std::time::Duration;

use chrono::Utc;
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager};

use crate::contracts::{COMPONENT_HOST_EVENT, ComponentCapability, ComponentCapabilityResult, ComponentHostEvent, ComponentInstanceSnapshot, ComponentLoadStatus, ComponentManifest, ComponentRegistrationFailure, ComponentRegistrationSnapshot, ComponentRuntimeError, ComponentRuntimeSnapshot, WorkspaceRect};

/// 官方组件前端注册表当前支持的入口。
const LOCAL_CLOCK_ENTRY: &str = "official:local-clock";
const MUSIC_ENTRY: &str = "official:music-control";

/// 内置官方组件清单来源。
const LOCAL_CLOCK_MANIFEST: &str = include_str!("../../components/official.local-clock.json");
const MUSIC_MANIFEST: &str = include_str!("../../components/official.music-control.json");

static TIME_CONSUMERS: LazyLock<(Mutex<HashMap<u64, String>>, Condvar)> = LazyLock::new(|| (Mutex::new(HashMap::new()), Condvar::new())); // 独立订阅身份映射到原生宿主。
static NEXT_TIME_SUBSCRIPTION: AtomicU64 = AtomicU64::new(1); // 重载后的身份不复用。

/// 持有官方组件清单、活动实例注册结果和能力边界。
pub struct ComponentRuntimeState
{
    /// 已通过清单和能力校验的官方组件注册表。
    manifests: HashMap<String, ComponentManifest>,
    /// 单个清单失败但不阻塞其他组件的结果。
    failures: Vec<ComponentRegistrationFailure>,
    /// 统一工作区的实例注册结果。
    instances: Mutex<Vec<ComponentRegistrationSnapshot>>,
    /// 注册表或实例集合变化时递增的修订号。
    revision: AtomicU64,
    /// 下一个宿主事件序号。
    next_event_sequence: AtomicU64,
    /// 独立 Windowed WebView2 transport 可读取的最近宿主事件。
    latest_host_event: Mutex<Option<ComponentHostEvent>>,
}

/// 提供官方组件注册、实例同步、能力调用和宿主事件。
impl ComponentRuntimeState
{
    /// 加载全部官方组件清单，单个失败只形成隔离错误。
    pub fn new() -> Self
    {
        let mut sources = vec![("official.local-clock", LOCAL_CLOCK_MANIFEST), ("official.music-control", MUSIC_MANIFEST)]; // Release 只注册正式功能。
        if cfg!(debug_assertions)
        {
            sources.push(("official.debug-test", include_str!("../../components/official.debug-test.json")));
        }
        Self::from_manifest_sources(&sources)
    }

    /// 返回成功注册清单对应的默认组件实例。
    pub fn default_instances(&self) -> Vec<ComponentInstanceSnapshot>
    {
        if !self.manifests.contains_key("official.local-clock")
        {
            return Vec::new();
        }
        let mut instances = vec![ComponentInstanceSnapshot { instance_id: "official.local-clock.default".to_string(), component_id: "official.local-clock".to_string(), monitor_id: String::new(), projection_monitor_id: None, projection_bounds: None, bounds: WorkspaceRect { left: 0.0, top: 0.0, width: 220.0, height: 116.0 }, state_version: 1, state: json!({"format": "24-hour"}) }]; // 保持既有时钟实例身份和持久状态。
        if self.manifests.contains_key("official.music-control")
        {
            instances.push(ComponentInstanceSnapshot { instance_id: "official.music-control.default".to_string(), component_id: "official.music-control".to_string(), monitor_id: String::new(), projection_monitor_id: None, projection_bounds: None, bounds: WorkspaceRect { left: 0.0, top: 140.0, width: 220.0, height: 150.0 }, state_version: 1, state: json!({}) });
        }
        if self.manifests.contains_key("official.debug-test")
        {
            instances.push(ComponentInstanceSnapshot { instance_id: "official.debug-test.default".to_string(), component_id: "official.debug-test".to_string(), monitor_id: String::new(), projection_monitor_id: None, projection_bounds: None, bounds: WorkspaceRect { left: 0.0, top: 16.0, width: 420.0, height: 270.0 }, state_version: 1, state: json!({}) });
        }
        instances
    }

    /// 校验并注册统一工作区的全部组件实例。
    pub fn snapshot(&self, instances: &[ComponentInstanceSnapshot]) -> Result<ComponentRuntimeSnapshot, ComponentRuntimeError>
    {
        let registrations = instances.iter().filter(|instance| cfg!(debug_assertions) || instance.component_id != "official.debug-test").map(|instance| self.registration_snapshot(instance)).collect::<Vec<_>>(); // 当前布局中每个实例互相隔离的注册结果。
        let mut current = self.instances.lock().map_err(|_| ComponentRuntimeError::state_unavailable())?; // 等待发布的实例注册集合。
        if *current != registrations
        {
            *current = registrations;
            self.revision.fetch_add(1, Ordering::Relaxed);
        }
        let mut manifests = self.manifests.values().cloned().collect::<Vec<_>>(); // 稳定排序后发布的官方清单集合。
        manifests.sort_by(|first, second| first.id.cmp(&second.id));
        Ok(ComponentRuntimeSnapshot { revision: self.revision.load(Ordering::Relaxed), manifests, instances: current.clone(), failures: self.failures.clone() })
    }

    /// 校验实例声明后执行一个已授权宿主能力。
    pub fn invoke_capability(&self, instances: &[ComponentInstanceSnapshot], instance_id: &str, capability: ComponentCapability) -> Result<ComponentCapabilityResult, ComponentRuntimeError>
    {
        let snapshot = self.snapshot(instances)?; // 能力调用前同步的当前实例注册结果。
        let registration = snapshot.instances.iter().find(|instance| instance.instance_id == instance_id).ok_or_else(|| ComponentRuntimeError::invalid_instance("组件实例不存在"))?;
        if registration.status != ComponentLoadStatus::Ready || !registration.authorized_capabilities.contains(&capability)
        {
            return Err(ComponentRuntimeError::capability_denied(capability));
        }
        let payload = match capability
        {
            ComponentCapability::ReadHostTime => json!({
                "unixMillis": Utc::now().timestamp_millis()
            }),
            ComponentCapability::ReadCurrentMedia => crate::platform::windows::music::snapshot().map_err(|message| ComponentRuntimeError { code: "media-read-failed".to_string(), message })?,
        }; // 当前授权能力的最小结构化结果。
        crate::log_debug!("组件：宿主能力调用完成，实例={}，能力={capability:?}", instance_id);
        Ok(ComponentCapabilityResult { instance_id: instance_id.to_string(), capability, payload })
    }

    /// 返回最近一次已发布的组件宿主事件。
    pub fn current_host_event(&self) -> Result<Option<ComponentHostEvent>, ComponentRuntimeError> { Ok(Some(ComponentHostEvent { sequence: self.next_event_sequence.fetch_add(1, Ordering::Relaxed), kind: "time-tick".to_string(), unix_millis: Utc::now().timestamp_millis() })) }

    /// 登记已经通过命令层实例与能力校验的时间消费者。
    pub(crate) fn subscribe_time(monitor: String) -> u64
    {
        let id = NEXT_TIME_SUBSCRIPTION.fetch_add(1, Ordering::Relaxed); // 本次订阅身份。
        TIME_CONSUMERS.0.lock().unwrap_or_else(|error| error.into_inner()).insert(id, monitor);
        TIME_CONSUMERS.1.notify_one();
        id
    }

    /// 只解除对应身份，迟到的清理不影响新实例。
    pub(crate) fn unsubscribe_time(id: u64)
    {
        TIME_CONSUMERS.0.lock().unwrap_or_else(|error| error.into_inner()).remove(&id);
        TIME_CONSUMERS.1.notify_one();
    }

    /// 页面重载与宿主销毁清理该屏全部时间消费者。
    pub(crate) fn clear_time_monitor(monitor: &str)
    {
        TIME_CONSUMERS.0.lock().unwrap_or_else(|error| error.into_inner()).retain(|_, value| value != monitor);
        TIME_CONSUMERS.1.notify_one();
    }

    /// 原生桥只向仍有活动消费者的宿主交付时间事件。
    pub(crate) fn has_time_consumers(monitor: &str) -> bool { TIME_CONSUMERS.0.lock().unwrap_or_else(|error| error.into_inner()).values().any(|value| value == monitor) }

    /// 暂停策略变化唤醒计时源重新判断活动消费者。
    pub(crate) fn wake_time_source()
    {
        let consumers_guard = TIME_CONSUMERS.0.lock().unwrap_or_else(|error| error.into_inner()); // 与条件等待配对，防止丢失唤醒。
        std::hint::black_box(&consumers_guard); // 锁必须保持到唤醒通知完成。
        TIME_CONSUMERS.1.notify_one();
    }

    /// 启动官方组件共享的低频宿主时间事件。
    pub fn start_host_events(app: AppHandle)
    {
        crate::log_info!("组件：宿主事件发布器已启动，间隔=1秒");
        thread::spawn(move || {
            loop
            {
                let mut consumers = TIME_CONSUMERS.0.lock().unwrap_or_else(|error| error.into_inner()); // 空闲时阻塞，不按秒检查。
                while !consumers.values().any(|monitor| !crate::platform::windows::refresh_policy::is_paused(monitor))
                {
                    consumers = TIME_CONSUMERS.1.wait(consumers).unwrap_or_else(|error| error.into_inner());
                }
                let (consumers, elapsed) = TIME_CONSUMERS.1.wait_timeout(consumers, Duration::from_secs(1)).unwrap_or_else(|error| error.into_inner()); // 订阅变化打断周期并重新判定。
                if !elapsed.timed_out() || !consumers.values().any(|monitor| !crate::platform::windows::refresh_policy::is_paused(monitor))
                {
                    continue;
                }
                drop(consumers);
                let Some(state) = app.try_state::<ComponentRuntimeState>()
                else
                {
                    break;
                };
                if !app.state::<crate::app::settings::SettingsState>().preferences().is_ok_and(|preferences| preferences.clock_enabled)
                {
                    continue;
                }
                let event = ComponentHostEvent { sequence: state.next_event_sequence.fetch_add(1, Ordering::Relaxed), kind: "time-tick".to_string(), unix_millis: Utc::now().timestamp_millis() }; // 已注册时间能力组件共享的宿主事件。
                let Ok(mut latest_event) = state.latest_host_event.lock()
                else
                {
                    crate::log_error!("组件：宿主事件状态不可用，事件发布器停止");
                    break;
                }; // 供独立 Windowed transport 轮询的最近事件。
                *latest_event = Some(event.clone());
                drop(latest_event);
                if app.emit(COMPONENT_HOST_EVENT, event).is_err()
                {
                    break;
                }
            }
        });
    }

    /// 从一组独立清单来源建立容错官方注册表。
    fn from_manifest_sources(sources: &[(&str, &str)]) -> Self
    {
        let mut manifests = HashMap::new(); // 已通过校验的官方组件清单。
        let mut failures = Vec::new(); // 不阻塞其他组件的清单失败集合。
        for (fallback_id, source) in sources
        {
            let manifest = serde_json::from_str::<ComponentManifest>(source).map_err(|error| ComponentRuntimeError::invalid_manifest(format!("组件清单 JSON 无效: {error}"))).and_then(|manifest| {
                validate_manifest(&manifest)?;
                Ok(manifest)
            });
            match manifest
            {
                Ok(manifest) if manifests.contains_key(&manifest.id) => failures.push(ComponentRegistrationFailure { component_id: manifest.id, message: "组件清单身份重复".to_string() }),
                Ok(manifest) =>
                {
                    crate::log_info!("组件：官方清单已注册，身份={}，入口={}，能力={}", manifest.id, manifest.entry, manifest.required_capabilities.len());
                    manifests.insert(manifest.id.clone(), manifest);
                }
                Err(error) =>
                {
                    crate::log_error!("组件：官方清单加载失败，身份={}，错误={}", fallback_id, error.message);
                    failures.push(ComponentRegistrationFailure { component_id: (*fallback_id).to_string(), message: error.message });
                }
            }
        }
        Self { manifests, failures, instances: Mutex::new(Vec::new()), revision: AtomicU64::new(1), next_event_sequence: AtomicU64::new(1), latest_host_event: Mutex::new(None) }
    }

    /// 将一个持久实例转换为成功或失败的隔离注册结果。
    fn registration_snapshot(&self, instance: &ComponentInstanceSnapshot) -> ComponentRegistrationSnapshot
    {
        let Some(manifest) = self.manifests.get(&instance.component_id)
        else
        {
            return failed_registration(instance, "组件清单不存在");
        };
        if instance.state_version != manifest.persistence_version
        {
            return failed_registration(instance, "组件持久状态版本不兼容");
        }
        ComponentRegistrationSnapshot { instance_id: instance.instance_id.clone(), component_id: instance.component_id.clone(), entry: Some(manifest.entry.clone()), authorized_capabilities: manifest.required_capabilities.clone(), status: ComponentLoadStatus::Ready, error: None }
    }
}

/// 校验官方组件清单身份、入口、能力和持久化版本。
fn validate_manifest(manifest: &ComponentManifest) -> Result<(), ComponentRuntimeError>
{
    if manifest.id.is_empty() || manifest.id.len() > 128 || !manifest.id.chars().all(|character| character.is_ascii_lowercase() || character.is_ascii_digit() || matches!(character, '.' | '-'))
    {
        return Err(ComponentRuntimeError::invalid_manifest("组件清单身份无效"));
    }
    if manifest.name.trim().is_empty() || manifest.name.chars().count() > 64
    {
        return Err(ComponentRuntimeError::invalid_manifest("组件清单名称无效"));
    }
    let debug_manifest = cfg!(debug_assertions) && manifest.id == "official.debug-test" && manifest.entry == "official:debug-test"; // 仅 Debug 可注册测试入口。
    if manifest.entry != LOCAL_CLOCK_ENTRY && manifest.entry != MUSIC_ENTRY && !debug_manifest
    {
        return Err(ComponentRuntimeError::invalid_manifest("组件前端入口不受支持"));
    }
    if manifest.persistence_version == 0
    {
        return Err(ComponentRuntimeError::invalid_manifest("组件持久化版本无效"));
    }
    if debug_manifest && manifest.required_capabilities.is_empty()
    {
        return Ok(());
    }
    let capabilities = manifest.required_capabilities.iter().copied().collect::<HashSet<_>>(); // 清单中唯一的能力声明集合。
    if capabilities.len() != manifest.required_capabilities.len() || capabilities.is_empty()
    {
        return Err(ComponentRuntimeError::invalid_manifest("组件能力声明无效"));
    }
    let expected = if manifest.id == "official.music-control" && manifest.entry == MUSIC_ENTRY
    {
        ComponentCapability::ReadCurrentMedia
    }
    else
    {
        ComponentCapability::ReadHostTime
    }; // 各官方组件只授权自身需要的能力。
    if capabilities != HashSet::from([expected])
    {
        return Err(ComponentRuntimeError::invalid_manifest("组件请求了未授权宿主能力"));
    }
    Ok(())
}

/// 创建不影响其他实例的组件加载失败快照。
fn failed_registration(instance: &ComponentInstanceSnapshot, message: &str) -> ComponentRegistrationSnapshot { ComponentRegistrationSnapshot { instance_id: instance.instance_id.clone(), component_id: instance.component_id.clone(), entry: None, authorized_capabilities: Vec::new(), status: ComponentLoadStatus::Failed, error: Some(message.to_string()) } }

#[cfg(test)]
#[path = "mod.test.rs"]
mod tests;
