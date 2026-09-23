use serde::{Deserialize, Serialize};

/// 格子表面与相邻格子空白分别使用 DIP 表达。
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GridPreferences
{
    pub width: u16,      // 单个格子宽度。
    pub height: u16,     // 单个格子高度。
    pub column_gap: u16, // 列间空白。
    pub row_gap: u16,    // 行间空白。
}

/// 沿用桌面 78 × 98 的网格步进。
impl Default for GridPreferences
{
    /// 创建默认桌面格子尺寸与间距。
    fn default() -> Self { Self { width: 68, height: 90, column_gap: 10, row_gap: 8 } }
}

/// 前后端共用的网格输入范围。
impl GridPreferences
{
    /// 限制尺寸以保证图像、两行名称及安全容量。
    pub fn validate(&self) -> Result<(), String>
    {
        if !(64..=160).contains(&self.width) || !(90..=180).contains(&self.height) || self.column_gap > 48 || self.row_gap > 48
        {
            return Err("网格宽度须为 64–160，高度须为 90–180，间距须为 0–48 DIP".to_string());
        }
        Ok(())
    }
}

/// 界面和桌面分别选择的深浅模式。
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ApplicationTheme
{
    #[default]
    System,
    Light,
    Dark,
}

/// 桌面网格独立保存的主题与强调色。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DesktopAppearance
{
    pub theme: ApplicationTheme,        // 桌面深浅模式。
    pub accent_color: String,           // 桌面自定义强调色。
    pub use_windows_accent_color: bool, // 桌面是否使用 Windows 强调色。
}

/// 桌面主题缺省时跟随系统，使用默认蓝色且关闭系统强调色。
impl Default for DesktopAppearance
{
    /// 创建独立于界面主题的桌面默认值。
    fn default() -> Self { Self { theme: ApplicationTheme::System, accent_color: "#2563eb".to_string(), use_windows_accent_color: false } }
}

/// 原子保存的用户界面与项目打开偏好。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ApplicationPreferences
{
    pub config_directory: String,              // 下次启动使用的配置目录，支持 %APPDATA% 前缀。
    pub theme: ApplicationTheme,               // 当前用户选择的应用主题。
    pub desktop_appearance: DesktopAppearance, // 桌面独立主题，缺失时使用默认值。
    pub double_click_to_open: bool,            // 是否要求双击打开桌面项目。
    pub accent_color: String,                  // 应用界面与自绘菜单的十六进制主题色。
    pub use_windows_accent_color: bool,        // 使用 Windows 强调色，保留自定义颜色。
    pub corner_radius: u8,                     // 表面圆角的 DIP 大小。
    pub font_family: String,                   // 用户选择的界面字体族。
    pub font_size: u8,                         // 普通界面文字的 DIP 字号。
    pub desktop_grid: GridPreferences,         // 未归类桌面格子尺寸与间距。
    pub fence_grid: GridPreferences,           // 围栏平铺格子尺寸与间距。
    pub clock_enabled: bool,                   // 本地时间格子显隐，隐藏不删除布局。
    pub music_enabled: bool,                   // 音乐控制显隐，默认关闭且保留布局。
    pub collect_enabled: bool,                 // 收集箱格子显隐。
    pub collect_auto_save_on_exit: bool,       // 编辑窗退出时自动保存。
    pub collect_image_import_mode: String,     // 图片收入方式：copy、move 或 link。
    pub music_show_source: bool,               // 音乐控制显示媒体来源。
    pub clock_hour12: bool,                    // 本地时间使用十二小时制。
    pub clock_show_seconds: bool,              // 本地时间显示秒数。
    pub debug_enabled: bool,                   // Debug 构建中的测试格子显隐。
    pub debug_show_details: bool,              // 测试格子是否显示详细宿主字段。
    pub component_snap: bool,                  // 功能格子拖动与缩放时边缘对齐。
    pub follow_system_reduced_motion: bool,    // 是否由 Windows 减少动态效果决定实际动画状态。
    pub interface_animations: bool,            // 不跟随系统时使用的界面动画偏好。
    pub folder_portal_enabled: bool,           // 文件夹门户功能入口显隐。
    pub pause_on_fullscreen: bool,             // 前台全屏或最大化覆盖本屏时暂停刷新。
    pub pause_on_focus: bool,                  // 外部应用获得焦点时暂停全部桌面刷新。
}

/// 首次启动跟随系统，并沿用现有双击打开行为。
impl Default for ApplicationPreferences
{
    /// 创建首次启动的应用偏好，音乐默认关闭。
    fn default() -> Self { Self { theme: ApplicationTheme::System, desktop_appearance: DesktopAppearance::default(), config_directory: r"%APPDATA%\TIAmo\DesktopTools".to_string(), double_click_to_open: true, accent_color: "#2563eb".to_string(), use_windows_accent_color: false, corner_radius: 6, font_family: "Segoe UI Variable".to_string(), font_size: 13, desktop_grid: GridPreferences::default(), fence_grid: GridPreferences { column_gap: 6, ..Default::default() }, clock_enabled: true, music_enabled: false, collect_enabled: true, collect_auto_save_on_exit: false, collect_image_import_mode: "copy".to_string(), music_show_source: true, clock_hour12: false, clock_show_seconds: true, debug_enabled: true, debug_show_details: true, component_snap: true, follow_system_reduced_motion: true, interface_animations: true, folder_portal_enabled: true, pause_on_fullscreen: false, pause_on_focus: false } }
}

/// 校验配置文件和命令输入共用的外观边界。
impl ApplicationPreferences
{
    /// 拒绝无效颜色、越界数值及不能安全应用的字体。
    pub fn validate(&self) -> Result<(), String>
    {
        let directory = self.config_directory.trim(); // 契约只校验文本，不读取当前用户环境。
        let suffix = if directory.get(..9).is_some_and(|prefix| prefix.eq_ignore_ascii_case("%APPDATA%"))
        {
            &directory[9..]
        }
        else
        {
            directory
        }; // 只允许约定的变量前缀。
        if directory.is_empty() || suffix.contains(['%', '\0', '*', '?', '"', '<', '>', '|'])
        {
            return Err("配置目录不能为空，且不能包含未知变量或特殊字符".to_string());
        }
        if !matches!(self.collect_image_import_mode.as_str(), "copy" | "move" | "link")
        {
            return Err("图片收入方式无效".to_string());
        }
        self.desktop_grid.validate()?;
        self.fence_grid.validate()?;
        for color in [&self.accent_color, &self.desktop_appearance.accent_color]
        {
            if color.len() != 7 || !color.starts_with('#') || !color.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)
            {
                return Err("主题色必须为 #RRGGBB 格式".to_string());
            }
        }
        if self.corner_radius > 12
        {
            return Err("圆角必须介于 0 和 12 之间".to_string());
        }
        if !(12..=18).contains(&self.font_size)
        {
            return Err("字号必须介于 12 和 18 之间".to_string());
        }
        if !["Segoe UI Variable", "Segoe UI", "Microsoft YaHei UI", "Microsoft YaHei", "SimSun", "Arial"].contains(&self.font_family.as_str())
        {
            return Err("不支持的界面字体".to_string());
        }
        Ok(())
    }
}

/// 设置窗口读取的完整权威状态。
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsSnapshot
{
    pub preferences: ApplicationPreferences, // 已持久化的用户偏好。
    pub autostart: bool,                     // 当前 Windows 注册项是否启用本程序。
    pub version: String,                     // Cargo 构建产生的应用版本。
    pub debug_build: bool,                   // Rust 构建模式决定测试功能可用性。
}

#[cfg(test)]
mod music_tests
{
    use super::ApplicationPreferences;

    /// 老配置默认关闭音乐，开关值必须完整通过实际序列化边界。
    #[test]
    fn music_preference_roundtrip()
    {
        let legacy: ApplicationPreferences = serde_json::from_str("{}").unwrap(); // 缺少音乐字段的旧配置。
        assert!(!legacy.music_enabled);
        for enabled in [false, true]
        {
            let mut preferences = legacy.clone(); // 当前待保存偏好。
            preferences.music_enabled = enabled;
            let value = serde_json::to_value(preferences).unwrap(); // 实际命令与存储的 JSON 字段。
            assert_eq!(value["musicEnabled"], enabled);
            assert_eq!(serde_json::from_value::<ApplicationPreferences>(value).unwrap().music_enabled, enabled);
        }
    }

    /// 动画控制缺失时跟随系统，两个开关值分别通过序列化保留。
    #[test]
    fn animation_preference_roundtrip()
    {
        let defaults: ApplicationPreferences = serde_json::from_str("{}").unwrap(); // 默认开启装饰动画。
        assert!(defaults.follow_system_reduced_motion);
        assert!(defaults.interface_animations);
        let disabled = ApplicationPreferences { follow_system_reduced_motion: false, interface_animations: false, ..defaults }; // 用户显式改为手动关闭。
        let value = serde_json::to_value(disabled).unwrap(); // 与保存命令相同的 camelCase 字段。
        assert_eq!(value["followSystemReducedMotion"], false);
        assert_eq!(value["interfaceAnimations"], false);
        let restored = serde_json::from_value::<ApplicationPreferences>(value).unwrap();
        assert!(!restored.follow_system_reduced_motion);
        assert!(!restored.interface_animations);
        assert!(serde_json::from_str::<ApplicationPreferences>(r#"{"followSystemReducedMotion":"false"}"#).is_err());
        assert!(serde_json::from_str::<ApplicationPreferences>(r#"{"interfaceAnimations":"false"}"#).is_err());
    }
}
