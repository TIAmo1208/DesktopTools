use serde::{Deserialize, Serialize};

/// 已接入的天气来源；多个来源可以同时配置，每个天气格子自己选择来源，不自动失败切源，也不拼接两家数据。
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Source
{
    #[default]
    Qweather,
    Foreca,
}

impl Source
{
    /// 设置页按此顺序列出已配置来源。
    pub const ALL: [Source; 2] = [Source::Qweather, Source::Foreca];

    /// 用于确认命令、缓存文件和前端标识的稳定键，不使用供应商显示名。
    pub fn key(self) -> &'static str
    {
        match self
        {
            Source::Qweather => "qweather",
            Source::Foreca => "foreca",
        }
    }
}
