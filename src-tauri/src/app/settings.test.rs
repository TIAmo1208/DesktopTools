use super::*;
use crate::contracts::settings::ApplicationTheme;

/// 缺省目录随生效配置定位，显式偏好独立保存并拒绝不安全路径。
#[test]
fn plugin_directory_defaults_resolves_and_persists()
{
    let directory = std::env::temp_dir().join(format!("desktop-tools-plugin-preferences-{}", std::process::id())); // 本测试独占配置。
    let config = directory.join("config"); // 模拟用户自定义配置目录。
    let defaults: ApplicationPreferences = serde_json::from_str("{}").unwrap(); // 缺失字段直接使用当前默认值。
    assert!(defaults.plugin_directory.is_empty());
    let default_plugins = PathBuf::from(std::env::var_os("APPDATA").unwrap()).join("TIAmo").join("plugins"); // 空偏好不再跟随配置目录。
    assert_eq!(resolve_plugin_directory("", &config).unwrap(), default_plugins);
    assert_eq!(resolve_plugin_directory("  ", &config).unwrap(), default_plugins);
    assert_eq!(resolve_plugin_directory(r"%APPDATA%\TIAmo\plugin", &config).unwrap(), PathBuf::from(std::env::var_os("APPDATA").unwrap()).join("TIAmo/plugin"));
    let path = config.join("settings.json"); // 使用正式保存入口验证字段往返。
    let state = SettingsState::load(path.clone()).unwrap();
    let mut preferences = state.preferences().unwrap(); // 显式路径不随配置目录变化。
    preferences.plugin_directory = directory.join("custom-plugin").to_string_lossy().into_owned();
    state.save(preferences.clone()).unwrap();
    assert_eq!(SettingsState::load(path.clone()).unwrap().preferences().unwrap().plugin_directory, preferences.plugin_directory);
    for invalid in ["relative", r"C:\data\..\other", "%UNKNOWN%\\data", r"\\.\device", r"C:\data\file:stream"]
    {
        let invalid_preferences = ApplicationPreferences { plugin_directory: invalid.into(), ..preferences.clone() }; // 每次错误都不得覆盖已存偏好。
        assert!(state.save(invalid_preferences).is_err());
    }
    assert_eq!(state.preferences().unwrap(), preferences);
    std::fs::remove_dir_all(directory).unwrap();
}

/// 天气显示偏好沿用真实保存入口，缺失字段默认关闭且不接受伪布尔值。
#[test]
fn weather_visibility_defaults_and_persists()
{
    let directory = std::env::temp_dir().join(format!("desktop-tools-weather-preference-{}", std::process::id())); // 本测试独占目录。
    let path = directory.join("settings.json"); // 普通偏好文件，不含凭据。
    let state = SettingsState::load(path.clone()).unwrap(); // 真实加载入口。
    let mut preferences = state.preferences().unwrap(); // 待保存偏好。
    assert!(!preferences.weather_enabled);
    for enabled in [true, false]
    {
        preferences.weather_enabled = enabled;
        state.save(preferences.clone()).unwrap();
        assert_eq!(SettingsState::load(path.clone()).unwrap().preferences().unwrap(), preferences);
    }
    assert!(!serde_json::from_str::<ApplicationPreferences>("{}").unwrap().weather_enabled);
    assert!(serde_json::from_str::<ApplicationPreferences>(r#"{"weatherEnabled":"true"}"#).is_err());
    std::fs::remove_dir_all(directory).unwrap();
}

/// 已移除的天气来源偏好字段不参与反序列化，天气显隐仍按当前字段取值。
#[test]
fn retired_weather_source_preference_is_ignored()
{
    let enabled: ApplicationPreferences = serde_json::from_str(r#"{"weatherSource":"foreca","weatherEnabled":true}"#).unwrap(); // 旧版本遗留的来源字段。
    assert!(enabled.weather_enabled, "天气显隐按当前字段恢复");
    assert!(serde_json::to_value(&enabled).unwrap().get("weatherSource").is_none(), "已移除字段不再写回");
    let disabled: ApplicationPreferences = serde_json::from_str(r#"{"weatherSource":"qweather","weatherEnabled":false}"#).unwrap();
    assert!(!disabled.weather_enabled);
}

/// 格式正确的相对路径在应用层拒绝，文件和权威偏好保持原样。
#[test]
fn invalid_resolved_directory_preserves_saved_preferences()
{
    let directory = std::env::temp_dir().join(format!("desktop-tools-path-validation-{}", std::process::id())); // 独占测试目录。
    let path = directory.join("settings.json"); // 本次检查的真实保存入口。
    std::fs::create_dir_all(&directory).unwrap();
    let state = SettingsState::load(path.clone()).unwrap(); // 当前有效配置。
    let original = state.save(state.preferences().unwrap()).unwrap(); // 建立已保存事实。
    let bytes = std::fs::read(&path).unwrap(); // 失败前的文件内容。
    let invalid = ApplicationPreferences { config_directory: "relative-directory".into(), ..original.clone() }; // 格式层不解释实际路径。
    assert!(invalid.validate().is_ok());
    assert!(state.save(invalid).is_err());
    assert_eq!(state.preferences().unwrap(), original);
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    std::fs::remove_dir_all(directory).unwrap();
}

/// 缺失的桌面主题使用固定默认值，两组分别保存并在失败后保留权威状态。
#[test]
fn desktop_appearance_defaults_and_persists_independently()
{
    let directory = std::env::temp_dir().join(format!("desktop-tools-theme-split-{}", std::process::id())); // 本测试独占的持久化目录。
    let path = directory.join("settings.json"); // 缺少桌面主题的配置文件。
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(&path, r##"{"theme":"dark","accentColor":"#aabbcc","useWindowsAccentColor":true}"##).unwrap();
    let state = SettingsState::load(path.clone()).unwrap(); // 正式加载入口为缺失字段使用默认值。
    let mut preferences = state.preferences().unwrap(); // 等待分组修改的配置。
    assert_eq!(preferences.desktop_appearance.theme, ApplicationTheme::System);
    assert_eq!(preferences.desktop_appearance.accent_color, "#2563eb");
    assert!(!preferences.desktop_appearance.use_windows_accent_color);
    preferences.desktop_appearance.theme = ApplicationTheme::Dark;
    preferences.desktop_appearance.accent_color = "#b42318".to_string();
    let desktop = preferences.desktop_appearance.clone(); // 桌面自定义设置不随界面修改。
    preferences.theme = ApplicationTheme::Light;
    preferences.accent_color = "#087f5b".to_string();
    preferences.use_windows_accent_color = false;
    state.save(preferences.clone()).unwrap();
    let restored = SettingsState::load(path.clone()).unwrap().preferences().unwrap(); // 重启后两组保持不同值。
    assert_eq!(restored, preferences);
    assert_eq!(restored.desktop_appearance, desktop);
    preferences.desktop_appearance.accent_color = "invalid".to_string();
    assert!(state.save(preferences).is_err());
    assert_eq!(state.preferences().unwrap(), restored);
    let mut payload = serde_json::to_value(&restored).unwrap(); // 验证命令入口拒绝无效桌面主题。
    payload["desktopAppearance"]["theme"] = "invalid".into();
    assert!(serde_json::from_value::<ApplicationPreferences>(payload).is_err());
    std::fs::remove_dir_all(directory).unwrap();
}

/// 验证重启迁移两个配置及备份、源文件保留、冲突回退和路径校验。
#[test]
fn configuration_directory_moves_on_next_start_without_overwriting()
{
    let root = std::env::temp_dir().join(format!("desktop-tools-migration-{}", std::process::id())); // 本测试独占目录。
    let source = root.join("bootstrap"); // 模拟旧版本配置目录。
    let target = root.join("target"); // 下次启动目录。
    std::fs::create_dir_all(source.join("workspace-layout.snapshots")).unwrap();
    let preferences = ApplicationPreferences { config_directory: target.to_string_lossy().into_owned(), ..Default::default() }; // 已保存的迁移意图。
    save_json_atomic(&source.join("settings.json"), &preferences).unwrap();
    std::fs::write(source.join("workspace-layout.json"), b"{\"revision\":7}").unwrap();
    std::fs::write(source.join("workspace-layout.snapshots/backup.json"), b"{}").unwrap();
    assert!(!target.exists());
    assert_eq!(std::fs::canonicalize(prepare_config_directory(&source).unwrap()).unwrap(), std::fs::canonicalize(&target).unwrap());
    assert_eq!(std::fs::read(target.join("workspace-layout.json")).unwrap(), std::fs::read(source.join("workspace-layout.json")).unwrap());
    assert!(target.join("workspace-layout.snapshots/backup.json").exists());
    assert!(source.join("settings.json").exists());
    assert_eq!(load_json::<PathBuf>(&source.join("config-location.json")).unwrap(), Some(target.clone()), "配置指针写调用方目标路径，不含 verbatim 前缀");
    assert_eq!(std::fs::canonicalize(prepare_config_directory(&source).unwrap()).unwrap(), std::fs::canonicalize(&target).unwrap());
    let occupied = root.join("occupied"); // 有用户数据的目标必须拒绝覆盖。
    std::fs::create_dir(&occupied).unwrap();
    std::fs::write(occupied.join("settings.json"), b"keep").unwrap();
    let conflicting = ApplicationPreferences { config_directory: occupied.to_string_lossy().into_owned(), ..preferences }; // 模拟再次更改目录。
    save_json_atomic(&target.join("settings.json"), &conflicting).unwrap();
    assert_eq!(std::fs::canonicalize(prepare_config_directory(&source).unwrap()).unwrap(), std::fs::canonicalize(&target).unwrap());
    assert_eq!(std::fs::read(occupied.join("settings.json")).unwrap(), b"keep");
    for invalid in ["", "relative", r"C:\data\..\other", "%UNKNOWN%\\data", r"\\.\device"]
    {
        assert!(resolve_config_directory(invalid).is_err());
    }
    assert_eq!(resolve_config_directory(r"%APPDATA%\TIAmo\DesktopTools").unwrap(), PathBuf::from(std::env::var_os("APPDATA").unwrap()).join("TIAmo/DesktopTools"));
    std::fs::remove_dir_all(root).unwrap();
}

/// 插件位置指针缺失时只写基线；偏好保存后重启才整目录迁移并更新指针。
#[test]
fn plugin_directory_moves_on_next_start_after_preferences_saved()
{
    let root = std::env::temp_dir().join(format!("desktop-tools-plugin-directory-migration-{}", std::process::id())); // 本测试独占目录。
    let bootstrap = root.join("bootstrap"); // 启动定位目录，指针放在这里。
    let config = root.join("config"); // 当前生效配置目录。
    let old = root.join("plugins"); // 当前生效插件目录。
    let new = root.join("moved-plugins"); // 下次启动目标。
    std::fs::create_dir_all(&bootstrap).unwrap();
    std::fs::create_dir_all(&config).unwrap();
    std::fs::create_dir_all(old.join("Clock")).unwrap();
    std::fs::write(old.join("Clock/manifest.ini"), b"[plugin]\nid = plugin.clock\n").unwrap();
    let pointer = bootstrap.join("plugin-location.json"); // 插件位置指针。
    let preferences = ApplicationPreferences { plugin_directory: old.to_string_lossy().into_owned(), ..Default::default() }; // 首次定位的偏好。
    assert_eq!(prepare_plugin_directory(&bootstrap, &config, &preferences).unwrap(), old);
    assert_eq!(load_json::<PathBuf>(&pointer).unwrap(), Some(old.clone()));
    assert!(!new.exists(), "指针缺失只记录基线，不复制");

    let saved = ApplicationPreferences { plugin_directory: new.to_string_lossy().into_owned(), ..preferences.clone() };
    let state = SettingsState::load(config.join("settings.json")).unwrap(); // 真实保存入口。
    state.save(saved.clone()).unwrap();
    assert!(!new.exists(), "保存偏好本身不迁移目录");
    assert_eq!(effective_plugin_directory(&pointer, &state.preferences().unwrap(), &config).unwrap(), (old.clone(), true));

    let restarted = SettingsState::load(config.join("settings.json")).unwrap(); // 重启后读取新偏好。
    assert_eq!(std::fs::canonicalize(prepare_plugin_directory(&bootstrap, &config, &restarted.preferences().unwrap()).unwrap()).unwrap(), std::fs::canonicalize(&new).unwrap());
    assert_eq!(std::fs::read(new.join("Clock/manifest.ini")).unwrap(), b"[plugin]\nid = plugin.clock\n");
    assert!(old.join("Clock/manifest.ini").exists(), "原目录保留");
    let stored = load_json::<PathBuf>(&pointer).unwrap().unwrap(); // 迁移后写入的指针值。
    assert_eq!(stored, new, "插件指针写调用方目标路径");
    assert!(!stored.to_string_lossy().starts_with(r"\\?\"), "指针不含 verbatim 前缀");
    assert_eq!(effective_plugin_directory(&pointer, &saved, &config).unwrap(), (new.clone(), true), "迁移后的指针按有效值读取");
    assert_eq!(std::fs::canonicalize(prepare_plugin_directory(&bootstrap, &config, &saved).unwrap()).unwrap(), std::fs::canonicalize(&new).unwrap(), "同一目录再次启动不再迁移");
    std::fs::remove_dir_all(root).unwrap();
}

/// 连续修改插件目录可从 A 迁到 B 再迁到 C，迁移后的指针始终按有效值读取。
#[test]
fn consecutive_plugin_directory_migrations_keep_pointer_usable()
{
    let root = std::env::temp_dir().join(format!("desktop-tools-plugin-directory-chain-{}", std::process::id())); // 本测试独占目录。
    let bootstrap = root.join("bootstrap");
    let config = root.join("config");
    let first = root.join("plugins-a"); // 初始插件目录。
    let second = root.join("plugins-b"); // 第一次迁移目标。
    let third = root.join("plugins-c"); // 第二次迁移目标。
    std::fs::create_dir_all(&bootstrap).unwrap();
    std::fs::create_dir_all(&config).unwrap();
    std::fs::create_dir_all(&first).unwrap();
    std::fs::write(first.join("plugin.txt"), b"keep").unwrap();
    let pointer = bootstrap.join("plugin-location.json");
    let preferences = ApplicationPreferences { plugin_directory: first.to_string_lossy().into_owned(), ..Default::default() };
    assert_eq!(prepare_plugin_directory(&bootstrap, &config, &preferences).unwrap(), first);
    let moved = ApplicationPreferences { plugin_directory: second.to_string_lossy().into_owned(), ..preferences.clone() };
    assert_eq!(std::fs::canonicalize(prepare_plugin_directory(&bootstrap, &config, &moved).unwrap()).unwrap(), std::fs::canonicalize(&second).unwrap());
    assert_eq!(load_json::<PathBuf>(&pointer).unwrap(), Some(second.clone()), "第一次迁移后的指针不是 verbatim 形式");
    assert_eq!(effective_plugin_directory(&pointer, &moved, &config).unwrap(), (second.clone(), true), "迁移后的指针按有效值读取");
    let last = ApplicationPreferences { plugin_directory: third.to_string_lossy().into_owned(), ..moved.clone() };
    assert_eq!(std::fs::canonicalize(prepare_plugin_directory(&bootstrap, &config, &last).unwrap()).unwrap(), std::fs::canonicalize(&third).unwrap());
    assert_eq!(std::fs::read(third.join("plugin.txt")).unwrap(), b"keep", "第二次迁移从 B 复制到 C，未走缺失回退");
    assert_eq!(load_json::<PathBuf>(&pointer).unwrap(), Some(third.clone()), "第二次迁移指针仍写调用方目标路径");
    assert_eq!(effective_plugin_directory(&pointer, &last, &config).unwrap(), (third, true));
    std::fs::remove_dir_all(root).unwrap();
}

/// 目标非空时迁移失败并继续使用旧目录，偏好保持新值，清空目标后下次启动重试成功。
#[test]
fn plugin_directory_migration_keeps_old_directory_when_target_occupied()
{
    let root = std::env::temp_dir().join(format!("desktop-tools-plugin-directory-occupied-{}", std::process::id())); // 本测试独占目录。
    let bootstrap = root.join("bootstrap");
    let config = root.join("config");
    let old = root.join("plugins");
    let occupied = root.join("occupied"); // 已有用户数据的目标目录。
    std::fs::create_dir_all(&bootstrap).unwrap();
    std::fs::create_dir_all(&config).unwrap();
    std::fs::create_dir_all(&old).unwrap();
    std::fs::write(old.join("keep.json"), b"{}").unwrap();
    let preferences = ApplicationPreferences { plugin_directory: old.to_string_lossy().into_owned(), ..Default::default() };
    assert_eq!(prepare_plugin_directory(&bootstrap, &config, &preferences).unwrap(), old);
    std::fs::create_dir_all(&occupied).unwrap();
    std::fs::write(occupied.join("keep.txt"), b"keep").unwrap();
    let pointer = bootstrap.join("plugin-location.json"); // 上次生效位置。
    let blocked = ApplicationPreferences { plugin_directory: occupied.to_string_lossy().into_owned(), ..preferences.clone() };
    let pointer_before = std::fs::read(&pointer).unwrap(); // 失败前后的指针字节。
    assert_eq!(prepare_plugin_directory(&bootstrap, &config, &blocked).unwrap(), old);
    assert_eq!(std::fs::read(&pointer).unwrap(), pointer_before, "失败不切换指针");
    assert_eq!(std::fs::read(occupied.join("keep.txt")).unwrap(), b"keep");
    assert_eq!(std::fs::read(old.join("keep.json")).unwrap(), b"{}");
    std::fs::remove_dir_all(&occupied).unwrap();
    assert_eq!(std::fs::canonicalize(prepare_plugin_directory(&bootstrap, &config, &blocked).unwrap()).unwrap(), std::fs::canonicalize(&occupied).unwrap());
    assert_eq!(std::fs::read(occupied.join("keep.json")).unwrap(), b"{}");
    std::fs::remove_dir_all(root).unwrap();
}

/// 插件目录互相包含、目标含链接或重解析点时拒绝迁移，数据保持原样。
#[test]
fn plugin_directory_migration_rejects_containment_and_links()
{
    let root = std::env::temp_dir().join(format!("desktop-tools-plugin-directory-reject-{}", std::process::id())); // 本测试独占目录。
    let bootstrap = root.join("bootstrap");
    let config = root.join("config");
    let old = root.join("plugins");
    std::fs::create_dir_all(&bootstrap).unwrap();
    std::fs::create_dir_all(&config).unwrap();
    std::fs::create_dir_all(&old).unwrap();
    std::fs::write(old.join("main.js"), b"var ok = 1;").unwrap();
    let preferences = ApplicationPreferences { plugin_directory: old.to_string_lossy().into_owned(), ..Default::default() };
    assert_eq!(prepare_plugin_directory(&bootstrap, &config, &preferences).unwrap(), old);
    let pointer = bootstrap.join("plugin-location.json"); // 上次生效位置。
    let pointer_before = std::fs::read(&pointer).unwrap();
    for target in [old.join("nested"), root.clone()]
    {
        let blocked = ApplicationPreferences { plugin_directory: target.to_string_lossy().into_owned(), ..preferences.clone() }; // 目标在源内或源在目标内。
        assert_eq!(prepare_plugin_directory(&bootstrap, &config, &blocked).unwrap(), old);
        assert_eq!(std::fs::read(&pointer).unwrap(), pointer_before);
    }
    std::fs::remove_dir(old.join("nested")).unwrap(); // 清除包含检测留下的空目录。
    let real = root.join("real"); // 链接指向的真实目录。
    std::fs::create_dir_all(&real).unwrap();
    std::fs::write(real.join("outside.js"), b"outside").unwrap();
    let junction = old.join("Linked");
    assert!(std::process::Command::new("cmd").args(["/C", "mklink", "/J"]).arg(&junction).arg(&real).output().unwrap().status.success());
    let linked = ApplicationPreferences { plugin_directory: root.join("linked-target").to_string_lossy().into_owned(), ..preferences };
    assert_eq!(prepare_plugin_directory(&bootstrap, &config, &linked).unwrap(), old);
    assert_eq!(std::fs::read(&pointer).unwrap(), pointer_before);
    assert_eq!(std::fs::read(real.join("outside.js")).unwrap(), b"outside");
    assert!(old.join("main.js").exists());
    std::fs::remove_dir(&junction).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

/// 空偏好回退 %APPDATA%\TIAmo\plugins，指针缺失时只记录基线而不创建目录。
#[test]
fn empty_plugin_preference_baselines_to_default_directory()
{
    let bootstrap = std::env::temp_dir().join(format!("desktop-tools-plugin-directory-default-{}", std::process::id())); // 只写指针，不碰真实插件目录。
    std::fs::create_dir_all(&bootstrap).unwrap();
    let expected = PathBuf::from(std::env::var_os("APPDATA").unwrap()).join("TIAmo").join("plugins"); // 与配置目录无关的默认插件根。
    assert_eq!(prepare_plugin_directory(&bootstrap, &bootstrap.join("config"), &ApplicationPreferences::default()).unwrap(), expected);
    assert_eq!(load_json::<PathBuf>(&bootstrap.join("plugin-location.json")).unwrap(), Some(expected));
    std::fs::remove_dir_all(bootstrap).unwrap();
}

/// 来自磁盘的插件目录指针非法时被忽略，按偏好重写基线后继续启动。
#[test]
fn invalid_plugin_location_pointer_is_ignored_and_rebaselined()
{
    let root = std::env::temp_dir().join(format!("desktop-tools-plugin-pointer-invalid-{}", std::process::id())); // 本测试独占目录。
    let bootstrap = root.join("bootstrap");
    let config = root.join("config");
    let plugins = root.join("plugins"); // 偏好解析出的有效目录。
    std::fs::create_dir_all(&bootstrap).unwrap();
    std::fs::create_dir_all(&config).unwrap();
    let preferences = ApplicationPreferences { plugin_directory: plugins.to_string_lossy().into_owned(), ..Default::default() }; // 显式偏好。
    let pointer = bootstrap.join("plugin-location.json"); // 来自磁盘的指针。
    for value in [r#""relative-directory""#, r#""C:\\data\\..\\plugins""#, r#""\\\\.\\device\\plugins""#, r#""\\\\?\\C:\\plugins""#, r#""C:\\plugins*bad""#]
    {
        std::fs::write(&pointer, value).unwrap();
        assert_eq!(prepare_plugin_directory(&bootstrap, &config, &preferences).unwrap(), plugins, "{value}");
        assert_eq!(load_json::<PathBuf>(&pointer).unwrap(), Some(plugins.clone()), "{value}");
    }
    std::fs::remove_dir_all(root).unwrap();
}

/// 损坏的插件目录指针按缺失处理并重写基线，不阻止启动。
#[test]
fn corrupt_plugin_location_pointer_is_ignored_and_rebaselined()
{
    let root = std::env::temp_dir().join(format!("desktop-tools-plugin-pointer-corrupt-{}", std::process::id())); // 本测试独占目录。
    let bootstrap = root.join("bootstrap");
    let config = root.join("config");
    let plugins = root.join("plugins");
    std::fs::create_dir_all(&bootstrap).unwrap();
    std::fs::create_dir_all(&config).unwrap();
    let preferences = ApplicationPreferences { plugin_directory: plugins.to_string_lossy().into_owned(), ..Default::default() };
    let pointer = bootstrap.join("plugin-location.json");
    std::fs::write(&pointer, b"{not json").unwrap();
    assert_eq!(prepare_plugin_directory(&bootstrap, &config, &preferences).unwrap(), plugins);
    assert_eq!(load_json::<PathBuf>(&pointer).unwrap(), Some(plugins.clone()));
    std::fs::remove_dir_all(root).unwrap();
}

/// 基线写入失败只记警告，首次启动仍使用当前解析出的目录。
#[test]
fn plugin_location_write_failure_keeps_running_directory()
{
    let root = std::env::temp_dir().join(format!("desktop-tools-plugin-pointer-write-failure-{}", std::process::id())); // 本测试独占目录。
    let bootstrap = root.join("bootstrap");
    std::fs::create_dir_all(bootstrap.join("plugin-location.json")).unwrap(); // 用目录占住指针路径，原子替换必定失败。
    let plugins = root.join("plugins"); // 期望继续使用的目录。
    let preferences = ApplicationPreferences { plugin_directory: plugins.to_string_lossy().into_owned(), ..Default::default() };
    assert_eq!(prepare_plugin_directory(&bootstrap, &bootstrap, &preferences).unwrap(), plugins);
    assert!(bootstrap.join("plugin-location.json").is_dir(), "写入失败不得删除原路径");
    std::fs::remove_dir_all(root).unwrap();
}

/// 验证配置重启恢复、失败不提交，以及不接受未知主题。
#[test]
fn preferences_are_persisted_and_validated()
{
    let directory = std::env::temp_dir().join(format!("desktop-tools-settings-{}", std::process::id())); // 当前测试独占目录。
    let path = directory.join("settings.json"); // 当前测试配置路径。
    let state = SettingsState::load(path.clone()).unwrap(); // 首次启动状态。
    assert!(state.preferences().unwrap().double_click_to_open);
    let next = ApplicationPreferences { theme: ApplicationTheme::Dark, double_click_to_open: false, use_windows_accent_color: true, accent_color: "#aabbcc".to_string(), ..Default::default() }; // 待保存的非默认配置。
    state.save(next.clone()).unwrap();
    assert_eq!(SettingsState::load(path.clone()).unwrap().preferences().unwrap(), next);
    assert!(serde_json::from_str::<ApplicationPreferences>(r#"{"theme":"invalid","doubleClickToOpen":true}"#).is_err());
    let blocked = SettingsState { path: path.join("blocked.json"), preferences: Mutex::new(ApplicationPreferences::default()) }; // 文件作为父目录时必定无法写入。
    assert!(blocked.save(next).is_err());
    let invalid = ApplicationPreferences { font_size: 19, ..Default::default() }; // 不能持久化的越界外观配置。
    assert!(state.save(invalid).is_err());
    let legacy: ApplicationPreferences = serde_json::from_str(r#"{"theme":"light","doubleClickToOpen":true}"#).unwrap(); // 旧版字段缺失时的兼容配置。
    assert_eq!(legacy.font_size, 13);
    assert_eq!(legacy.corner_radius, 6);
    assert!(!legacy.use_windows_accent_color);
    assert!(legacy.validate().is_ok());
    assert_eq!(blocked.preferences().unwrap(), ApplicationPreferences::default());
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir(directory).unwrap();
}

/// 已删除的顶层偏好不阻塞启动，并在下一次保存时自然移除。
#[test]
fn retired_preference_fields_do_not_block_startup()
{
    let directory = std::env::temp_dir().join(format!("desktop-tools-retired-settings-{}", std::process::id())); // 本测试独占目录。
    let path = directory.join("settings.json"); // 包含已删除字段的旧设置文件。
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(&path, r#"{"theme":"light","musicEnabled":true,"musicShowArtwork":true}"#).unwrap();

    let state = SettingsState::load(path.clone()).unwrap(); // 旧字段被忽略，当前字段继续恢复。
    let preferences = state.preferences().unwrap();
    assert_eq!(preferences.theme, ApplicationTheme::Light);
    assert!(preferences.music_enabled);
    state.save(preferences).unwrap();

    let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert!(saved.get("musicShowArtwork").is_none());
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir(directory).unwrap();
}

/// 设置键是作者@插件ID。作者为空时仍保留 @。非法标识不产生键。
#[test]
fn plugin_settings_key_joins_author_and_rejects_illegal_ids()
{
    assert_eq!(plugin_settings_key("TIAmo", "plugin.item").unwrap(), "TIAmo@plugin.item");
    assert_eq!(plugin_settings_key("", "plugin.item").unwrap(), "@plugin.item");
    assert_eq!(plugin_settings_key("", "plugin.my-clock_2").unwrap(), "@plugin.my-clock_2");
    assert!(plugin_settings_author(&[], "plugin.clock").unwrap_err().contains("清单不存在"));
    for plugin_id in ["clock", "official.weather", "plugin.", "Plugin.clock", "plugin.时钟", "plugin.clock/../x", r"plugin.clock\..\x", "plugin.clock:stream", "plugin.clock ", "", "plugin.clock\0"]
    {
        assert!(plugin_settings_key("TIAmo", plugin_id).is_err(), "{plugin_id}");
    }
}

/// 缺失文件返回空；损坏的 JSON 保持原字节，非法标识不另建文件。
#[test]
fn missing_or_corrupt_plugin_settings_do_not_overwrite()
{
    let root = std::env::temp_dir().join(format!("desktop-tools-plugin-settings-{}", std::process::id())); // 本测试独占配置目录。
    std::fs::create_dir_all(&root).unwrap();
    assert_eq!(read_plugin_settings(&root, "", "plugin.clock").unwrap(), None);
    assert!(!root.join("settings-plugins.json").exists());
    let path = root.join("settings-plugins.json"); // 已损坏的用户文件。
    let bytes = b"{not json";
    std::fs::write(&path, bytes).unwrap();
    assert_eq!(read_plugin_settings(&root, "", "plugin.clock").unwrap(), None);
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert!(write_plugin_settings(&root, "", "plugin.clock/../outside", &serde_json::json!({})).is_err());
    assert!(write_plugin_settings(&root, "TIAmo", "plugin.clock", &serde_json::json!({"hour": 12})).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert!(!root.join("outside").exists());
    std::fs::remove_file(&path).unwrap();
    write_plugin_settings(&root, "", "plugin.clock", &serde_json::json!({"hour": 12})).unwrap();
    assert_eq!(read_plugin_settings(&root, "", "plugin.clock").unwrap(), Some(serde_json::json!({"hour": 12})));
    let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(saved["@plugin.clock"]["hour"], 12);
    std::fs::remove_dir_all(root).unwrap();
}

/// 配置目录迁移复制整个目录，插件设置文件随目录离开。
#[test]
fn configuration_directory_migration_takes_plugin_settings()
{
    let root = std::env::temp_dir().join(format!("desktop-tools-plugin-settings-migration-{}", std::process::id())); // 本测试独占目录。
    let source = root.join("bootstrap"); // 当前生效配置目录。
    let target = root.join("target"); // 下次启动目录。
    std::fs::create_dir_all(source.join("nested")).unwrap();
    let preferences = ApplicationPreferences { config_directory: target.to_string_lossy().into_owned(), ..Default::default() }; // 已保存的迁移意图。
    save_json_atomic(&source.join("settings.json"), &preferences).unwrap();
    std::fs::write(source.join("settings-plugins.json"), b"{\"@plugin.clock\":{\"hour\":1}}").unwrap();
    std::fs::write(source.join("nested/extra.txt"), b"keep").unwrap();
    assert_eq!(std::fs::canonicalize(prepare_config_directory(&source).unwrap()).unwrap(), std::fs::canonicalize(&target).unwrap());
    assert_eq!(std::fs::read(target.join("settings-plugins.json")).unwrap(), b"{\"@plugin.clock\":{\"hour\":1}}");
    assert_eq!(std::fs::read(target.join("nested/extra.txt")).unwrap(), b"keep");
    assert_eq!(std::fs::read(source.join("settings-plugins.json")).unwrap(), b"{\"@plugin.clock\":{\"hour\":1}}");
    std::fs::remove_dir_all(root).unwrap();
}

/// 停用名单缺失为空，非法与重复条目丢弃，超过 64 条截断。
#[test]
fn disabled_plugins_drop_invalid_entries_and_keep_sixty_four()
{
    let missing: ApplicationPreferences = serde_json::from_str("{}").unwrap(); // 旧文件没有该字段。
    assert!(missing.disabled_plugins.is_empty());
    let parsed: ApplicationPreferences = serde_json::from_str(r#"{"disabledPlugins":["plugin.clock","official.weather","plugin.clock","plugin.","plugin.b"]}"#).unwrap();
    assert_eq!(parsed.disabled_plugins, vec!["plugin.clock".to_string(), "plugin.b".to_string()]);
    let overflow = (0..70).map(|index| format!("plugin.n{index}")).collect::<Vec<_>>(); // 超过上限的合法标识。
    assert_eq!(crate::contracts::settings::normalize_disabled_plugins(overflow).len(), 64);
    let directory = std::env::temp_dir().join(format!("desktop-tools-disabled-plugins-{}", std::process::id())); // 本测试独占配置目录。
    let path = directory.join("settings.json");
    let state = SettingsState::load(path).unwrap();
    let mut preferences = state.preferences().unwrap();
    preferences.disabled_plugins = vec!["plugin.ok".into(), "bad".into(), "plugin.ok".into()];
    assert_eq!(state.save(preferences).unwrap().disabled_plugins, vec!["plugin.ok".to_string()]);
    std::fs::remove_dir_all(directory).unwrap();
}

/// 卸载只删除该插件的清单和脚本，设置文件、其他插件和目录外文件都保留。
#[test]
fn uninstall_plugin_keeps_settings_and_rejects_illegal_paths()
{
    let root = std::env::temp_dir().join(format!("desktop-tools-plugin-uninstall-{}", std::process::id())); // 本测试独占目录。
    let directory = root.join("plugin"); // 插件根。
    std::fs::create_dir_all(&directory).unwrap();
    let package = directory.join("时钟"); // 卸载删的是整个文件夹。
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(package.join("main.js"), "var ok = 1;").unwrap();
    std::fs::write(package.join("manifest.ini"), "[plugin]\nid = plugin.ok\nname = 时钟\nversion = 1.0.0\nentry = main.js\n").unwrap();
    std::fs::write(directory.join("plugin.ok.json"), "{}").unwrap();
    std::fs::write(directory.join("settings-plugins.json"), "{\"@plugin.ok\":{\"hour\":1}}").unwrap();
    std::fs::write(directory.join("plugin.other.js"), "var other = 1;").unwrap();
    std::fs::write(root.join("outside.js"), "pwned").unwrap();
    assert_eq!(plugin_manifest_file(&directory, "plugin.ok").unwrap(), package.join("manifest.ini").canonicalize().unwrap());
    assert_eq!(uninstall_plugin_files(&directory, &directory, "plugin.missing", false, "").unwrap(), Vec::<String>::new());
    assert_eq!(uninstall_plugin_files(&directory, &directory, "plugin.ok", false, "").unwrap(), vec!["时钟".to_string()]);
    assert!(!package.exists());
    assert_eq!(std::fs::read(directory.join("plugin.ok.json")).unwrap(), b"{}");
    assert_eq!(std::fs::read(directory.join("settings-plugins.json")).unwrap(), b"{\"@plugin.ok\":{\"hour\":1}}");
    assert_eq!(std::fs::read(directory.join("plugin.other.js")).unwrap(), b"var other = 1;");
    assert_eq!(std::fs::read(root.join("outside.js")).unwrap(), b"pwned");
    for plugin_id in ["official.weather", "plugin.", "../outside", "plugin.ok/../../outside", r"plugin.ok\..\outside"]
    {
        assert!(uninstall_plugin_files(&directory, &directory, plugin_id, true, "").is_err(), "{plugin_id}");
    }
    assert_eq!(std::fs::read(directory.join("plugin.ok.json")).unwrap(), b"{}");
    assert_eq!(std::fs::read(root.join("outside.js")).unwrap(), b"pwned");
    let real = root.join("real"); // 联接文件夹不能被当成插件包删掉。
    std::fs::create_dir_all(&real).unwrap();
    std::fs::write(real.join("main.js"), "var link = 1;").unwrap();
    std::fs::write(real.join("manifest.ini"), "[plugin]\nid = plugin.link\nname = 时钟\nversion = 1.0.0\nentry = main.js\n").unwrap();
    let junction = directory.join("时钟");
    assert!(std::process::Command::new("cmd").args(["/C", "mklink", "/J"]).arg(&junction).arg(&real).output().unwrap().status.success());
    assert_eq!(uninstall_plugin_files(&directory, &directory, "plugin.link", true, "").unwrap(), Vec::<String>::new());
    assert_eq!(std::fs::read(real.join("manifest.ini")).unwrap().is_empty(), false);
    assert!(junction.exists());
    std::fs::remove_dir(&junction).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

/// remove_settings 为假时保留配置目录中的设置；为真时只删这一份，且设置文件是重解析点时清单也不删。
#[test]
fn uninstall_plugin_remove_settings_is_optional_and_confined()
{
    let root = std::env::temp_dir().join(format!("desktop-tools-plugin-remove-settings-{}", std::process::id())); // 本测试独占目录。
    let plugin_directory = root.join("plugin"); // 清单与脚本所在目录。
    let config_directory = root.join("config"); // 设置文件所在目录，与插件根分开。
    std::fs::create_dir_all(&plugin_directory).unwrap();
    std::fs::create_dir_all(&config_directory).unwrap();
    let package = plugin_directory.join("时钟"); // 插件包是文件夹，不是扁平 json。
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(package.join("main.js"), "var ok = 1;").unwrap();
    std::fs::write(package.join("manifest.ini"), "[plugin]\nid = plugin.ok\nname = 时钟\nversion = 1.0.0\nentry = main.js\n").unwrap();
    std::fs::write(config_directory.join("settings-plugins.json"), "{\"@plugin.ok\":{\"hour\":1},\"TIAmo@plugin.other\":{}}").unwrap();
    std::fs::write(root.join("outside.js"), "pwned").unwrap();
    assert_eq!(plugin_settings_key("", "plugin.ok").unwrap(), "@plugin.ok");
    assert_eq!(uninstall_plugin_files(&plugin_directory, &config_directory, "plugin.ok", false, "").unwrap(), vec!["时钟".to_string()]);
    assert_eq!(std::fs::read(config_directory.join("settings-plugins.json")).unwrap(), b"{\"@plugin.ok\":{\"hour\":1},\"TIAmo@plugin.other\":{}}");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(package.join("main.js"), "var ok = 1;").unwrap();
    std::fs::write(package.join("manifest.ini"), "[plugin]\nid = plugin.ok\nname = 时钟\nversion = 1.0.0\nentry = main.js\n").unwrap();
    assert_eq!(uninstall_plugin_files(&plugin_directory, &config_directory, "plugin.ok", true, "").unwrap(), vec!["时钟".to_string()]);
    let left: serde_json::Value = serde_json::from_slice(&std::fs::read(config_directory.join("settings-plugins.json")).unwrap()).unwrap();
    assert!(left.get("@plugin.ok").is_none());
    assert!(left.get("TIAmo@plugin.other").is_some());
    assert_eq!(std::fs::read(root.join("outside.js")).unwrap(), b"pwned");
    std::fs::write(config_directory.join("settings-plugins.json"), "{\"@plugin.ok\":{\"hour\":1}}").unwrap();
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(package.join("main.js"), "var ok = 1;").unwrap();
    std::fs::write(package.join("manifest.ini"), "[plugin]\nid = plugin.ok\nname = 时钟\nversion = 1.0.0\nentry = main.js\n").unwrap();
    assert_eq!(uninstall_plugin_files(&plugin_directory, &config_directory, "plugin.ok", true, "").unwrap(), vec!["时钟".to_string(), "settings-plugins.json".to_string()]);
    assert!(!config_directory.join("settings-plugins.json").exists());
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(package.join("main.js"), "var ok = 1;").unwrap();
    std::fs::write(package.join("manifest.ini"), "[plugin]\nid = plugin.ok\nname = 时钟\nversion = 1.0.0\nentry = main.js\n").unwrap();
    let junction = config_directory.join("settings-plugins.json"); // 设置文件本身是重解析点时不得先删插件文件夹。
    assert!(std::process::Command::new("cmd").args(["/C", "mklink", "/J"]).arg(&junction).arg(&plugin_directory).output().unwrap().status.success());
    assert!(uninstall_plugin_files(&plugin_directory, &config_directory, "plugin.ok", true, "").is_err());
    assert!(package.join("manifest.ini").exists());
    assert!(junction.exists());
    std::fs::remove_dir(&junction).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

fn sample_manifest(id: &str, name: &str, version: &str) -> String { format!("[plugin]\nid = {id}\nname = {name}\nversion = {version}\nentry = main.js\n") }

fn write_package(directory: &std::path::Path, name: &str, manifest: &str)
{
    let package = directory.join(name); // 一层子目录里的插件。
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(package.join("main.js"), "var ok = 1;").unwrap();
    std::fs::write(package.join("manifest.ini"), manifest).unwrap();
}

/// 计划带上待装作者；已经装过时带上那份清单里的插件标识。
#[test]
fn install_plan_carries_author_and_installed_id()
{
    let root = std::env::temp_dir().join(format!("desktop-tools-plugin-plan-author-{}", std::process::id())); // 本测试独占目录。
    let plugin_root = root.join("plugins");
    let source = root.join("source");
    write_package(&source, "Clock", "[plugin]\nname = Clock\nversion = 1.0.0\nentry = main.js\nauthor = 张三\n");
    let fresh = inspect_plugin_folder(&source, &plugin_root).unwrap();
    assert_eq!(fresh.author, "张三");
    assert!(fresh.conflict.is_none());
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let plugin_id = install_prepared_plugin(&plugin_root, &fresh, false, &cancel, |_| panic!("新安装不应卸载"), |_, _| ()).unwrap();
    let again = root.join("again");
    write_package(&again, "Clock", "[plugin]\nname = Clock\nversion = 1.1.0\nentry = main.js\nauthor = 李四\n");
    let upgrade = inspect_plugin_folder(&again, &plugin_root).unwrap();
    assert_eq!(upgrade.author, "李四");
    let conflict = upgrade.conflict.expect("已安装同名插件应带上冲突");
    assert_eq!(conflict.installed_id, plugin_id);
    assert!(!conflict.downgrade);
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn plugin_install_plans_versions_and_cancellation()
{
    let root = std::env::temp_dir().join(format!("desktop-tools-plugin-install-plan-{}", std::process::id())); // 本测试独占目录。
    let source = root.join("source");
    let plugin_root = root.join("plugins");
    write_package(&source, "Clock", &sample_manifest("author.bad", "Clock", "1.0.0"));
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let prepared = inspect_plugin_folder(&source, &plugin_root).unwrap();
    assert!(prepared.conflict.is_none());
    assert!(prepared.author.is_empty());
    let plugin_id = install_prepared_plugin(&plugin_root, &prepared, false, &cancel, |_| panic!("新安装不应卸载"), |_, _| ()).unwrap();
    assert_eq!(plugin_id, "plugin.clock");
    assert!(std::fs::read_to_string(plugin_root.join("Clock/manifest.ini")).unwrap().contains("id = plugin.clock"));
    assert!(std::fs::read_dir(&plugin_root).unwrap().all(|entry| !entry.unwrap().file_name().to_string_lossy().starts_with(".install-")));
    let newer = root.join("newer");
    write_package(&newer, "Clock", &sample_manifest("author.bad", "Clock", "1.1.0"));
    let upgrade = inspect_plugin_folder(&newer, &plugin_root).unwrap();
    assert!(!upgrade.conflict.as_ref().unwrap().downgrade);
    assert_eq!(upgrade.conflict.as_ref().unwrap().installed_id, plugin_id);
    let older = root.join("older");
    write_package(&older, "Clock", &sample_manifest("author.bad", "Clock", "0.9.0"));
    let downgrade = inspect_plugin_folder(&older, &plugin_root).unwrap();
    assert!(downgrade.conflict.as_ref().unwrap().downgrade);
    assert!(install_prepared_plugin(&plugin_root, &downgrade, false, &cancel, |_| panic!("未确认不得卸载"), |_, _| ()).is_err());
    assert!(std::fs::read_to_string(plugin_root.join("Clock/manifest.ini")).unwrap().contains("version = 1.0.0"));
    let mut removed = String::new();
    install_prepared_plugin(
        &plugin_root,
        &downgrade,
        true,
        &cancel,
        |id| {
            removed = id.to_string();
            std::fs::remove_dir_all(plugin_root.join("Clock")).unwrap();
            Ok(())
        },
        |_, _| (),
    )
    .unwrap();
    assert_eq!(removed, "plugin.clock");
    assert!(std::fs::read_to_string(plugin_root.join("Clock/manifest.ini")).unwrap().contains("version = 0.9.0"));
    let nested = root.join("nested-source");
    std::fs::create_dir_all(nested.join("Inner")).unwrap();
    std::fs::write(nested.join("Inner/main.js"), "var nested = 1;").unwrap();
    std::fs::write(nested.join("Inner/manifest.ini"), sample_manifest("nope", "Inner", "1.0.0")).unwrap();
    assert_eq!(inspect_plugin_folder(&nested, &plugin_root).unwrap().name, "Inner");
    cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    let fresh = inspect_plugin_folder(&nested, &plugin_root).unwrap();
    assert!(install_prepared_plugin(&plugin_root, &fresh, false, &cancel, |_| Ok(()), |_, _| ()).unwrap_err().contains("取消"));
    assert!(!plugin_root.join("Inner").exists());
    assert!(reject_zip_entry("../evil.js", None, 1).is_err());
    assert!(reject_zip_entry("main.js", Some(0o120777), 1).is_err());
    assert!(reject_zip_entry("main.js", None, 64 * 1024 * 1024 + 1).is_err());
    assert!(!install_progress_due(false, 99, 1));
    assert!(install_progress_due(false, 100, 0));
    assert!(install_progress_due(false, 0, 2));
    let archive = root.join("plugin.zip");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
    let options = zip::write::SimpleFileOptions::default();
    zip.start_file("Wrapped/manifest.ini", options).unwrap();
    use std::io::Write;
    zip.write_all(sample_manifest("author.bad", "Wrapped", "1.0.0").as_bytes()).unwrap();
    zip.start_file("Wrapped/main.js", options).unwrap();
    zip.write_all(b"var zip = 1;").unwrap();
    zip.finish().unwrap();
    let from_zip = inspect_plugin_archive(&archive, &plugin_root).unwrap();
    cancel.store(false, std::sync::atomic::Ordering::Relaxed);
    assert_eq!(install_prepared_plugin(&plugin_root, &from_zip, false, &cancel, |_| Ok(()), |_, _| ()).unwrap(), "plugin.wrapped");
    let escape = root.join("escape.zip");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&escape).unwrap());
    zip.start_file("../evil.js", options).unwrap();
    zip.write_all(b"bad").unwrap();
    zip.finish().unwrap();
    assert!(inspect_plugin_archive(&escape, &plugin_root).is_err());
    let crowded = root.join("crowded.zip");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&crowded).unwrap());
    for index in 0..=2000
    {
        zip.start_file(format!("f{index}.txt"), options).unwrap();
    }
    zip.finish().unwrap();
    assert!(inspect_plugin_archive(&crowded, &plugin_root).unwrap_err().contains("2000"));
    std::fs::remove_dir_all(root).unwrap();
}

/// 安装拒绝把缺键、版本区间、非法名称、越界入口和 zip 路径写进错误正文。
#[test]
fn plugin_install_rejections_name_the_reason()
{
    let root = std::env::temp_dir().join(format!("desktop-tools-plugin-install-reason-{}", std::process::id())); // 本测试独占目录。
    let plugin_root = root.join("plugins");
    std::fs::create_dir_all(&plugin_root).unwrap();
    let missing = root.join("missing");
    write_package(&missing, "Pack", "[plugin]\nversion = 1.0.0\nentry = main.js\n");
    let missing_name = inspect_plugin_folder(&missing, &plugin_root).unwrap_err();
    assert!(missing_name.contains("缺少 name"), "{missing_name}");
    let high = root.join("high");
    write_package(&high, "High", "[plugin]\nname = High\nversion = 1.0.0\nentry = main.js\nminAppVersion = 99.0.0\n");
    let too_new = inspect_plugin_folder(&high, &plugin_root).unwrap_err();
    assert!(too_new.contains("99.0.0") && too_new.contains(env!("CARGO_PKG_VERSION")), "{too_new}");
    let low = root.join("low");
    write_package(&low, "Low", "[plugin]\nname = Low\nversion = 1.0.0\nentry = main.js\nmaxAppVersion = 0.0.1\n");
    let too_old = inspect_plugin_folder(&low, &plugin_root).unwrap_err();
    assert!(too_old.contains("0.0.1") && too_old.contains(env!("CARGO_PKG_VERSION")), "{too_old}");
    let named = root.join("named");
    write_package(&named, "Bad", "[plugin]\nname = a/b\nversion = 1.0.0\nentry = main.js\n");
    let bad_name = inspect_plugin_folder(&named, &plugin_root).unwrap_err();
    assert!(bad_name.contains("名称无效") && bad_name.contains("a/b"), "{bad_name}");
    let escaped = root.join("escaped");
    write_package(&escaped, "Out", "[plugin]\nname = Out\nversion = 1.0.0\nentry = ../main.js\n");
    let outside = inspect_plugin_folder(&escaped, &plugin_root).unwrap_err();
    assert!(outside.contains("../main.js"), "{outside}");
    let absent = root.join("absent");
    std::fs::create_dir_all(absent.join("Gone")).unwrap();
    std::fs::write(absent.join("Gone/manifest.ini"), "[plugin]\nname = Gone\nversion = 1.0.0\nentry = missing.js\n").unwrap();
    assert!(inspect_plugin_folder(&absent, &plugin_root).is_ok(), "入口脚本缺失不再拒绝安装");
    let archive = root.join("escape.zip");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
    zip.start_file("../evil.js", zip::write::SimpleFileOptions::default()).unwrap();
    use std::io::Write;
    zip.write_all(b"bad").unwrap();
    zip.finish().unwrap();
    let zip_error = inspect_plugin_archive(&archive, &plugin_root).unwrap_err();
    assert!(zip_error.contains("非法路径") && zip_error.contains("../evil.js"), "{zip_error}");
    std::fs::remove_dir_all(root).unwrap();
}

/// 入口脚本还没放进文件夹时，安装计划仍然成立。
#[test]
fn missing_entry_script_can_still_be_prepared()
{
    let root = std::env::temp_dir().join(format!("desktop-tools-plugin-missing-entry-{}", std::process::id())); // 本测试独占目录。
    let plugin_root = root.join("plugins");
    std::fs::create_dir_all(&plugin_root).unwrap();
    let source = root.join("NoEntry"); // 只有清单，没有 main.js。
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(source.join("manifest.ini"), "[plugin]\nname = NoEntry\nversion = 1.0.0\nentry = main.js\n").unwrap();
    let prepared = inspect_plugin_folder(&source, &plugin_root).expect("缺入口脚本仍应得到安装计划");
    assert_eq!(prepared.name, "NoEntry");
    std::fs::remove_dir_all(root).unwrap();
}
