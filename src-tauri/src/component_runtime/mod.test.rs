use super::{ComponentRuntimeState, validate_manifest};
use crate::contracts::{ComponentCapability, ComponentCapabilityDeclaration, ComponentInstanceSnapshot, ComponentLoadStatus, ComponentManifest, WorkspaceRect};
use serde_json::json;

/// Debug 清单与默认实例严格跟随 Rust 构建模式。
#[test]
fn debug_widget_is_registered_only_in_debug_builds()
{
    let runtime = ComponentRuntimeState::new(); // 当前构建注册表。
    let instances = runtime.default_instances(); // 当前构建默认实例。
    assert_eq!(instances.iter().any(|instance| instance.component_id == "official.debug-test"), cfg!(debug_assertions));
    assert_eq!(runtime.manifests.contains_key("official.debug-test"), cfg!(debug_assertions));
}

/// 天气格子由用户显式创建，默认集合不再注入天气实例。
#[test]
fn default_instances_do_not_contain_a_weather_tile()
{
    let runtime = ComponentRuntimeState::new(); // 当前构建注册表。
    let instances = runtime.default_instances(); // 当前构建默认实例。

    assert!(runtime.manifests.contains_key("official.weather"), "天气清单本身仍须注册，否则用户无法创建天气格子");
    assert!(!instances.iter().any(|instance| instance.component_id == "official.weather"));
}

/// 独立身份保护新订阅，宿主清理不影响其他屏幕。
#[test]
fn time_subscriptions_are_scoped_and_do_not_reuse_identity()
{
    let first = ComponentRuntimeState::subscribe_time("test-clock-left".to_string()); // 左屏旧页面。
    let other = ComponentRuntimeState::subscribe_time("test-clock-right".to_string()); // 右屏保持活动。
    ComponentRuntimeState::clear_time_monitor("test-clock-left");
    assert!(!ComponentRuntimeState::has_time_consumers("test-clock-left"));
    let next = ComponentRuntimeState::subscribe_time("test-clock-left".to_string()); // 左屏重载后的新页面。
    assert_ne!(first, next);
    ComponentRuntimeState::unsubscribe_time(first);
    assert!(ComponentRuntimeState::has_time_consumers("test-clock-left"));
    assert!(ComponentRuntimeState::has_time_consumers("test-clock-right"));
    ComponentRuntimeState::unsubscribe_time(next);
    ComponentRuntimeState::unsubscribe_time(other);
    assert!(!ComponentRuntimeState::has_time_consumers("test-clock-left"));
    assert!(!ComponentRuntimeState::has_time_consumers("test-clock-right"));
}

/// 验证未授权能力在清单注册边界被拒绝。
#[test]
fn manifest_rejects_an_unauthorized_capability_set()
{
    let manifest = ComponentManifest { id: "official.local-clock".to_string(), name: "本地时间".to_string(), entry: "official:local-clock".to_string(), required_capabilities: Vec::new(), creatable: true, removable: true, persistence_version: 1 }; // 缺少宿主时间能力的无效官方清单。

    assert_eq!(validate_manifest(&manifest).expect_err("空能力集合应被拒绝").code, "invalid-component-manifest");
}

/// 验证缺失新字段的清单解析为不可创建、不可移除，显式声明按清单取值。
#[test]
fn declared_creation_and_removal_default_to_false()
{
    let absent = serde_json::from_str::<ComponentManifest>(r#"{"id":"official.local-clock","name":"本地时间","entry":"official:local-clock","requiredCapabilities":["read-host-time"],"persistenceVersion":1}"#).expect("缺失能力字段的清单应可解析");
    let declared = serde_json::from_str::<ComponentManifest>(r#"{"id":"official.collect-box","name":"收集箱","entry":"official:collect-box","requiredCapabilities":[],"creatable":true,"removable":false,"persistenceVersion":1}"#).expect("声明能力字段的清单应可解析");

    assert!(!absent.creatable && !absent.removable);
    assert!(declared.creatable && !declared.removable);
    assert_eq!(validate_manifest(&absent), Ok(()));
}

/// 验证注册表只发布成功注册清单声明的创建与移除能力。
#[test]
fn registered_manifests_publish_creation_and_removal_declarations()
{
    let runtime = ComponentRuntimeState::new(); // 当前构建注册表。
    let declaration = |component_id: &str| runtime.capability_declarations().iter().find(|declaration| declaration.component_id == component_id).cloned(); // 当前构建的单项能力声明。

    assert_eq!(declaration("official.collect-box"), Some(ComponentCapabilityDeclaration { component_id: "official.collect-box".to_string(), creatable: true, removable: true }));
    assert_eq!(declaration("official.weather"), Some(ComponentCapabilityDeclaration { component_id: "official.weather".to_string(), creatable: true, removable: true }));
    assert_eq!(declaration("official.local-clock"), Some(ComponentCapabilityDeclaration { component_id: "official.local-clock".to_string(), creatable: false, removable: false }));
    assert_eq!(runtime.capability_declarations().len(), runtime.snapshot(&[]).unwrap().manifests.len());
}

/// 验证未知实例形成失败状态且不影响有效官方实例。
#[test]
fn failed_instance_is_isolated_from_ready_instances()
{
    let runtime = ComponentRuntimeState::new(); // 已注册本地时间清单的组件运行时。
    let instance = |instance_id: &str, component_id: &str| ComponentInstanceSnapshot { instance_id: instance_id.to_string(), component_id: component_id.to_string(), monitor_id: "DISPLAY1".to_string(), projection_monitor_id: None, projection_bounds: None, bounds: WorkspaceRect { left: 20.0, top: 20.0, width: 220.0, height: 116.0 }, state_version: 1, state: json!({}) }; // 当前测试复用的组件实例构造器。

    let snapshot = runtime.snapshot(&[instance("clock", "official.local-clock"), instance("missing", "official.missing")]).expect("实例集合应可隔离注册");

    assert_eq!(snapshot.instances[0].status, ComponentLoadStatus::Ready);
    assert_eq!(snapshot.instances[1].status, ComponentLoadStatus::Failed);
    assert_eq!(snapshot.instances[0].authorized_capabilities, [ComponentCapability::ReadHostTime]);
}
