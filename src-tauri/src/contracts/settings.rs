use serde::{Deserialize, Serialize};

/// 格子表面与相邻格子空白分别使用 DIP 表达。
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GridPreferences {
    pub width: u16,      // 单个格子宽度。
    pub height: u16,     // 单个格子高度。
    pub column_gap: u16, // 列间空白。
    pub row_gap: u16,    // 行间空白。
}

/// 沿用桌面 78 × 98 的网格步进。
impl Default for GridPreferences {
    fn default() -> Self {
        Self {
            width: 68,
            height: 90,
            column_gap: 10,
            row_gap: 8,
        }
    }
}

/// 前后端共用的网格输入范围。
impl GridPreferences {
    /// 限制尺寸以保证图像、两行名称及安全容量。
    pub fn validate(&self) -> Result<(), String> {
        if !(64..=160).contains(&self.width)
            || !(90..=180).contains(&self.height)
            || self.column_gap > 48
            || self.row_gap > 48
        {
            return Err("网格宽度须为 64–160，高度须为 90–180，间距须为 0–48 DIP".to_string());
        }
        Ok(())
    }
}

/// 普通窗口与桌面工作区共用的主题偏好。
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ApplicationTheme {
    #[default]
    System,
    Light,
    Dark,
}

/// 原子保存的用户界面与项目打开偏好。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct ApplicationPreferences {
    pub config_directory: String, // 下次启动使用的配置目录，支持 %APPDATA% 前缀。
    pub theme: ApplicationTheme,  // 当前用户选择的应用主题。
    pub double_click_to_open: bool, // 是否要求双击打开桌面项目。
    pub accent_color: String,     // 选中和焦点共用的十六进制主题色。
    pub use_windows_accent_color: bool, // 使用 Windows 强调色，保留自定义颜色。
    pub corner_radius: u8,        // 表面圆角的 DIP 大小。
    pub font_family: String,      // 用户选择的界面字体族。
    pub font_size: u8,            // 普通界面文字的 DIP 字号。
    pub desktop_grid: GridPreferences, // 未归类桌面格子尺寸与间距。
    pub fence_grid: GridPreferences, // 围栏平铺格子尺寸与间距。
    pub clock_enabled: bool,      // 本地时间格子显隐，隐藏不删除布局。
    pub clock_hour12: bool,       // 本地时间使用十二小时制。
    pub clock_show_seconds: bool, // 本地时间显示秒数。
    pub debug_enabled: bool,      // Debug 构建中的测试格子显隐。
    pub debug_show_details: bool, // 测试格子是否显示详细宿主字段。
    pub component_snap: bool,     // 功能格子拖动与缩放时边缘对齐。
}

/// 首次启动跟随系统，并沿用现有双击打开行为。
impl Default for ApplicationPreferences {
    fn default() -> Self {
        Self {
            theme: ApplicationTheme::System,
            config_directory: r"%APPDATA%\TIAmo\DesktopTools".to_string(),
            double_click_to_open: true,
            accent_color: "#2563eb".to_string(),
            use_windows_accent_color: false,
            corner_radius: 6,
            font_family: "Segoe UI Variable".to_string(),
            font_size: 13,
            desktop_grid: GridPreferences::default(),
            fence_grid: GridPreferences {
                column_gap: 6,
                ..Default::default()
            },
            clock_enabled: true,
            clock_hour12: false,
            clock_show_seconds: true,
            debug_enabled: true,
            debug_show_details: true,
            component_snap: true,
        }
    }
}

/// 校验配置文件和命令输入共用的外观边界。
impl ApplicationPreferences {
    /// 拒绝无效颜色、越界数值及不能安全应用的字体。
    pub fn validate(&self) -> Result<(), String> {
        crate::app::settings::resolve_config_directory(&self.config_directory)?;
        self.desktop_grid.validate()?;
        self.fence_grid.validate()?;
        if self.accent_color.len() != 7
            || !self.accent_color.starts_with('#')
            || !self.accent_color.as_bytes()[1..]
                .iter()
                .all(u8::is_ascii_hexdigit)
        {
            return Err("主题色必须为 #RRGGBB 格式".to_string());
        }
        if self.corner_radius > 12 {
            return Err("圆角必须介于 0 和 12 之间".to_string());
        }
        if !(12..=18).contains(&self.font_size) {
            return Err("字号必须介于 12 和 18 之间".to_string());
        }
        if ![
            "Segoe UI Variable",
            "Segoe UI",
            "Microsoft YaHei UI",
            "Microsoft YaHei",
            "SimSun",
            "Arial",
        ]
        .contains(&self.font_family.as_str())
        {
            return Err("不支持的界面字体".to_string());
        }
        Ok(())
    }
}

/// 设置窗口读取的完整权威状态。
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsSnapshot {
    pub preferences: ApplicationPreferences, // 已持久化的用户偏好。
    pub autostart: bool,                     // 当前 Windows 注册项是否启用本程序。
    pub version: String,                     // Cargo 构建产生的应用版本。
    pub debug_build: bool,                   // Rust 构建模式决定测试功能可用性。
}
