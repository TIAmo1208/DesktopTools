use super::{ComponentRuntimeState, validate_manifest};
use crate::contracts::{ComponentCapability, ComponentCapabilityDeclaration, ComponentInstanceSnapshot, ComponentLoadStatus, ComponentManifest, WorkspaceRect};
use serde_json::json;

/// 插件槽只接受二选一能力，且不能冒用官方入口。
#[test]
fn plugin_slot_validates_identity_entry_and_single_capability()
{
    let mut manifest: ComponentManifest = serde_json::from_str(super::LOCAL_CLOCK_MANIFEST).unwrap(); // 从有效官方清单构造边界输入。
    manifest.id = "plugin.test".to_string();
    manifest.entry = super::PLUGIN_SLOT_ENTRY.to_string();
    for capability in [ComponentCapability::ReadHostTime, ComponentCapability::ReadCurrentMedia]
    {
        manifest.required_capabilities = vec![capability];
        assert_eq!(validate_manifest(&manifest), Ok(()));
    }
    manifest.required_capabilities = vec![ComponentCapability::ReadHostTime, ComponentCapability::ReadCurrentMedia];
    assert!(validate_manifest(&manifest).is_err());
    manifest.required_capabilities = vec![ComponentCapability::ReadHostTime];
    for (id, entry) in [("official.weather", "official:local-clock"), ("plugin.test", "official:local-clock"), ("official.local-clock", "plugin:slot"), ("plugin.test", "https://example.com/plugin.js")]
    {
        manifest.id = id.to_string();
        manifest.entry = entry.to_string();
        assert_eq!(validate_manifest(&manifest).unwrap_err().code, "invalid-component-manifest");
    }
}

/// 实际目录来源有界读取、坏清单隔离、缺失目录为空、官方身份优先。
#[test]
fn plugin_directory_sources_are_bounded_and_isolated()
{
    let directory = std::env::temp_dir().join(format!("desktop-tools-plugin-manifests-{}", std::process::id())); // 本测试独占的临时来源。
    assert!(ComponentRuntimeState::new(&directory).catalog().failures.is_empty());
    std::fs::create_dir_all(&directory).unwrap();
    let valid = directory.join("测试"); // 文件夹名必须等于 ini 的 name。
    std::fs::create_dir_all(valid.join("dist")).unwrap();
    std::fs::write(valid.join("dist/main.js"), "var ok = 1;").unwrap();
    std::fs::write(valid.join("manifest.ini"), "[plugin]\nid = plugin.test\nname = 测试\nversion = 1.2.3\nentry = dist/main.js\n").unwrap();
    std::fs::write(directory.join("plugin.test.json"), "{\"id\":\"plugin.flat\"}").unwrap(); // 根上的旧清单不再识别。
    let broken = directory.join("坏清单");
    std::fs::create_dir_all(&broken).unwrap();
    std::fs::write(broken.join("manifest.ini"), "not-ini").unwrap();
    let large = directory.join("过大");
    std::fs::create_dir_all(&large).unwrap();
    std::fs::write(large.join("manifest.ini"), " ".repeat(65537)).unwrap();
    let duplicate = directory.join("重复");
    std::fs::create_dir_all(&duplicate).unwrap();
    std::fs::write(duplicate.join("main.js"), "var clock = 1;").unwrap();
    std::fs::write(duplicate.join("manifest.ini"), "[plugin]\nid = plugin.test\nname = 重复\nversion = 1.0.0\nentry = main.js\n").unwrap();
    let runtime = ComponentRuntimeState::new(&directory); // 先注册的合法插件保留，后来的同身份只进失败。
    assert_eq!(runtime.catalog().manifests["plugin.test"].script, "测试/dist/main.js");
    assert_eq!(runtime.catalog().manifests["plugin.test"].entry, super::PLUGIN_SLOT_ENTRY);
    assert!(!runtime.catalog().manifests.contains_key("plugin.flat"));
    assert_eq!(runtime.catalog().manifests["official.local-clock"].name, serde_json::from_str::<ComponentManifest>(super::LOCAL_CLOCK_MANIFEST).unwrap().name);
    assert_eq!(runtime.catalog().failures.iter().map(|failure| failure.component_id.as_str()).collect::<Vec<_>>(), ["plugin.test", "坏清单", "过大"]);
    assert!(runtime.catalog().failures.iter().any(|failure| failure.message.contains("64 KiB")));
    std::fs::remove_dir_all(&directory).unwrap();
}

/// 名称、版本、入口和程序版本区间不通过时只进入 failures。
#[test]
fn plugin_folder_manifest_rejects_invalid_package_fields()
{
    let directory = std::env::temp_dir().join(format!("desktop-tools-plugin-ini-{}", std::process::id())); // 本测试独占的临时来源。
    let write = |folder: &str, body: &str, script: bool| {
        let path = directory.join(folder); // 一个待校验的插件文件夹。
        std::fs::create_dir_all(&path).unwrap();
        if script
        {
            std::fs::write(path.join("main.js"), "var x = 1;").unwrap();
        }
        std::fs::write(path.join("manifest.ini"), body).unwrap();
    };
    write("时钟", "[plugin]\nid = clock\nname = 时钟\nversion = 1.0.0\nentry = main.js\n", true);
    write("坏名", "[plugin]\nid = plugin.bad\nname = 坏名\nversion = 1.0.0\nentry = ../main.js\n", true);
    write("坏版本", "[plugin]\nid = plugin.badver\nname = 坏版本\nversion = 1.0\nentry = main.js\n", true);
    write("过高", "[plugin]\nid = plugin.high\nname = 过高\nversion = 1.0.0\nentry = main.js\nminAppVersion = 99.0.0\n", true);
    let runtime = ComponentRuntimeState::new(&directory);
    assert!(runtime.catalog().manifests.keys().all(|id| !id.starts_with("plugin.") && id != "clock"));
    assert_eq!(runtime.catalog().failures.len(), 4);
    std::fs::remove_dir_all(&directory).unwrap();
}

/// Debug 清单与默认实例严格跟随 Rust 构建模式。
#[test]
fn debug_widget_is_registered_only_in_debug_builds()
{
    let runtime = ComponentRuntimeState::new(&std::env::temp_dir().join(format!("desktop-tools-no-plugins-{}", std::process::id()))); // 当前构建注册表。
    let instances = runtime.default_instances(); // 当前构建默认实例。
    assert_eq!(instances.iter().any(|instance| instance.component_id == "official.debug-test"), cfg!(debug_assertions));
    assert_eq!(runtime.catalog().manifests.contains_key("official.debug-test"), cfg!(debug_assertions));
}

/// 天气格子由用户显式创建，默认集合不再注入天气实例。
#[test]
fn default_instances_do_not_contain_a_weather_tile()
{
    let runtime = ComponentRuntimeState::new(&std::env::temp_dir().join(format!("desktop-tools-no-plugins-{}", std::process::id()))); // 当前构建注册表。
    let instances = runtime.default_instances(); // 当前构建默认实例。

    assert!(runtime.catalog().manifests.contains_key("official.weather"), "天气清单本身仍须注册，否则用户无法创建天气格子");
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
    let manifest = ComponentManifest { id: "official.local-clock".to_string(), name: "本地时间".to_string(), entry: "official:local-clock".to_string(), script: String::new(), required_capabilities: Vec::new(), creatable: true, removable: true, persistence_version: 1, max_tiles: 0, repository: String::new(), description: String::new(), author: String::new() }; // 缺少宿主时间能力的无效官方清单。

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
    let runtime = ComponentRuntimeState::new(&std::env::temp_dir().join(format!("desktop-tools-no-plugins-{}", std::process::id()))); // 当前构建注册表。
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
    let runtime = ComponentRuntimeState::new(&std::env::temp_dir().join(format!("desktop-tools-no-plugins-{}", std::process::id()))); // 已注册本地时间清单的组件运行时。
    let instance = |instance_id: &str, component_id: &str| ComponentInstanceSnapshot { instance_id: instance_id.to_string(), component_id: component_id.to_string(), monitor_id: "DISPLAY1".to_string(), projection_monitor_id: None, projection_bounds: None, bounds: WorkspaceRect { left: 20.0, top: 20.0, width: 220.0, height: 116.0 }, state_version: 1, state: json!({}) }; // 当前测试复用的组件实例构造器。

    let snapshot = runtime.snapshot(&[instance("clock", "official.local-clock"), instance("missing", "official.missing")]).expect("实例集合应可隔离注册");

    assert_eq!(snapshot.instances[0].status, ComponentLoadStatus::Ready);
    assert_eq!(snapshot.instances[1].status, ComponentLoadStatus::Failed);
    assert_eq!(snapshot.instances[0].authorized_capabilities, [ComponentCapability::ReadHostTime]);
}

/// 安装或删除文件夹后重扫清单。已有实例还在，坏清单只进 failures。
#[test]
fn reload_plugins_rescans_without_dropping_instances()
{
    let directory = std::env::temp_dir().join(format!("desktop-tools-plugin-reload-{}", std::process::id())); // 本测试独占目录。
    std::fs::create_dir_all(directory.join("时钟")).unwrap();
    std::fs::write(directory.join("时钟/main.js"), "var ok = 1;").unwrap();
    std::fs::write(directory.join("时钟/manifest.ini"), "[plugin]\nid = plugin.clock\nname = 时钟\nversion = 1.0.0\nentry = main.js\n").unwrap();
    let runtime = ComponentRuntimeState::new(&directory);
    let clock = runtime.default_instances().into_iter().find(|instance| instance.component_id == "official.local-clock").unwrap(); // 重扫前后都要留着的官方实例。
    assert!(runtime.snapshot(std::slice::from_ref(&clock)).unwrap().instances.iter().any(|instance| instance.instance_id == clock.instance_id));
    std::fs::create_dir_all(directory.join("坏清单")).unwrap();
    std::fs::write(directory.join("坏清单/manifest.ini"), "not-ini").unwrap();
    runtime.reload_plugins();
    assert!(runtime.catalog().manifests.contains_key("plugin.clock"));
    assert!(runtime.catalog().manifests.contains_key("official.local-clock"));
    assert!(runtime.catalog().failures.iter().any(|failure| failure.component_id == "坏清单"));
    let after = runtime.snapshot(std::slice::from_ref(&clock)).unwrap();
    assert!(after.instances.iter().any(|instance| instance.instance_id == clock.instance_id && instance.status == ComponentLoadStatus::Ready));
    std::fs::remove_dir_all(directory.join("时钟")).unwrap();
    runtime.reload_plugins();
    assert!(!runtime.catalog().manifests.contains_key("plugin.clock"));
    assert!(runtime.catalog().manifests.contains_key("official.local-clock"));
    std::fs::remove_dir_all(&directory).unwrap();
}

/// maxTiles 缺省或非法都不限；达到上限时拒绝再创建。
#[test]
fn plugin_max_tiles_limits_creation_and_ignores_invalid_values()
{
    let directory = std::env::temp_dir().join(format!("desktop-tools-plugin-max-tiles-{}", std::process::id())); // 本测试独占目录。
    let write = |folder: &str, id: &str, max_tiles: &str| {
        let path = directory.join(folder); // 一个待注册的插件文件夹。
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("main.js"), "var ok = 1;").unwrap();
        std::fs::write(path.join("manifest.ini"), format!("[plugin]\nid = {id}\nname = {folder}\nversion = 1.0.0\nentry = main.js\nmaxTiles = {max_tiles}\n")).unwrap();
    };
    write("限三", "plugin.three", "3");
    write("负数", "plugin.neg", "-1");
    write("坏值", "plugin.bad", "abc");
    let runtime = ComponentRuntimeState::new(&directory);
    assert_eq!(runtime.catalog().manifests["plugin.three"].max_tiles, 3);
    assert_eq!(runtime.catalog().manifests["plugin.neg"].max_tiles, 0);
    assert_eq!(runtime.catalog().manifests["plugin.bad"].max_tiles, 0);
    assert!(runtime.catalog().failures.iter().all(|failure| failure.component_id != "plugin.neg" && failure.component_id != "plugin.bad"));
    assert!(super::enforce_plugin_tile_limit(3, 2).is_ok());
    assert_eq!(super::enforce_plugin_tile_limit(3, 3).unwrap_err(), "已达到该插件的格子上限（3 个）");
    assert!(super::enforce_plugin_tile_limit(0, 9).is_ok());
    std::fs::remove_dir_all(directory).unwrap();
}

/// creatable 缺省为假，写 1 或 true 才可创建；removable 缺省为真。非法值不拒绝注册。
#[test]
fn plugin_creation_flags_follow_the_ini_defaults()
{
    let directory = std::env::temp_dir().join(format!("desktop-tools-plugin-flags-{}", std::process::id())); // 本测试独占目录。
    let write = |folder: &str, id: &str, extra: &str| {
        let path = directory.join(folder); // 一个待注册的插件文件夹。
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("main.js"), "var ok = 1;").unwrap();
        std::fs::write(path.join("manifest.ini"), format!("[plugin]\nid = {id}\nname = {folder}\nversion = 1.0.0\nentry = main.js\n{extra}")).unwrap();
    };
    write("缺省", "plugin.plain", "");
    write("可加", "plugin.add", "creatable = 1\nremovable = 0\n");
    write("真值", "plugin.yes", "creatable = true\n");
    write("坏旗", "plugin.junk", "creatable = yes\nremovable =\n");
    let runtime = ComponentRuntimeState::new(&directory);
    let manifest = |id: &str| runtime.catalog().manifests.get(id).unwrap().clone();
    assert!(!manifest("plugin.plain").creatable && manifest("plugin.plain").removable);
    assert!(manifest("plugin.add").creatable && !manifest("plugin.add").removable);
    assert!(manifest("plugin.yes").creatable && manifest("plugin.yes").removable);
    assert!(!manifest("plugin.junk").creatable && manifest("plugin.junk").removable);
    assert!(runtime.catalog().failures.iter().all(|failure| failure.component_id != "plugin.junk"));
    std::fs::remove_dir_all(directory).unwrap();
}

/// 入口脚本可以先不存在。非法仓库地址丢掉，合法的留下。
#[test]
fn missing_entry_script_still_registers_and_repository_is_filtered()
{
    let directory = std::env::temp_dir().join(format!("desktop-tools-plugin-repository-{}", std::process::id())); // 本测试独占目录。
    let path = directory.join("缺脚本"); // 没有 main.js 的插件。
    std::fs::create_dir_all(&path).unwrap();
    std::fs::write(path.join("manifest.ini"), "[plugin]\nid = plugin.missing-script\nname = 缺脚本\nversion = 1.0.0\nentry = main.js\nrepository = ftp://example.com/plugin\n").unwrap();
    let linked = directory.join("有仓库"); // 带合法仓库地址。
    std::fs::create_dir_all(&linked).unwrap();
    std::fs::write(linked.join("main.js"), "var ok = 1;").unwrap();
    std::fs::write(linked.join("manifest.ini"), "[plugin]\nid = plugin.repo\nname = 有仓库\nversion = 1.0.0\nentry = main.js\nrepository = https://example.com/plugin\n").unwrap();
    let runtime = ComponentRuntimeState::new(&directory);
    assert!(runtime.catalog().manifests.get("plugin.missing-script").expect("缺入口脚本仍应进入清单").repository.is_empty());
    assert_eq!(runtime.catalog().manifests["plugin.repo"].repository, "https://example.com/plugin");
    assert!(super::declared_plugin_repository("").is_err());
    assert_eq!(super::declared_plugin_repository("https://example.com/plugin").unwrap(), "https://example.com/plugin");
    std::fs::remove_dir_all(directory).unwrap();
}

/// 简介缺省为空。ini 不跨行，\n 还原成换行。
#[test]
fn plugin_description_keeps_escaped_newlines()
{
    let directory = std::env::temp_dir().join(format!("desktop-tools-plugin-description-{}", std::process::id())); // 本测试独占目录。
    let plain = directory.join("无简介");
    let wrapped = directory.join("有简介");
    std::fs::create_dir_all(&plain).unwrap();
    std::fs::create_dir_all(&wrapped).unwrap();
    std::fs::write(plain.join("main.js"), "var ok = 1;").unwrap();
    std::fs::write(wrapped.join("main.js"), "var ok = 1;").unwrap();
    std::fs::write(plain.join("manifest.ini"), "[plugin]\nid = plugin.plain\nname = 无简介\nversion = 1.0.0\nentry = main.js\n").unwrap();
    std::fs::write(wrapped.join("manifest.ini"), "[plugin]\nid = plugin.about\nname = 有简介\nversion = 1.0.0\nentry = main.js\ndescription = 第一行\\n第二行\nauthor = 张三\n").unwrap();
    let runtime = ComponentRuntimeState::new(&directory);
    assert!(runtime.catalog().manifests["plugin.plain"].description.is_empty());
    assert!(runtime.catalog().manifests["plugin.plain"].author.is_empty());
    assert_eq!(runtime.catalog().manifests["plugin.about"].description, "第一行\n第二行");
    assert_eq!(runtime.catalog().manifests["plugin.about"].author, "张三");
    std::fs::remove_dir_all(directory).unwrap();
}
