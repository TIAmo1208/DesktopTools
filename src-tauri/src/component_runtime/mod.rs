use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::Duration;

use chrono::Utc;
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager};

use crate::contracts::{
    COMPONENT_HOST_EVENT, ComponentCapability, ComponentCapabilityResult, ComponentHostEvent,
    ComponentInstanceSnapshot, ComponentLoadStatus, ComponentManifest,
    ComponentRegistrationFailure, ComponentRegistrationSnapshot, ComponentRuntimeError,
    ComponentRuntimeSnapshot, WorkspaceRect,
};

/// 官方组件前端注册表当前支持的入口。
const LOCAL_CLOCK_ENTRY: &str = "official:local-clock";

/// 内置官方组件清单来源。
const LOCAL_CLOCK_MANIFEST: &str = include_str!("../../components/official.local-clock.json");

/// 持有官方组件清单、活动实例注册结果和能力边界。
pub struct ComponentRuntimeState {
    /// 已通过清单和能力校验的官方组件注册表。
    manifests: HashMap<String, ComponentManifest>,
    /// 单个清单失败但不阻塞其他组件的结果。
    failures: Vec<ComponentRegistrationFailure>,
    /// 当前活动虚拟桌面的实例注册结果。
    instances: Mutex<Vec<ComponentRegistrationSnapshot>>,
    /// 注册表或实例集合变化时递增的修订号。
    revision: AtomicU64,
    /// 下一个宿主事件序号。
    next_event_sequence: AtomicU64,
    /// 独立 Windowed WebView2 transport 可读取的最近宿主事件。
    latest_host_event: Mutex<Option<ComponentHostEvent>>,
}

/// 提供官方组件注册、实例同步、能力调用和宿主事件。
impl ComponentRuntimeState {
    /// 加载全部官方组件清单，单个失败只形成隔离错误。
    pub fn new() -> Self {
        let mut sources = vec![("official.local-clock", LOCAL_CLOCK_MANIFEST)]; // Release 只注册正式功能。
        if cfg!(debug_assertions) {
            sources.push((
                "official.debug-test",
                include_str!("../../components/official.debug-test.json"),
            ));
        }
        Self::from_manifest_sources(&sources)
    }

    /// 返回成功注册清单对应的默认组件实例。
    pub fn default_instances(&self) -> Vec<ComponentInstanceSnapshot> {
        if !self.manifests.contains_key("official.local-clock") {
            return Vec::new();
        }
        let mut instances = vec![ComponentInstanceSnapshot {
            instance_id: "official.local-clock.default".to_string(),
            component_id: "official.local-clock".to_string(),
            monitor_id: String::new(),
            projection_monitor_id: None,
            projection_bounds: None,
            bounds: WorkspaceRect {
                left: 0.0,
                top: 0.0,
                width: 220.0,
                height: 116.0,
            },
            state_version: 1,
            state: json!({"format": "24-hour"}),
        }]; // 保持既有时钟实例身份和持久状态。
        if self.manifests.contains_key("official.debug-test") {
            instances.push(ComponentInstanceSnapshot {
                instance_id: "official.debug-test.default".to_string(),
                component_id: "official.debug-test".to_string(),
                monitor_id: String::new(),
                projection_monitor_id: None,
                projection_bounds: None,
                bounds: WorkspaceRect {
                    left: 0.0,
                    top: 16.0,
                    width: 420.0,
                    height: 270.0,
                },
                state_version: 1,
                state: json!({}),
            });
        }
        instances
    }

    /// 校验并注册当前活动虚拟桌面的全部组件实例。
    pub fn snapshot(
        &self,
        instances: &[ComponentInstanceSnapshot],
    ) -> Result<ComponentRuntimeSnapshot, ComponentRuntimeError> {
        let registrations = instances
            .iter()
            .filter(|instance| {
                cfg!(debug_assertions) || instance.component_id != "official.debug-test"
            })
            .map(|instance| self.registration_snapshot(instance))
            .collect::<Vec<_>>(); // 当前布局中每个实例互相隔离的注册结果。
        let mut current = self
            .instances
            .lock()
            .map_err(|_| ComponentRuntimeError::state_unavailable())?; // 等待发布的实例注册集合。
        if *current != registrations {
            *current = registrations;
            self.revision.fetch_add(1, Ordering::Relaxed);
        }
        let mut manifests = self.manifests.values().cloned().collect::<Vec<_>>(); // 稳定排序后发布的官方清单集合。
        manifests.sort_by(|first, second| first.id.cmp(&second.id));
        Ok(ComponentRuntimeSnapshot {
            revision: self.revision.load(Ordering::Relaxed),
            manifests,
            instances: current.clone(),
            failures: self.failures.clone(),
        })
    }

    /// 校验实例声明后执行一个已授权宿主能力。
    pub fn invoke_capability(
        &self,
        instances: &[ComponentInstanceSnapshot],
        instance_id: &str,
        capability: ComponentCapability,
    ) -> Result<ComponentCapabilityResult, ComponentRuntimeError> {
        let snapshot = self.snapshot(instances)?; // 能力调用前同步的当前实例注册结果。
        let registration = snapshot
            .instances
            .iter()
            .find(|instance| instance.instance_id == instance_id)
            .ok_or_else(|| ComponentRuntimeError::invalid_instance("组件实例不存在"))?;
        if registration.status != ComponentLoadStatus::Ready
            || !registration.authorized_capabilities.contains(&capability)
        {
            return Err(ComponentRuntimeError::capability_denied(capability));
        }
        let payload = match capability {
            ComponentCapability::ReadHostTime => json!({
                "unixMillis": Utc::now().timestamp_millis()
            }),
        }; // 当前授权能力的最小结构化结果。
        crate::log_debug!(
            "组件：宿主能力调用完成，实例={}，能力={capability:?}",
            instance_id
        );
        Ok(ComponentCapabilityResult {
            instance_id: instance_id.to_string(),
            capability,
            payload,
        })
    }

    /// 返回最近一次已发布的组件宿主事件。
    pub fn current_host_event(&self) -> Result<Option<ComponentHostEvent>, ComponentRuntimeError> {
        self.latest_host_event
            .lock()
            .map(|event| event.clone())
            .map_err(|_| ComponentRuntimeError::state_unavailable())
    }

    /// 启动官方组件共享的低频宿主时间事件。
    pub fn start_host_events(app: AppHandle) {
        crate::log_info!("组件：宿主事件发布器已启动，间隔=1秒");
        thread::spawn(move || {
            loop {
                thread::sleep(Duration::from_secs(1));
                let Some(state) = app.try_state::<ComponentRuntimeState>() else {
                    break;
                };
                if !state.manifests.values().any(|manifest| {
                    manifest
                        .required_capabilities
                        .contains(&ComponentCapability::ReadHostTime)
                }) {
                    continue;
                }
                let event = ComponentHostEvent {
                    sequence: state.next_event_sequence.fetch_add(1, Ordering::Relaxed),
                    kind: "time-tick".to_string(),
                    unix_millis: Utc::now().timestamp_millis(),
                }; // 已注册时间能力组件共享的宿主事件。
                let Ok(mut latest_event) = state.latest_host_event.lock() else {
                    crate::log_error!("组件：宿主事件状态不可用，事件发布器停止");
                    break;
                }; // 供独立 Windowed transport 轮询的最近事件。
                *latest_event = Some(event.clone());
                drop(latest_event);
                if app.emit(COMPONENT_HOST_EVENT, event).is_err() {
                    break;
                }
            }
        });
    }

    /// 从一组独立清单来源建立容错官方注册表。
    fn from_manifest_sources(sources: &[(&str, &str)]) -> Self {
        let mut manifests = HashMap::new(); // 已通过校验的官方组件清单。
        let mut failures = Vec::new(); // 不阻塞其他组件的清单失败集合。
        for (fallback_id, source) in sources {
            let manifest = serde_json::from_str::<ComponentManifest>(source)
                .map_err(|error| {
                    ComponentRuntimeError::invalid_manifest(format!("组件清单 JSON 无效: {error}"))
                })
                .and_then(|manifest| {
                    validate_manifest(&manifest)?;
                    Ok(manifest)
                });
            match manifest {
                Ok(manifest) if manifests.contains_key(&manifest.id) => {
                    failures.push(ComponentRegistrationFailure {
                        component_id: manifest.id,
                        message: "组件清单身份重复".to_string(),
                    })
                }
                Ok(manifest) => {
                    crate::log_info!(
                        "组件：官方清单已注册，身份={}，入口={}，能力={}",
                        manifest.id,
                        manifest.entry,
                        manifest.required_capabilities.len()
                    );
                    manifests.insert(manifest.id.clone(), manifest);
                }
                Err(error) => {
                    crate::log_error!(
                        "组件：官方清单加载失败，身份={}，错误={}",
                        fallback_id,
                        error.message
                    );
                    failures.push(ComponentRegistrationFailure {
                        component_id: (*fallback_id).to_string(),
                        message: error.message,
                    });
                }
            }
        }
        Self {
            manifests,
            failures,
            instances: Mutex::new(Vec::new()),
            revision: AtomicU64::new(1),
            next_event_sequence: AtomicU64::new(1),
            latest_host_event: Mutex::new(None),
        }
    }

    /// 将一个持久实例转换为成功或失败的隔离注册结果。
    fn registration_snapshot(
        &self,
        instance: &ComponentInstanceSnapshot,
    ) -> ComponentRegistrationSnapshot {
        let Some(manifest) = self.manifests.get(&instance.component_id) else {
            return failed_registration(instance, "组件清单不存在");
        };
        if instance.state_version != manifest.persistence_version {
            return failed_registration(instance, "组件持久状态版本不兼容");
        }
        ComponentRegistrationSnapshot {
            instance_id: instance.instance_id.clone(),
            component_id: instance.component_id.clone(),
            entry: Some(manifest.entry.clone()),
            authorized_capabilities: manifest.required_capabilities.clone(),
            status: ComponentLoadStatus::Ready,
            error: None,
        }
    }
}

/// 校验官方组件清单身份、入口、能力和持久化版本。
fn validate_manifest(manifest: &ComponentManifest) -> Result<(), ComponentRuntimeError> {
    if manifest.id.is_empty()
        || manifest.id.len() > 128
        || !manifest.id.chars().all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '.' | '-')
        })
    {
        return Err(ComponentRuntimeError::invalid_manifest("组件清单身份无效"));
    }
    if manifest.name.trim().is_empty() || manifest.name.chars().count() > 64 {
        return Err(ComponentRuntimeError::invalid_manifest("组件清单名称无效"));
    }
    let debug_manifest = cfg!(debug_assertions)
        && manifest.id == "official.debug-test"
        && manifest.entry == "official:debug-test"; // 仅 Debug 可注册测试入口。
    if manifest.entry != LOCAL_CLOCK_ENTRY && !debug_manifest {
        return Err(ComponentRuntimeError::invalid_manifest(
            "组件前端入口不受支持",
        ));
    }
    if manifest.persistence_version == 0 {
        return Err(ComponentRuntimeError::invalid_manifest(
            "组件持久化版本无效",
        ));
    }
    if debug_manifest && manifest.required_capabilities.is_empty() {
        return Ok(());
    }
    let capabilities = manifest
        .required_capabilities
        .iter()
        .copied()
        .collect::<HashSet<_>>(); // 清单中唯一的能力声明集合。
    if capabilities.len() != manifest.required_capabilities.len() || capabilities.is_empty() {
        return Err(ComponentRuntimeError::invalid_manifest("组件能力声明无效"));
    }
    if capabilities != HashSet::from([ComponentCapability::ReadHostTime]) {
        return Err(ComponentRuntimeError::invalid_manifest(
            "组件请求了未授权宿主能力",
        ));
    }
    Ok(())
}

/// 创建不影响其他实例的组件加载失败快照。
fn failed_registration(
    instance: &ComponentInstanceSnapshot,
    message: &str,
) -> ComponentRegistrationSnapshot {
    ComponentRegistrationSnapshot {
        instance_id: instance.instance_id.clone(),
        component_id: instance.component_id.clone(),
        entry: None,
        authorized_capabilities: Vec::new(),
        status: ComponentLoadStatus::Failed,
        error: Some(message.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::{ComponentRuntimeState, validate_manifest};
    use crate::contracts::{
        ComponentCapability, ComponentInstanceSnapshot, ComponentLoadStatus, ComponentManifest,
        WorkspaceRect,
    };
    use serde_json::json;

    /// Debug 清单与默认实例严格跟随 Rust 构建模式。
    #[test]
    fn debug_widget_is_registered_only_in_debug_builds() {
        let runtime = ComponentRuntimeState::new(); // 当前构建注册表。
        let instances = runtime.default_instances(); // 当前构建默认实例。
        assert_eq!(
            instances
                .iter()
                .any(|instance| instance.component_id == "official.debug-test"),
            cfg!(debug_assertions)
        );
        assert_eq!(
            runtime.manifests.contains_key("official.debug-test"),
            cfg!(debug_assertions)
        );
    }

    /// 验证未授权能力在清单注册边界被拒绝。
    #[test]
    fn manifest_rejects_an_unauthorized_capability_set() {
        let manifest = ComponentManifest {
            id: "official.local-clock".to_string(),
            name: "本地时间".to_string(),
            entry: "official:local-clock".to_string(),
            required_capabilities: Vec::new(),
            persistence_version: 1,
        }; // 缺少宿主时间能力的无效官方清单。

        assert_eq!(
            validate_manifest(&manifest)
                .expect_err("空能力集合应被拒绝")
                .code,
            "invalid-component-manifest"
        );
    }

    /// 验证未知实例形成失败状态且不影响有效官方实例。
    #[test]
    fn failed_instance_is_isolated_from_ready_instances() {
        let runtime = ComponentRuntimeState::new(); // 已注册本地时间清单的组件运行时。
        let instance = |instance_id: &str, component_id: &str| ComponentInstanceSnapshot {
            instance_id: instance_id.to_string(),
            component_id: component_id.to_string(),
            monitor_id: "DISPLAY1".to_string(),
            projection_monitor_id: None,
            projection_bounds: None,
            bounds: WorkspaceRect {
                left: 20.0,
                top: 20.0,
                width: 220.0,
                height: 116.0,
            },
            state_version: 1,
            state: json!({}),
        }; // 当前测试复用的组件实例构造器。

        let snapshot = runtime
            .snapshot(&[
                instance("clock", "official.local-clock"),
                instance("missing", "official.missing"),
            ])
            .expect("实例集合应可隔离注册");

        assert_eq!(snapshot.instances[0].status, ComponentLoadStatus::Ready);
        assert_eq!(snapshot.instances[1].status, ComponentLoadStatus::Failed);
        assert_eq!(
            snapshot.instances[0].authorized_capabilities,
            [ComponentCapability::ReadHostTime]
        );
    }
}
