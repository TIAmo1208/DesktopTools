use super::validate_creatable_plugin;
use crate::contracts::ComponentManifest;

/// 造一份只用于门禁判断的插件清单。
fn manifest(id: &str, entry: &str, creatable: bool) -> ComponentManifest { ComponentManifest { id: id.to_string(), name: id.to_string(), entry: entry.to_string(), script: String::new(), required_capabilities: Vec::new(), creatable, removable: false, persistence_version: 1, max_tiles: 0, repository: String::new(), description: String::new(), author: String::new() } }

/// 非插件标识、缺失清单和未声明可创建都不能进入创建事务。
#[test]
fn create_plugin_component_rejects_ineligible_plugins()
{
    let manifests = [manifest("plugin.clock", "plugin:slot", false), manifest("official.weather", "official:weather", true)]; // 一个不可创建的插件和一份官方清单。
    assert_eq!(validate_creatable_plugin(&manifests, "official.weather").unwrap_err(), "插件标识无效");
    assert_eq!(validate_creatable_plugin(&manifests, "plugin.").unwrap_err(), "插件标识无效");
    assert_eq!(validate_creatable_plugin(&manifests, "plugin.missing").unwrap_err(), "插件清单不存在");
    assert_eq!(validate_creatable_plugin(&manifests, "plugin.clock").unwrap_err(), "该插件不支持创建");
    assert!(validate_creatable_plugin(&[manifest("plugin.clock", "plugin:slot", true)], "plugin.clock").is_ok());
}

/// 选中默认目录时偏好为空串，其他目录保留绝对路径。
#[test]
fn plugin_directory_preference_uses_empty_string_for_the_default()
{
    let config = std::env::temp_dir().join(format!("desktop-tools-plugin-pref-{}", std::process::id())); // 本测试独占配置目录。
    let default = std::path::PathBuf::from(std::env::var_os("APPDATA").unwrap()).join("TIAmo").join("plugins"); // 空偏好解析出的位置。
    assert_eq!(super::plugin_directory_preference(&default.to_string_lossy(), &config), "");
    assert_eq!(super::plugin_directory_preference(r"D:\plugins", &config), r"D:\plugins");
}

/// 一个来源校验失败时，其余来源仍生成计划。
#[test]
fn one_bad_install_source_keeps_the_other_plan()
{
    let root = std::env::temp_dir().join(format!("desktop-tools-plugin-batch-{}", std::process::id())); // 本测试独占目录。
    let good = root.join("good");
    std::fs::create_dir_all(good.join("Clock")).unwrap();
    std::fs::write(good.join("Clock/main.js"), "var ok = 1;").unwrap();
    std::fs::write(good.join("Clock/manifest.ini"), "[plugin]\nname = Clock\nversion = 1.0.0\nentry = main.js\n").unwrap();
    let bad = root.join("bad");
    std::fs::create_dir_all(&bad).unwrap();
    let prepared = super::prepare_sources(vec![good.to_string_lossy().into_owned(), bad.to_string_lossy().into_owned()], &root.join("plugins"), crate::app::settings::inspect_plugin_folder);
    assert_eq!(prepared.plans.len(), 1);
    assert_eq!(prepared.plans[0].name, "Clock");
    assert_eq!(prepared.errors.len(), 1);
    std::fs::remove_dir_all(root).unwrap();
}
