use super::*;
use crate::contracts::settings::ApplicationTheme;

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
    assert_eq!(std::fs::canonicalize(prepare_config_directory(&source).unwrap()).unwrap(), std::fs::canonicalize(&target).unwrap());
    let occupied = root.join("occupied"); // 有用户数据的目标必须拒绝覆盖。
    std::fs::create_dir(&occupied).unwrap();
    std::fs::write(occupied.join("settings.json"), b"keep").unwrap();
    let conflicting = ApplicationPreferences { config_directory: occupied.to_string_lossy().into_owned(), ..preferences }; // 模拟再次更改目录。
    save_json_atomic(&target.join("settings.json"), &conflicting).unwrap();
    assert_eq!(prepare_config_directory(&source).unwrap(), std::fs::canonicalize(&target).unwrap());
    assert_eq!(std::fs::read(occupied.join("settings.json")).unwrap(), b"keep");
    for invalid in ["", "relative", r"C:\data\..\other", "%UNKNOWN%\\data", r"\\.\device"]
    {
        assert!(resolve_config_directory(invalid).is_err());
    }
    assert_eq!(resolve_config_directory(r"%APPDATA%\TIAmo\DesktopTools").unwrap(), PathBuf::from(std::env::var_os("APPDATA").unwrap()).join("TIAmo/DesktopTools"));
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
