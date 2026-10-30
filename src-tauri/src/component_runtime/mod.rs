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
    /// 启动时解析的插件来源与资产目录。安装和卸载成功后按这个目录重扫。
    plugin_directory: std::path::PathBuf,
    /// 已通过校验的清单、能力声明和隔离失败。重扫时整表替换，不碰实例。
    catalog: Mutex<ComponentCatalog>,
    /// 统一工作区的实例注册结果。
    instances: Mutex<Vec<ComponentRegistrationSnapshot>>,
    /// 注册表或实例集合变化时递增的修订号。
    revision: AtomicU64,
    /// 下一个宿主事件序号。
    next_event_sequence: AtomicU64,
    /// 独立 Windowed WebView2 transport 可读取的最近宿主事件。
    latest_host_event: Mutex<Option<ComponentHostEvent>>,
}

/// 一份可整体替换的清单表。官方清单和插件清单放在一起。
struct ComponentCatalog
{
    manifests: HashMap<String, ComponentManifest>,     // 已通过校验的清单。
    declarations: Vec<ComponentCapabilityDeclaration>, // 按身份排序的创建与移除能力。
    failures: Vec<ComponentRegistrationFailure>,       // 单个清单失败，不挡住其余。
}

/// 提供官方插件注册、实例同步、能力调用和宿主事件。
impl ComponentRuntimeState
{
    /// 加载官方与插件目录清单，单个失败只形成隔离错误。
    pub fn new(plugin_directory: &std::path::Path) -> Self
    {
        let mut sources = vec![("official.local-clock".to_string(), LOCAL_CLOCK_MANIFEST.to_string()), ("official.music-control".to_string(), MUSIC_MANIFEST.to_string()), ("official.collect-box".to_string(), COLLECT_MANIFEST.to_string()), ("official.weather".to_string(), WEATHER_MANIFEST.to_string())]; // 官方先插入，重复身份不会覆盖。
        let mut failures = Vec::new(); // 目录和单个文件错误不阻塞其余清单。
        if cfg!(debug_assertions)
        {
            sources.push(("official.debug-test".to_string(), include_str!("../../components/official.debug-test.json").to_string()));
        }
        let plugins = load_plugin_folders(plugin_directory, &mut failures); // 只认一层子目录里的 manifest.ini，根上的旧 json 不读。
        Self::from_manifest_sources(&sources, failures, plugins, plugin_directory)
    }

    /// 协议资产读取与清单扫描使用同一个目录，偏好变更下次启动生效。
    pub fn plugin_directory(&self) -> &std::path::Path { &self.plugin_directory }

    /// 重新读取插件根。只替换清单和失败项，不改工作区，也不清已登记的实例。
    pub fn reload_plugins(&self)
    {
        let fresh = Self::new(&self.plugin_directory); // 官方清单照原样重建，插件文件夹重新扫描。
        *self.catalog.lock().unwrap_or_else(|error| error.into_inner()) = fresh.catalog.into_inner().unwrap_or_else(|error| error.into_inner());
        self.revision.fetch_add(1, Ordering::Relaxed);
    }

    fn catalog(&self) -> std::sync::MutexGuard<'_, ComponentCatalog> { self.catalog.lock().unwrap_or_else(|error| error.into_inner()) }

    /// 返回成功注册清单对应的默认插件实例。
    pub fn default_instances(&self) -> Vec<ComponentInstanceSnapshot>
    {
        let catalog = self.catalog(); // 默认实例只看当前清单里有没有官方插件。
        if !catalog.manifests.contains_key("official.local-clock")
        {
            return Vec::new();
        }
        let mut instances = vec![ComponentInstanceSnapshot { instance_id: "official.local-clock.default".to_string(), component_id: "official.local-clock".to_string(), monitor_id: String::new(), projection_monitor_id: None, projection_bounds: None, bounds: WorkspaceRect { left: 0.0, top: 0.0, width: 220.0, height: 116.0 }, state_version: 1, state: json!({"format": "24-hour"}) }]; // 保持既有时钟实例身份和持久状态。
        if catalog.manifests.contains_key("official.music-control")
        {
            instances.push(ComponentInstanceSnapshot { instance_id: "official.music-control.default".to_string(), component_id: "official.music-control".to_string(), monitor_id: String::new(), projection_monitor_id: None, projection_bounds: None, bounds: WorkspaceRect { left: 0.0, top: 140.0, width: 220.0, height: 150.0 }, state_version: 1, state: json!({}) });
        }
        if catalog.manifests.contains_key("official.collect-box")
        {
            instances.push(ComponentInstanceSnapshot { instance_id: "official.collect-box.default".to_string(), component_id: "official.collect-box".to_string(), monitor_id: String::new(), projection_monitor_id: None, projection_bounds: None, bounds: WorkspaceRect { left: 0.0, top: 300.0, width: 360.0, height: 280.0 }, state_version: 1, state: json!({}) });
        }
        if catalog.manifests.contains_key("official.debug-test")
        {
            instances.push(ComponentInstanceSnapshot { instance_id: "official.debug-test.default".to_string(), component_id: "official.debug-test".to_string(), monitor_id: String::new(), projection_monitor_id: None, projection_bounds: None, bounds: WorkspaceRect { left: 0.0, top: 16.0, width: 420.0, height: 270.0 }, state_version: 1, state: json!({}) });
        }
        instances
    }

    /// 返回全部已注册清单声明的创建与移除能力，供工作区命令校验可操作范围。
    pub fn capability_declarations(&self) -> Vec<ComponentCapabilityDeclaration> { self.catalog().declarations.clone() }

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
        let catalog = self.catalog(); // 发布当前清单，不在这里改工作区。
        let mut manifests = catalog.manifests.values().cloned().collect::<Vec<_>>(); // 稳定排序后发布的清单集合。
        manifests.sort_by(|first, second| first.id.cmp(&second.id));
        let failures = catalog.failures.clone();
        drop(catalog);
        Ok(ComponentRuntimeSnapshot { revision: self.revision.load(Ordering::Relaxed), manifests, instances: current.clone(), failures })
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
    fn from_manifest_sources(sources: &[(String, String)], mut failures: Vec<ComponentRegistrationFailure>, plugins: Vec<ComponentManifest>, plugin_directory: &std::path::Path) -> Self
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
        for manifest in plugins
        {
            if manifests.contains_key(&manifest.id)
            {
                failures.push(ComponentRegistrationFailure { component_id: manifest.id, message: "插件清单身份重复".to_string() });
            }
            else
            {
                crate::log_info!("插件：清单已注册，身份={}，脚本={}", manifest.id, manifest.script);
                manifests.insert(manifest.id.clone(), manifest);
            }
        }
        failures.sort_by(|first, second| first.component_id.cmp(&second.component_id));
        for failure in &failures
        {
            crate::log_warn!("插件：清单注册失败，身份={}，原因={}", failure.component_id, failure.message); // 失败只留日志与消息，不占桌面。
        }
        let declarations = capability_declarations(&manifests); // 跟清单一起替换的能力声明。
        Self { plugin_directory: plugin_directory.to_path_buf(), catalog: Mutex::new(ComponentCatalog { manifests, declarations, failures }), instances: Mutex::new(Vec::new()), revision: AtomicU64::new(1), next_event_sequence: AtomicU64::new(1), latest_host_event: Mutex::new(None) }
    }

    /// 将一个持久实例转换为成功或失败的隔离注册结果。
    fn registration_snapshot(&self, instance: &ComponentInstanceSnapshot) -> ComponentRegistrationSnapshot
    {
        let Some(manifest) = self.catalog().manifests.get(&instance.component_id).cloned()
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

/// 按插件标识找到已通过校验的文件夹清单。目录不存在时没有插件。
pub(crate) fn find_plugin_package(directory: &std::path::Path, plugin_id: &str) -> Result<Option<ComponentManifest>, String>
{
    let mut failures = Vec::new(); // 查找单个插件时不把别人的错误变成这次的失败。
    let plugins = load_plugin_folders(directory, &mut failures);
    Ok(plugins.into_iter().find(|manifest| manifest.id == plugin_id))
}

/// 读取插件根下一层子目录中的 manifest.ini。根上的文件直接忽略。
fn load_plugin_folders(directory: &std::path::Path, failures: &mut Vec<ComponentRegistrationFailure>) -> Vec<ComponentManifest>
{
    use std::{fs, io::Read, os::windows::fs::MetadataExt};
    let mut folders = Vec::new(); // 按文件夹名稳定排序后再注册。
    match fs::read_dir(directory)
    {
        Ok(entries) =>
        {
            for entry in entries
            {
                match entry
                {
                    Ok(entry) => folders.push(entry.path()),
                    Err(error) => failures.push(ComponentRegistrationFailure { component_id: "plugin.directory".to_string(), message: format!("读取插件目录项失败: {error}") }),
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(error) =>
        {
            failures.push(ComponentRegistrationFailure { component_id: "plugin.directory".to_string(), message: format!("读取插件目录失败: {error}") });
            return Vec::new();
        }
    }
    folders.sort();
    let mut plugins = Vec::new(); // 通过校验的插件清单。
    for folder in folders
    {
        let folder_name = folder.file_name().and_then(|name| name.to_str()).unwrap_or("").to_string(); // 失败记录用的文件夹名。
        let metadata = match fs::symlink_metadata(&folder)
        {
            Ok(metadata) => metadata,
            Err(error) =>
            {
                failures.push(ComponentRegistrationFailure { component_id: folder_name, message: format!("读取插件文件夹失败: {error}") });
                continue;
            }
        };
        if !metadata.is_dir() || metadata.file_attributes() & 0x400 != 0
        {
            continue;
        }
        let manifest_path = folder.join("manifest.ini"); // 每个插件文件夹只认这一个文件。
        if !manifest_path.is_file()
        {
            continue;
        }
        let source = (|| -> std::io::Result<String> {
            let metadata = fs::symlink_metadata(&manifest_path)?; // 不跟随清单链接。
            if !metadata.is_file() || metadata.file_attributes() & 0x400 != 0
            {
                return Err(std::io::Error::other("插件清单不支持链接、重解析点或目录"));
            }
            let mut bytes = Vec::new(); // 最多读取 64 KiB 加一个溢出哨兵字节。
            fs::File::open(&manifest_path)?.take(65537).read_to_end(&mut bytes)?;
            if bytes.len() > 65536
            {
                return Err(std::io::Error::other("插件清单超过 64 KiB"));
            }
            String::from_utf8(bytes).map_err(std::io::Error::other)
        })();
        let manifest = source.map_err(|error| error.to_string()).and_then(|text| plugin_manifest_from_ini(&folder_name, &folder, &text));
        match manifest
        {
            Ok(manifest) => plugins.push(manifest),
            Err(error) => failures.push(ComponentRegistrationFailure { component_id: folder_name, message: error }),
        }
    }
    plugins
}

/// 把 manifest.ini 的 [plugin] 节变成运行时清单。ini 的 entry 是脚本相对路径，不是前端分派键。
fn plugin_manifest_from_ini(folder_name: &str, folder: &std::path::Path, text: &str) -> Result<ComponentManifest, String>
{
    let values = parse_plugin_ini(text)?; // 只读取 [plugin] 节。
    let name = required_ini(&values, "name")?;
    if name != folder_name
    {
        return Err("插件文件夹名与名称不一致".to_string());
    }
    validate_plugin_folder_name(&name)?;
    let version = required_ini(&values, "version")?;
    parse_plugin_version(&version).ok_or_else(|| "插件版本无效".to_string())?;
    let id = required_ini(&values, "id")?;
    if !id.starts_with("plugin.") || id.len() == "plugin.".len() || id.len() > 128 || !id.chars().all(|character| character.is_ascii_lowercase() || character.is_ascii_digit() || matches!(character, '.' | '-'))
    {
        return Err("插件标识无效".to_string());
    }
    let entry = required_ini(&values, "entry")?;
    confirm_plugin_entry(folder, &entry)?;
    let app_version = parse_plugin_version(env!("CARGO_PKG_VERSION")).ok_or_else(|| "程序版本无效".to_string())?; // 与 package_info 使用同一份 Cargo 版本。
    if let Some(minimum) = optional_version(&values, "minAppVersion")?
    {
        if app_version < minimum
        {
            return Err(format!("插件要求的程序版本过高，需要 {}，当前 {}", values["minAppVersion"], env!("CARGO_PKG_VERSION")));
        }
    }
    if let Some(maximum) = optional_version(&values, "maxAppVersion")?
    {
        if app_version > maximum
        {
            return Err(format!("插件不支持当前程序版本，最高 {}，当前 {}", values["maxAppVersion"], env!("CARGO_PKG_VERSION")));
        }
    }
    let script = format!("{folder_name}/{}", entry.replace('\\', "/")); // 插件根相对路径，用正斜杠交给前端。
    let max_tiles = plugin_max_tiles(&id, values.get("maxTiles").map(String::as_str)); // 缺省和 0 都不限。
    let creatable = plugin_flag(&id, "creatable", values.get("creatable").map(String::as_str), false); // 缺省不能添加到桌面。
    let removable = plugin_flag(&id, "removable", values.get("removable").map(String::as_str), true); // 缺省允许用户移除格子。
    let repository = plugin_repository(&id, values.get("repository").map(String::as_str)); // 非法地址视同未声明。
    let description = plugin_description(values.get("description").map(String::as_str)); // ini 按行切，换行写成 \n。
    let author = values.get("author").map(|value| value.trim().to_string()).unwrap_or_default(); // 缺省没有作者。
    let manifest = ComponentManifest { id, name, entry: PLUGIN_SLOT_ENTRY.to_string(), script, required_capabilities: Vec::new(), creatable, removable, persistence_version: 1, max_tiles, repository, description, author };
    validate_manifest(&manifest).map_err(|error| error.message)?;
    Ok(manifest)
}

/// 只接受 1/0 和 true/false。缺省不记警告；空串和其他值记警告后用缺省。
fn plugin_flag(plugin_id: &str, key: &str, raw: Option<&str>, default: bool) -> bool
{
    let Some(raw) = raw
    else
    {
        return default;
    };
    match raw.trim().to_ascii_lowercase().as_str()
    {
        "1" | "true" => true,
        "0" | "false" => false,
        value =>
        {
            crate::log_warn!("插件：{key} 无效，按缺省处理，插件={plugin_id}，值={value}");
            default
        }
    }
}

/// 现有 ini 不支持跨行，简介里的换行写成 \n，读的时候还原。
fn plugin_description(raw: Option<&str>) -> String { raw.map(str::trim).filter(|value| !value.is_empty()).unwrap_or("").replace("\\n", "\n") }

/// 只保留 http 或 https。空值和非法值都视同未声明。
pub(crate) fn plugin_repository(plugin_id: &str, raw: Option<&str>) -> String
{
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty())
    else
    {
        return String::new();
    };
    let https = raw.len() >= 8 && raw[..8].eq_ignore_ascii_case("https://");
    let http = raw.len() >= 7 && raw[..7].eq_ignore_ascii_case("http://");
    if https || http
    {
        return raw.to_string();
    }
    crate::log_warn!("插件：repository 无效，已忽略，插件={plugin_id}，值={raw}");
    String::new()
}

/// 打开仓库时只使用清单里留下的地址。
pub(crate) fn declared_plugin_repository(repository: &str) -> Result<&str, String>
{
    if repository.is_empty()
    {
        return Err("该插件没有声明仓库".to_string());
    }
    Ok(repository)
}

/// 缺省和 0 表示不限。负数、非数字和放不下的值记警告后按不限，不拒绝清单。
pub(crate) fn plugin_max_tiles(plugin_id: &str, raw: Option<&str>) -> u32
{
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty())
    else
    {
        return 0;
    };
    match raw.parse::<i64>()
    {
        Ok(value) if (1..=i64::from(u32::MAX)).contains(&value) => value as u32,
        Ok(0) => 0,
        _ =>
        {
            crate::log_warn!("插件：maxTiles 无效，按不限处理，插件={plugin_id}，值={raw}");
            0
        }
    }
}

/// 已有格子数达到上限时拒绝再创建。0 表示不限。
pub(crate) fn enforce_plugin_tile_limit(max_tiles: u32, count: usize) -> Result<(), String>
{
    if max_tiles == 0 || count < max_tiles as usize
    {
        return Ok(());
    }
    Err(format!("已达到该插件的格子上限（{max_tiles} 个）"))
}

/// 名称要能直接作为 Windows 文件夹名。
fn validate_plugin_folder_name(name: &str) -> Result<(), String>
{
    if name.is_empty() || name.chars().count() > 80 || name.chars().any(|character| character.is_control() || matches!(character, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
    {
        return Err("插件名称无效".to_string());
    }
    Ok(())
}

/// 入口必须是文件夹内的普通脚本，拒绝绝对路径、父级跳转、链接和重解析点。
fn confirm_plugin_entry(folder: &std::path::Path, entry: &str) -> Result<(), String>
{
    use std::os::windows::fs::MetadataExt;
    use std::path::Component;
    let relative = std::path::Path::new(entry); // 不把反斜杠或盘符当成文件夹内路径。
    if entry.is_empty() || entry.contains(['\\', ':', '%', '\0']) || relative.is_absolute() || relative.components().any(|part| !matches!(part, Component::Normal(_))) || relative.extension().and_then(|value| value.to_str()) != Some("js")
    {
        return Err("插件入口脚本无效".to_string());
    }
    let candidate = folder.join(relative); // 尚未跟随链接的脚本。
    let mut missing = false; // 脚本文件可以先不存在，加载失败交给前端。
    for ancestor in candidate.ancestors().take_while(|ancestor| ancestor.starts_with(folder))
    {
        let metadata = match std::fs::symlink_metadata(ancestor)
        {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && ancestor == candidate =>
            {
                missing = true;
                continue;
            }
            Err(error) => return Err(format!("读取插件入口脚本失败：{error}")),
        };
        if metadata.file_type().is_symlink() || metadata.file_attributes() & 0x400 != 0
        {
            return Err("插件入口脚本不能是链接或重解析点".to_string());
        }
    }
    if missing
    {
        crate::log_warn!("插件：入口脚本不存在，仍注册清单，入口={entry}");
        return Ok(());
    }
    let root = std::fs::canonicalize(folder).map_err(|error| format!("读取插件入口脚本失败：{error}"))?;
    let path = std::fs::canonicalize(&candidate).map_err(|error| format!("读取插件入口脚本失败：{error}"))?;
    if !path.starts_with(&root) || !path.is_file()
    {
        return Err("插件入口脚本必须位于插件文件夹内".to_string());
    }
    Ok(())
}

/// 解析三段数字版本。空文本表示没有这条边界。
fn optional_version(values: &HashMap<String, String>, key: &str) -> Result<Option<(u64, u64, u64)>, String>
{
    let Some(value) = values.get(key).map(|value| value.trim()).filter(|value| !value.is_empty())
    else
    {
        return Ok(None);
    };
    parse_plugin_version(value).map(Some).ok_or_else(|| "插件要求的程序版本无效".to_string())
}

/// 只接受 `主.次.修订`。
fn parse_plugin_version(value: &str) -> Option<(u64, u64, u64)>
{
    let mut parts = value.split('.');
    let version = (parts.next()?.parse().ok()?, parts.next()?.parse().ok()?, parts.next()?.parse().ok()?);
    parts.next().is_none().then_some(version)
}

/// 读取必填键。插件自带的 id 若被安装流程改写，这里读到的已是宿主写入的值。
fn required_ini(values: &HashMap<String, String>, key: &str) -> Result<String, String> { values.get(key).map(|value| value.trim().to_string()).filter(|value| !value.is_empty()).ok_or_else(|| format!("插件清单缺少 {key}")) }

/// 只解析 UTF-8 ini 的 [plugin] 节。
fn parse_plugin_ini(text: &str) -> Result<HashMap<String, String>, String>
{
    let mut section = false; // 当前行是否属于 [plugin]。
    let mut values = HashMap::new(); // 该节里的键。
    for line in text.trim_start_matches('\u{feff}').lines()
    {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';')
        {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|value| value.strip_suffix(']'))
        {
            section = name.trim().eq_ignore_ascii_case("plugin");
            continue;
        }
        if !section
        {
            continue;
        }
        let Some((key, value)) = line.split_once('=')
        else
        {
            return Err("插件清单格式无效".to_string());
        };
        values.insert(key.trim().to_string(), value.trim().to_string());
    }
    if values.is_empty()
    {
        return Err("插件清单缺少 [plugin]".to_string());
    }
    Ok(values)
}

/// 校验官方插件清单身份、入口、能力和持久化版本。
fn validate_manifest(manifest: &ComponentManifest) -> Result<(), ComponentRuntimeError>
{
    if manifest.id.is_empty() || manifest.id.len() > 128 || !manifest.id.chars().all(|character| character.is_ascii_lowercase() || character.is_ascii_digit() || matches!(character, '.' | '-'))
    {
        return Err(ComponentRuntimeError::invalid_manifest("插件清单身份无效"));
    }
    if manifest.name.trim().is_empty() || manifest.name.chars().count() > 80
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
    if plugin_slot && manifest.required_capabilities.is_empty()
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
