use super::{ComponentRuntimeState, validate_manifest};
use crate::contracts::{ComponentCapability, ComponentCapabilityDeclaration, ComponentInstanceSnapshot, ComponentLoadStatus, ComponentManifest, WorkspaceRect};
use serde_json::json;

/// 主题清单与数据经过真实目录扫描，错误均进入隔离失败集合。
#[test]
fn theme_manifest_validates_assets_and_appearance()
{
    let directory = std::env::temp_dir().join(format!("desktop-tools-theme-{}", std::process::id())); // 本测试独占来源。
    let folder = directory.join("theme"); // 主题与普通格子复用同一清单。
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("main.js"), "").unwrap();
    std::fs::write(folder.join("theme.css"), ":root {}").unwrap();
    let base = "[plugin]\nid=plugin.theme\nname=theme\nversion=1.0.0\nentry=main.js\ncreatable=true\nmaxTiles=5\n"; // 保留原有 entry 校验。
    let theme = format!("{base}role=theme\nstylesheet=theme.css\n"); // 合法主题声明。
    std::fs::write(folder.join("theme.json"), r##"{"appearance":{"accentColor":"#Ab12Cd","desktopAccentColor":"#123456","cornerRadius":12,"fontFamily":"Arial","fontSize":18},"notification":{"surface":"#0d1322","text":"#e8f1ff","success":"#00f5a0","weight":"semibold","cornerStyle":"bevel","cornerSize":12},"boot":{"durationMs":2600}}"##).unwrap();
    std::fs::write(folder.join("manifest.ini"), &theme).unwrap();
    let runtime = ComponentRuntimeState::new(&directory); // 检查运行时下发载荷。
    assert!(runtime.catalog().failures.is_empty());
    let manifest = runtime.catalog().manifests["plugin.theme"].clone();
    assert_eq!(manifest.role, crate::contracts::ComponentRole::Theme);
    assert_eq!(manifest.stylesheet, "theme/theme.css");
    assert_eq!(manifest.version, "1.0.0");
    assert!(manifest.stylesheet_cache_key.starts_with("1.0.0-8-"));
    assert_eq!(manifest.script, "theme/main.js"); // 主题和格子一样可以带入口脚本。
    assert!(manifest.creatable); // 带入口脚本的主题可以声明创建格子。
    assert_eq!(manifest.max_tiles, 5);
    assert_eq!(serde_json::to_value(manifest.theme_appearance).unwrap()["accentColor"], "#Ab12Cd");
    let notification = manifest.theme_notification.clone().expect("主题可以声明消息配色"); // 未声明的字段留给内置调色板。
    assert_eq!((notification.surface.as_deref(), notification.text.as_deref()), (Some("#0d1322"), Some("#e8f1ff")));
    assert_eq!(notification.border, None);
    assert_eq!(notification.success.as_deref(), Some("#00f5a0")); // 状态色可选，逐项覆盖内置语义色。
    assert_eq!(notification.weight.as_deref(), Some("semibold")); // 字重只接受四个枚举。
    assert_eq!(notification.corner_style.as_deref(), Some("bevel"));
    assert_eq!(notification.corner_size, Some(12));
    assert_eq!(manifest.theme_boot.as_ref().and_then(|boot| boot.duration_ms), Some(2600)); // 启动覆盖层时长归主题数据。
    std::fs::remove_file(folder.join("main.js")).unwrap();
    let scriptless = ComponentRuntimeState::new(&directory).catalog().manifests["plugin.theme"].clone();
    assert!(scriptless.script.is_empty()); // 入口脚本缺失时不下发，无脚本主题不产生加载失败。
    assert!(!scriptless.creatable); // 没有格子入口的主题不能创建，也不读格子上限。
    assert_eq!(scriptless.max_tiles, 0);
    let unchanged = scriptless.stylesheet_cache_key.clone();
    assert_eq!(ComponentRuntimeState::new(&directory).catalog().manifests["plugin.theme"].stylesheet_cache_key, unchanged); // 未改文件时缓存键稳定。
    std::fs::write(folder.join("theme.css"), ":root { color: red; }").unwrap();
    let changed = ComponentRuntimeState::new(&directory).catalog().manifests["plugin.theme"].stylesheet_cache_key.clone();
    assert_ne!(changed, unchanged); // 文件大小变化即使时间戳粒度不足也会让缓存键变化。
    std::fs::write(folder.join("theme.css"), ":root {}").unwrap();
    std::fs::write(folder.join("main.js"), "").unwrap();
    for body in [base.to_string(), format!("{base}role=tile\n")]
    {
        std::fs::write(folder.join("manifest.ini"), body).unwrap();
        let runtime = ComponentRuntimeState::new(&directory); // 缺省与显式 tile 保持既有结果。
        let manifest = runtime.catalog().manifests["plugin.theme"].clone();
        assert_eq!(manifest.role, crate::contracts::ComponentRole::Tile);
        assert_eq!(manifest.script, "theme/main.js");
        assert!(manifest.creatable);
        assert_eq!(manifest.max_tiles, 5);
        assert!(manifest.stylesheet.is_empty() && manifest.theme_appearance.is_none());
    }
    for suffix in ["role=unknown\n", "role=theme\n", "role=theme\nstylesheet=../theme.css\n", "role=theme\nstylesheet=/theme.css\n", "role=theme\nstylesheet=C:/theme.css\n", "role=theme\nstylesheet=sub\\theme.css\n", "role=theme\nstylesheet=main.js\n", "role=theme\nstylesheet=missing.css\n"]
    {
        std::fs::write(folder.join("manifest.ini"), format!("{base}{suffix}")).unwrap();
        assert_eq!(ComponentRuntimeState::new(&directory).catalog().failures.len(), 1, "{suffix}");
    }
    std::fs::write(folder.join("manifest.ini"), theme.replace("entry=main.js", "entry=../main.js")).unwrap();
    assert_eq!(ComponentRuntimeState::new(&directory).catalog().failures.len(), 1);
    std::fs::write(folder.join("manifest.ini"), &theme).unwrap();
    for body in [r##"{"appearance":{"accentColor":"#123456"},"notification":{"surface":"0d1322"}}"##, r##"{"appearance":{"accentColor":"#123456"},"notification":{"text":"#12345g"}}"##, r##"{"appearance":{"accentColor":"#123456"},"notification":{"unknown":"#123456"}}"##, r##"{"appearance":{"accentColor":"#123456"},"notification":{"weight":"heavy"}}"##, r##"{"appearance":{"accentColor":"#123456"},"notification":{"warning":"#12"}}"##, r##"{"appearance":{"accentColor":"#123456"},"notification":{"cornerSize":0}}"##, r##"{"appearance":{"accentColor":"#123456"},"notification":{"cornerSize":25}}"##, r##"{"appearance":{"accentColor":"#123456"},"notification":{"cornerStyle":"cut"}}"##, r##"{"appearance":{"accentColor":"#123456"},"notification":{"cornerEdge":"#fce300"}}"##, r##"{"appearance":{"accentColor":"#123456"},"boot":{"durationMs":20000}}"##]
    {
        std::fs::write(folder.join("theme.json"), body).unwrap();
        assert_eq!(ComponentRuntimeState::new(&directory).catalog().failures.len(), 1, "{body}"); // 消息配色非法或字段未知都拒绝注册。
    }
    std::fs::write(folder.join("theme.json"), r##"{"appearance":{"accentColor":"#Ab12Cd","desktopAccentColor":"#123456","cornerRadius":12,"fontFamily":"Arial","fontSize":18},"notification":{"surface":"#0d1322","text":"#e8f1ff","success":"#00f5a0","weight":"semibold","cornerStyle":"bevel","cornerSize":12},"boot":{"durationMs":2600}}"##).unwrap();
    for body in [r##"{"appearance":{"accentColor":"red"}}"##, r##"{"appearance":{"desktopAccentColor":"#12345g"}}"##, r#"{"appearance":{"fontFamily":"Unknown"}}"#, r#"{"appearance":{"fontSize":19}}"#, r#"{"appearance":{"fontSize":11}}"#, r#"{"appearance":{"fontSize":12.5}}"#, r#"{"appearance":{"cornerRadius":13}}"#, r#"{"appearance":{"cornerRadius":-1}}"#, "{}", "not-json"]
    {
        std::fs::write(folder.join("theme.json"), &body).unwrap();
        assert_eq!(ComponentRuntimeState::new(&directory).catalog().failures.len(), 1, "{body}");
    }
    std::fs::write(folder.join("theme.json"), " ".repeat(65537)).unwrap();
    assert_eq!(ComponentRuntimeState::new(&directory).catalog().failures.len(), 1);
    std::fs::remove_file(folder.join("theme.json")).unwrap();
    assert_eq!(ComponentRuntimeState::new(&directory).catalog().failures.len(), 1);
    std::fs::write(folder.join("theme.json"), r#"{"appearance":{}}"#).unwrap();
    let runtime = ComponentRuntimeState::new(&directory); // 缺省覆盖字段序列化为 null。
    assert_eq!(serde_json::to_value(&runtime.catalog().manifests["plugin.theme"].theme_appearance).unwrap(), json!({"accentColor":null,"desktopAccentColor":null,"cornerRadius":null,"fontFamily":null,"fontSize":null}));
    let linked = folder.join("linked"); // 无需管理员权限的目录联接，用于覆盖重解析点路径。
    let output = std::process::Command::new("cmd").args(["/C", "mklink", "/J"]).arg(&linked).arg(&folder).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    std::fs::write(folder.join("manifest.ini"), theme.replace("stylesheet=theme.css", "stylesheet=linked/theme.css")).unwrap();
    assert_eq!(ComponentRuntimeState::new(&directory).catalog().failures.len(), 1);
    std::fs::remove_dir(&linked).unwrap();
    std::fs::write(folder.join("manifest.ini"), &theme).unwrap();
    std::fs::remove_file(folder.join("theme.json")).unwrap();
    let output = std::process::Command::new("cmd").args(["/C", "mklink", "/J"]).arg(folder.join("theme.json")).arg(&folder).output().unwrap(); // 数据文件名同样拒绝重解析点。
    assert!(output.status.success());
    assert_eq!(ComponentRuntimeState::new(&directory).catalog().failures.len(), 1);
    std::fs::remove_dir(folder.join("theme.json")).unwrap();
    std::fs::remove_dir_all(&directory).unwrap();
}

/// 主题自带字体按清单校验并在快照里给出插件根相对路径。
#[test]
fn theme_fonts_are_validated_and_published()
{
    let directory = std::env::temp_dir().join(format!("desktop-tools-theme-fonts-{}", std::process::id())); // 本测试独占来源。
    let folder = directory.join("theme");
    std::fs::create_dir_all(folder.join("fonts")).unwrap();
    std::fs::write(folder.join("theme.css"), ":root {}").unwrap();
    std::fs::write(folder.join("fonts/songti.woff2"), vec![0u8; 128]).unwrap();
    std::fs::write(folder.join("manifest.ini"), "[plugin]\nid=plugin.theme\nname=theme\nversion=1.0.0\nentry=main.js\nrole=theme\nstylesheet=theme.css\n").unwrap();
    let theme = |fonts: &str, appearance: &str| format!("{{\"appearance\":{appearance},\"fonts\":{fonts}}}");
    let valid = r#"[{"family":"宋律宋体","file":"fonts/songti.woff2"}]"#; // 缺省字重与字形。
    let declared = r#"{"fontFamily":"宋律宋体"}"#;
    std::fs::write(folder.join("theme.json"), theme(valid, declared)).unwrap();
    let runtime = ComponentRuntimeState::new(&directory);
    assert!(runtime.catalog().failures.is_empty());
    let font = &runtime.catalog().manifests["plugin.theme"].theme_fonts[0];
    assert_eq!((font.family.as_str(), font.file.as_str(), font.weight, font.style.as_str()), ("宋律宋体", "theme/fonts/songti.woff2", 400, "normal"));
    for body in [
        theme(r#"[{"family":"宋律宋体","file":"fonts/songti.ttf"}]"#, declared),           // 文件不存在。
        theme(r#"[{"family":"宋律宋体","file":"../songti.woff2"}]"#, declared),            // 跳出插件文件夹。
        theme(r#"[{"family":"宋律宋体","file":"fonts/songti.png"}]"#, declared),           // 扩展名不支持。
        theme(r#"[{"family":"Arial; color:red","file":"fonts/songti.woff2"}]"#, declared), // 族名带注入字符。
        theme(r#"[{"family":"宋律宋体","file":"fonts/songti.woff2","weight":99}]"#, declared),
        theme(r#"[{"family":"宋律宋体","file":"fonts/songti.woff2","style":"oblique"}]"#, declared),
        theme(r#"[{"family":"宋律宋体","file":"fonts/songti.woff2"},{"family":"宋律宋体","file":"fonts/songti.woff2"}]"#, declared),                               // 族名重复。
        theme(&format!("[{}]", (0..5).map(|index| format!(r#"{{"family":"字体{index}","file":"fonts/songti.woff2"}}"#)).collect::<Vec<_>>().join(",")), declared), // 超过数量上限。
        theme(valid, r#"{"fontFamily":"没有声明的字体"}"#),                                                                                                        // 外观字体既不在系统名单也不在本主题。
    ]
    {
        std::fs::write(folder.join("theme.json"), &body).unwrap();
        assert_eq!(ComponentRuntimeState::new(&directory).catalog().failures.len(), 1, "{body}");
    }
    std::fs::write(folder.join("fonts/big.woff2"), vec![0u8; 16 * 1024 * 1024 + 1]).unwrap();
    std::fs::write(folder.join("theme.json"), theme(r#"[{"family":"宋律宋体","file":"fonts/big.woff2"}]"#, declared)).unwrap();
    assert_eq!(ComponentRuntimeState::new(&directory).catalog().failures.len(), 1); // 单文件超过 16 MiB。
    std::fs::remove_dir_all(&directory).unwrap();
}

/// 仓库里的预设插件都能经真实扫描注册：清单、样式表或主题数据写错会在这里失败。
#[test]
fn repo_preset_plugins_register()
{
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../plugin"); // 仓库预设插件根。
    let runtime = ComponentRuntimeState::new(&root);
    let catalog = runtime.catalog();
    assert!(catalog.failures.is_empty(), "{:?}", catalog.failures);
    let theme = catalog.manifests.get("plugin.theme-songlv").unwrap(); // 主题预设必须注册为主题角色。
    assert_eq!(theme.role, crate::contracts::ComponentRole::Theme);
    assert_eq!(theme.stylesheet, "主题-宋律/theme.css");
    assert!(catalog.manifests.contains_key("plugin.theme-qingjian"));
    assert!(catalog.manifests.contains_key("plugin.astrbot-status"));
    let night_city = catalog.manifests.get("plugin.theme-nightcity").unwrap(); // 换肤和自带格子放在同一个插件里。
    assert_eq!(night_city.role, crate::contracts::ComponentRole::Theme);
    assert_eq!(night_city.script, "主题-夜之城/index.js");
    assert!(night_city.creatable); // 带入口脚本的主题可以创建格子。
    assert_eq!(night_city.max_tiles, 2);
}

/// 点开头的目录不注册，作者@名称 的文件夹按同一份清单校验。
#[test]
fn plugin_scan_skips_dot_folders_and_accepts_author_prefix()
{
    let directory = std::env::temp_dir().join(format!("desktop-tools-plugin-folders-{}", std::process::id())); // 本测试独占目录。
    let _ = std::fs::remove_dir_all(&directory);
    let installed = directory.join("张三@时钟");
    std::fs::create_dir_all(&installed).unwrap();
    std::fs::write(installed.join("manifest.ini"), "[plugin]\nid = plugin.clock\nname = 时钟\nversion = 1.0.0\nentry = main.js\nauthor = 张三\n").unwrap();
    let hidden = directory.join(".install-1"); // 安装临时目录。
    std::fs::create_dir_all(&hidden).unwrap();
    std::fs::write(hidden.join("manifest.ini"), "[plugin]\nid = plugin.hidden\nname = hidden\nversion = 1.0.0\nentry = main.js\n").unwrap();
    let runtime = ComponentRuntimeState::new(&directory);
    let catalog = runtime.catalog();
    assert!(catalog.failures.is_empty(), "{:?}", catalog.failures);
    assert_eq!(catalog.manifests["plugin.clock"].script, "张三@时钟/main.js");
    assert!(!catalog.manifests.contains_key("plugin.hidden"), "点开头的目录不注册");
    std::fs::remove_dir_all(&directory).unwrap();
}

/// 两端字体白名单逐项相等，防止只修改一侧导致有效主题无法保存。
#[test]
fn theme_font_allowlist_matches_preferences_contract()
{
    let source = include_str!("../../../src/contracts/settings.ts"); // 前端偏好契约原文。
    let declaration = source.split("export const APPLICATION_FONTS = ").nth(1).unwrap().split(" as const;").next().unwrap(); // 只读取白名单数组。
    let fonts: Vec<String> = serde_json::from_str(declaration).unwrap();
    assert_eq!(fonts, super::APPLICATION_FONTS);
}

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
    let manifest = ComponentManifest { theme_notification: None, theme_boot: None, id: "official.local-clock".to_string(), name: "本地时间".to_string(), version: String::new(), stylesheet_cache_key: String::new(), entry: "official:local-clock".to_string(), script: String::new(), role: crate::contracts::ComponentRole::Tile, stylesheet: String::new(), theme_appearance: None, theme_fonts: Vec::new(), required_capabilities: Vec::new(), creatable: true, removable: true, persistence_version: 1, max_tiles: 0, repository: String::new(), description: String::new(), author: String::new() }; // 缺少宿主时间能力的无效官方清单。

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

    assert_eq!(declaration("official.collect-box"), Some(ComponentCapabilityDeclaration { component_id: "official.collect-box".to_string(), creatable: true, removable: true, max_tiles: 0 }));
    assert_eq!(declaration("official.weather"), Some(ComponentCapabilityDeclaration { component_id: "official.weather".to_string(), creatable: true, removable: true, max_tiles: 0 }));
    assert_eq!(declaration("official.local-clock"), Some(ComponentCapabilityDeclaration { component_id: "official.local-clock".to_string(), creatable: true, removable: true, max_tiles: 1 }));
    assert_eq!(declaration("official.music-control"), Some(ComponentCapabilityDeclaration { component_id: "official.music-control".to_string(), creatable: true, removable: true, max_tiles: 1 }));
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

/// ini 以逗号分隔声明能力，未知值与重复值只让该插件进入 failures。
#[test]
fn plugin_capabilities_are_parsed_from_the_ini()
{
    let directory = std::env::temp_dir().join(format!("desktop-tools-plugin-capabilities-{}", std::process::id())); // 本测试独占目录。
    let write = |folder: &str, id: &str, capabilities: &str| {
        let path = directory.join(folder); // 一个待注册的插件文件夹。
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("main.js"), "var ok = 1;").unwrap();
        std::fs::write(path.join("manifest.ini"), format!("[plugin]\nid = {id}\nname = {folder}\nversion = 1.0.0\nentry = main.js\n{capabilities}")).unwrap();
    };
    write("探活", "plugin.probe", "required_capabilities = probe-service-status\n");
    write("空白能力", "plugin.spaced", "required_capabilities =  read-current-media , \n");
    write("未知能力", "plugin.unknown", "required_capabilities = probe-service-status, readRegistry\n");
    write("重复能力", "plugin.repeat", "required_capabilities = probe-service-status,probe-service-status\n");
    let runtime = ComponentRuntimeState::new(&directory);
    let manifests = runtime.catalog().manifests.clone(); // 解析后的插件清单。
    assert_eq!(manifests["plugin.probe"].required_capabilities, [ComponentCapability::ProbeServiceStatus]);
    assert_eq!(manifests["plugin.spaced"].required_capabilities, [ComponentCapability::ReadCurrentMedia]);
    let failures = runtime.catalog().failures.clone(); // 非法声明形成的隔离失败。
    assert!(failures.iter().any(|failure| failure.component_id == "未知能力" && failure.message.contains("readRegistry")));
    assert!(failures.iter().any(|failure| failure.component_id == "重复能力"));
    assert!(!manifests.contains_key("plugin.unknown") && !manifests.contains_key("plugin.repeat"));
    std::fs::remove_dir_all(directory).unwrap();
}

/// 能力通道按实例校验：声明探活能力的插件成功，未声明的插件实例被拒绝。
#[test]
fn probe_service_status_is_authorized_per_instance()
{
    let directory = std::env::temp_dir().join(format!("desktop-tools-plugin-probe-gate-{}", std::process::id())); // 本测试独占目录。
    for (folder, id, capabilities) in [("探活", "plugin.probe", "probe-service-status"), ("时间", "plugin.clock", "read-host-time"), ("无能力", "plugin.plain", "")]
    {
        let path = directory.join(folder); // 一个待注册的插件文件夹。
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("main.js"), "var ok = 1;").unwrap();
        std::fs::write(path.join("manifest.ini"), format!("[plugin]\nid = {id}\nname = {folder}\nversion = 1.0.0\nentry = main.js\nrequired_capabilities = {capabilities}\n")).unwrap();
    }
    let runtime = ComponentRuntimeState::new(&directory); // 三个插件与官方清单共享的注册表。
    let instance = |instance_id: &str, component_id: &str| ComponentInstanceSnapshot { instance_id: instance_id.to_string(), component_id: component_id.to_string(), monitor_id: "DISPLAY1".to_string(), projection_monitor_id: None, projection_bounds: None, bounds: WorkspaceRect { left: 20.0, top: 20.0, width: 220.0, height: 116.0 }, state_version: 1, state: json!({}) }; // 当前测试复用的插件实例构造器。
    let instances = [instance("probe", "plugin.probe"), instance("clock", "plugin.clock"), instance("plain", "plugin.plain")]; // 声明者与未声明者。
    for denied in ["clock", "plain"]
    {
        assert_eq!(runtime.invoke_capability(&instances, denied, ComponentCapability::ProbeServiceStatus).unwrap_err().code, "component-capability-denied");
    }
    assert_eq!(runtime.invoke_capability(&instances, "probe", ComponentCapability::ReadHostTime).unwrap_err().code, "component-capability-denied"); // 反向也不越权。
    let result = runtime.invoke_capability(&instances, "probe", ComponentCapability::ProbeServiceStatus).expect("声明探活能力的插件应被授权");
    assert_eq!(result.capability, ComponentCapability::ProbeServiceStatus);
    assert!(["ok", "warn", "off", "unknown"].contains(&result.payload["level"].as_str().unwrap()));
    assert!(result.payload["checkedAt"].as_i64().unwrap() > 0);
    std::fs::remove_dir_all(directory).unwrap();
}
