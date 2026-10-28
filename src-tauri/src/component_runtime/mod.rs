use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Condvar, LazyLock, Mutex};
use std::thread;
use std::time::Duration;

use chrono::Utc;
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager};

use crate::contracts::{COMPONENT_HOST_EVENT, ComponentCapability, ComponentCapabilityDeclaration, ComponentCapabilityResult, ComponentHostEvent, ComponentInstanceSnapshot, ComponentLoadStatus, ComponentManifest, ComponentRegistrationFailure, ComponentRegistrationSnapshot, ComponentRuntimeError, ComponentRuntimeSnapshot, WorkspaceRect};

/// 官方插件前端注册表当前支持的入口。
const LOCAL_CLOCK_ENTRY: &str = "official:local-clock";
const MUSIC_ENTRY: &str = "official:music-control";
const PLUGIN_SLOT_ENTRY: &str = "plugin:slot"; // 插件共享的精确入口，不接受任意 URL。

/// 内置官方插件清单来源。
const LOCAL_CLOCK_MANIFEST: &str = include_str!("../../components/official.local-clock.json");
const MUSIC_MANIFEST: &str = include_str!("../../components/official.music-control.json");
const COLLECT_MANIFEST: &str = include_str!("../../components/official.collect-box.json");
const WEATHER_MANIFEST: &str = include_str!("../../components/official.weather.json");

static TIME_CONSUMERS: LazyLock<(Mutex<HashMap<u64, String>>, Condvar)> = LazyLock::new(|| (Mutex::new(HashMap::new()), Condvar::new())); // 独立订阅身份映射到原生宿主。
static NEXT_TIME_SUBSCRIPTION: AtomicU64 = AtomicU64::new(1); // 重载后的身份不复用。

/// 持有官方与用户插件清单、活动实例注册结果和能力边界。
pub struct ComponentRuntimeState
{
    /// 启动时解析并冻结的插件来源与资产目录。
    plugin_directory: std::path::PathBuf,
    /// 已通过清单和能力校验的官方插件注册表。
    manifests: HashMap<String, ComponentManifest>,
    /// 已注册清单声明的创建与移除能力，按身份稳定排序。
    declarations: Vec<ComponentCapabilityDeclaration>,
    /// 单个清单失败但不阻塞其他插件的结果。
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

/// 提供官方插件注册、实例同步、能力调用和宿主事件。
impl ComponentRuntimeState
{
    /// 加载官方与插件目录清单，单个失败只形成隔离错误。
    pub fn new(plugin_directory: &std::path::Path) -> Self
    {
        use std::{fs, io::Read, os::windows::fs::MetadataExt};
        let mut sources = vec![("official.local-clock".to_string(), LOCAL_CLOCK_MANIFEST.to_string()), ("official.music-control".to_string(), MUSIC_MANIFEST.to_string()), ("official.collect-box".to_string(), COLLECT_MANIFEST.to_string()), ("official.weather".to_string(), WEATHER_MANIFEST.to_string())]; // 官方先插入，重复身份不会覆盖。
        let mut failures = Vec::new(); // 目录和单个文件错误不阻塞其余清单。
        if cfg!(debug_assertions)
        {
            sources.push(("official.debug-test".to_string(), include_str!("../../components/official.debug-test.json").to_string()));
        }
        match fs::read_dir(plugin_directory)
        {
            Ok(entries) =>
            {
                let mut paths = Vec::new(); // 稳定文件顺序也固定插件重复身份的优先级。
                for entry in entries
                {
                    match entry
                    {
                        Ok(entry) if entry.path().extension().is_some_and(|extension| extension == "json") => paths.push(entry.path()),
                        Ok(_) =>
                        {}
                        Err(error) => failures.push(ComponentRegistrationFailure { component_id: "plugin.directory".to_string(), message: format!("读取插件目录项失败: {error}") }),
                    }
                }
                paths.sort();
                for path in paths
                {
                    let fallback_id = path.file_stem().unwrap_or_default().to_string_lossy().into_owned(); // JSON 损坏时仍可定位来源。
                    let source = (|| -> std::io::Result<String> {
                        let metadata = fs::symlink_metadata(&path)?; // 不跟随插件清单链接与重解析点。
                        if !metadata.is_file() || metadata.file_attributes() & 0x400 != 0
                        {
                            return Err(std::io::Error::other("插件清单不支持链接、重解析点或目录"));
                        }
                        let mut bytes = Vec::new(); // 最多读取 64 KiB 加一个溢出哨兵字节。
                        fs::File::open(&path)?.take(65537).read_to_end(&mut bytes)?;
                        if bytes.len() > 65536
                        {
                            return Err(std::io::Error::other("插件清单超过 64 KiB"));
                        }
                        String::from_utf8(bytes).map_err(std::io::Error::other)
                    })(); // 每个文件独立有界读取，失败继续。
                    match source
                    {
                        Ok(source) => sources.push((fallback_id, source)),
                        Err(error) => failures.push(ComponentRegistrationFailure { component_id: fallback_id, message: format!("读取插件清单失败: {error}") }),
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound =>
            {}
            Err(error) => failures.push(ComponentRegistrationFailure { component_id: "plugin.directory".to_string(), message: format!("读取插件目录失败: {error}") }),
        }
        Self::from_manifest_sources(&sources, failures, plugin_directory)
    }

    /// 协议资产读取与启动清单使用同一个冻结目录，偏好变更下次启动生效。
    pub fn plugin_directory(&self) -> &std::path::Path { &self.plugin_directory }

    /// 返回成功注册清单对应的默认插件实例。
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
        if self.manifests.contains_key("official.collect-box")
        {
            instances.push(ComponentInstanceSnapshot { instance_id: "official.collect-box.default".to_string(), component_id: "official.collect-box".to_string(), monitor_id: String::new(), projection_monitor_id: None, projection_bounds: None, bounds: WorkspaceRect { left: 0.0, top: 300.0, width: 360.0, height: 280.0 }, state_version: 1, state: json!({}) });
        }
        if self.manifests.contains_key("official.debug-test")
        {
            instances.push(ComponentInstanceSnapshot { instance_id: "official.debug-test.default".to_string(), component_id: "official.debug-test".to_string(), monitor_id: String::new(), projection_monitor_id: None, projection_bounds: None, bounds: WorkspaceRect { left: 0.0, top: 16.0, width: 420.0, height: 270.0 }, state_version: 1, state: json!({}) });
        }
        instances
    }

    /// 返回全部已注册清单声明的创建与移除能力，供工作区命令校验可操作范围。
    pub fn capability_declarations(&self) -> &[ComponentCapabilityDeclaration] { &self.declarations }

    /// 校验并注册统一工作区的全部插件实例。
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
        let registration = snapshot.instances.iter().find(|instance| instance.instance_id == instance_id).ok_or_else(|| ComponentRuntimeError::invalid_instance("插件实例不存在"))?;
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
        crate::log_debug!("插件：宿主能力调用完成，实例={}，能力={capability:?}", instance_id);
        Ok(ComponentCapabilityResult { instance_id: instance_id.to_string(), capability, payload })
    }

    /// 返回最近一次已发布的插件宿主事件。
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

    /// 启动官方插件共享的低频宿主时间事件。
    pub fn start_host_events(app: AppHandle)
    {
        crate::log_info!("插件：宿主事件发布器已启动，间隔=1秒");
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
                let event = ComponentHostEvent { sequence: state.next_event_sequence.fetch_add(1, Ordering::Relaxed), kind: "time-tick".to_string(), unix_millis: Utc::now().timestamp_millis() }; // 已注册时间能力插件共享的宿主事件。
                let Ok(mut latest_event) = state.latest_host_event.lock()
                else
                {
                    crate::log_error!("插件：宿主事件状态不可用，事件发布器停止");
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
    fn from_manifest_sources(sources: &[(String, String)], mut failures: Vec<ComponentRegistrationFailure>, plugin_directory: &std::path::Path) -> Self
    {
        let mut manifests = HashMap::new(); // 已通过校验的插件清单。
        for (fallback_id, source) in sources
        {
            let manifest = serde_json::from_str::<ComponentManifest>(source).map_err(|error| ComponentRuntimeError::invalid_manifest(format!("插件清单 JSON 无效: {error}"))).and_then(|manifest| {
                validate_manifest(&manifest)?;
                Ok(manifest)
            });
            match manifest
            {
                Ok(manifest) if manifests.contains_key(&manifest.id) => failures.push(ComponentRegistrationFailure { component_id: manifest.id, message: "插件清单身份重复".to_string() }),
                Ok(manifest) =>
                {
                    crate::log_info!("插件：清单已注册，身份={}，入口={}，能力={}", manifest.id, manifest.entry, manifest.required_capabilities.len());
                    manifests.insert(manifest.id.clone(), manifest);
                }
                Err(error) =>
                {
                    crate::log_error!("插件：清单加载失败，身份={}，错误={}", fallback_id, error.message);
                    failures.push(ComponentRegistrationFailure { component_id: (*fallback_id).to_string(), message: error.message });
                }
            }
        }
        failures.sort_by(|first, second| first.component_id.cmp(&second.component_id));
        Self { plugin_directory: plugin_directory.to_path_buf(), declarations: capability_declarations(&manifests), manifests, failures, instances: Mutex::new(Vec::new()), revision: AtomicU64::new(1), next_event_sequence: AtomicU64::new(1), latest_host_event: Mutex::new(None) }
    }

    /// 将一个持久实例转换为成功或失败的隔离注册结果。
    fn registration_snapshot(&self, instance: &ComponentInstanceSnapshot) -> ComponentRegistrationSnapshot
    {
        let Some(manifest) = self.manifests.get(&instance.component_id)
        else
        {
            return failed_registration(instance, "插件清单不存在");
        };
        if instance.state_version != manifest.persistence_version
        {
            return failed_registration(instance, "插件持久状态版本不兼容");
        }
        ComponentRegistrationSnapshot { instance_id: instance.instance_id.clone(), component_id: instance.component_id.clone(), entry: Some(manifest.entry.clone()), authorized_capabilities: manifest.required_capabilities.clone(), status: ComponentLoadStatus::Ready, error: None }
    }
}

/// 提取已注册清单声明的创建与移除能力，并按插件身份稳定排序。
fn capability_declarations(manifests: &HashMap<String, ComponentManifest>) -> Vec<ComponentCapabilityDeclaration>
{
    let mut declarations = manifests.values().map(|manifest| ComponentCapabilityDeclaration { component_id: manifest.id.clone(), creatable: manifest.creatable, removable: manifest.removable }).collect::<Vec<_>>(); // 每个成功注册清单对应一项能力声明。
    declarations.sort_by(|first, second| first.component_id.cmp(&second.component_id));
    declarations
}

/// 校验官方插件清单身份、入口、能力和持久化版本。
fn validate_manifest(manifest: &ComponentManifest) -> Result<(), ComponentRuntimeError>
{
    if manifest.id.is_empty() || manifest.id.len() > 128 || !manifest.id.chars().all(|character| character.is_ascii_lowercase() || character.is_ascii_digit() || matches!(character, '.' | '-'))
    {
        return Err(ComponentRuntimeError::invalid_manifest("插件清单身份无效"));
    }
    if manifest.name.trim().is_empty() || manifest.name.chars().count() > 64
    {
        return Err(ComponentRuntimeError::invalid_manifest("插件清单名称无效"));
    }
    let clock_manifest = manifest.id == "official.local-clock" && manifest.entry == LOCAL_CLOCK_ENTRY; // 官方身份与入口必须成对。
    let music_manifest = manifest.id == "official.music-control" && manifest.entry == MUSIC_ENTRY;
    let debug_manifest = cfg!(debug_assertions) && manifest.id == "official.debug-test" && manifest.entry == "official:debug-test"; // 仅 Debug 可注册测试入口。
    let collect_manifest = manifest.id == "official.collect-box" && manifest.entry == "official:collect-box";
    let weather_manifest = manifest.id == "official.weather" && manifest.entry == "official:weather";
    let plugin_slot = manifest.id.starts_with("plugin.") && manifest.entry == PLUGIN_SLOT_ENTRY; // 插件只能进入宿主槽，不能选择 URL 或官方入口。
    if !clock_manifest && !music_manifest && !debug_manifest && !collect_manifest && !weather_manifest && !plugin_slot
    {
        return Err(ComponentRuntimeError::invalid_manifest("插件前端入口不受支持"));
    }
    if manifest.persistence_version == 0
    {
        return Err(ComponentRuntimeError::invalid_manifest("插件持久化版本无效"));
    }
    if (debug_manifest || collect_manifest || weather_manifest) && manifest.required_capabilities.is_empty()
    {
        return Ok(());
    }
    let capabilities = manifest.required_capabilities.iter().copied().collect::<HashSet<_>>(); // 清单中唯一的能力声明集合。
    if capabilities.len() != manifest.required_capabilities.len() || capabilities.is_empty()
    {
        return Err(ComponentRuntimeError::invalid_manifest("插件能力声明无效"));
    }
    if plugin_slot && (capabilities == HashSet::from([ComponentCapability::ReadHostTime]) || capabilities == HashSet::from([ComponentCapability::ReadCurrentMedia]))
    {
        return Ok(());
    }
    let expected = if music_manifest
    {
        ComponentCapability::ReadCurrentMedia
    }
    else
    {
        ComponentCapability::ReadHostTime
    }; // 各官方插件只授权自身需要的能力。
    if capabilities != HashSet::from([expected])
    {
        return Err(ComponentRuntimeError::invalid_manifest("插件请求了未授权宿主能力"));
    }
    Ok(())
}

/// 创建不影响其他实例的插件加载失败快照。
fn failed_registration(instance: &ComponentInstanceSnapshot, message: &str) -> ComponentRegistrationSnapshot { ComponentRegistrationSnapshot { instance_id: instance.instance_id.clone(), component_id: instance.component_id.clone(), entry: None, authorized_capabilities: Vec::new(), status: ComponentLoadStatus::Failed, error: Some(message.to_string()) } }

#[cfg(test)]
#[path = "mod.test.rs"]
mod tests;
