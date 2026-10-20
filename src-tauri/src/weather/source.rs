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

    /// 该来源的搜索命中是否等价于位置可用于天气查询；只有官方明确说明时才省略验证请求。
    pub fn search_hit_proves_weather(self) -> bool
    {
        match self
        {
            Source::Qweather => false, // 官方另列“位置不支持所查数据”与数据权限错误码，必须真实请求才能判断。
            Source::Foreca => true,    // 官方 schema 标注位置标识即天气查询标识，搜索命中即可用于天气接口。
        }
    }
}
