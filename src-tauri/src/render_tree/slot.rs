//! 显示位：方案 §3.6 的 `tile` / `island` / `settings` 三个位置。

use serde::{Deserialize, Serialize};

/// 插件的显示位（清单 `[entries]` 的键名）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplaySlot
{
    /// 格子：渲染树，画在原生桌面。
    Tile,
    /// 浮岛卡：渲染树，画在原生浮岛。
    Island,
    /// 设置条目：DOM，画在 WebView 设置窗。
    Settings,
}

impl DisplaySlot
{
    /// 全部显示位，顺序与方案 §3.6 的表一致。
    pub const ALL: [Self; 3] = [Self::Tile, Self::Island, Self::Settings];

    /// 从清单键名解析。
    pub fn parse(value: &str) -> Option<Self>
    {
        Some(match value
        {
            "tile" => Self::Tile,
            "island" => Self::Island,
            "settings" => Self::Settings,
            _ => return None,
        })
    }

    /// 清单里的键名。
    pub fn as_str(self) -> &'static str
    {
        match self
        {
            Self::Tile => "tile",
            Self::Island => "island",
            Self::Settings => "settings",
        }
    }

    /// 该显示位是否由渲染树表达（`settings` 是 DOM，不走渲染树）。
    pub fn is_render_tree(self) -> bool { matches!(self, Self::Tile | Self::Island) }
}

impl std::fmt::Display for DisplaySlot
{
    /// 输出显示位名。
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { formatter.write_str(self.as_str()) }
}
