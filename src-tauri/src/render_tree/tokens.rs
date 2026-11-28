//! 主题令牌校验：渲染树里的颜色只能引用桌面层 `--fence-*` 令牌名。
//!
//! 依据 `docs/design/主题变量契约.md`「桌面网格（围栏）」：桌面层插件内容只读取本组令牌，
//! 不得回退到界面调色板的 `--color-*`。这里把这条约束落成可执行白名单，裸色值与未知令牌
//! 一律报错，而不是悄悄退回某个默认色。

use std::fmt;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// 桌面层允许引用的主题令牌（与主题变量契约的「桌面网格（围栏）」一节一一对应）。
///
/// `--fence-item-text-shadow` 是阴影值而不是颜色，本模块的样式子集不接收阴影，故不在此列。
pub const FENCE_TOKENS: &[&str] = &["--fence-border", "--fence-surface", "--fence-text", "--fence-active", "--fence-warning", "--fence-error", "--fence-muted", "--fence-focus-border"];

/// 令牌校验失败的原因。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TokenError
{
    /// 传入了裸色值（十六进制、`rgb()`/`hsl()`/`color()` 或颜色关键字）。
    BareColor(String),
    /// 令牌名不在桌面层白名单里。
    Unknown(String),
}

impl fmt::Display for TokenError
{
    /// 输出中文原因，供上层拼进 [`crate::render_tree::error::RenderTreeError`]。
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result
    {
        match self
        {
            Self::BareColor(value) => write!(formatter, "不接受裸色值 {value}"),
            Self::Unknown(value) => write!(formatter, "令牌 {value} 不在桌面层白名单"),
        }
    }
}

impl std::error::Error for TokenError {}

/// 已校验的主题令牌名。
///
/// 只允许 [`FENCE_TOKENS`] 中的名字；序列化成原始字符串，反序列化时重新校验，
/// 因而跨进程传递或手写 JSON 都绕不过这条规则。
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ThemeToken(String);

impl ThemeToken
{
    /// 校验令牌名并构造。
    pub fn parse(value: &str) -> Result<Self, TokenError>
    {
        let trimmed = value.trim(); // 容忍插件写出的首尾空白，但不接受空串。
        if looks_like_bare_color(trimmed)
        {
            return Err(TokenError::BareColor(trimmed.to_string()));
        }
        if !FENCE_TOKENS.contains(&trimmed)
        {
            return Err(TokenError::Unknown(trimmed.to_string()));
        }
        Ok(Self(trimmed.to_string()))
    }

    /// 令牌原文（保留 `--` 前缀）。
    pub fn as_str(&self) -> &str { &self.0 }
}

impl fmt::Display for ThemeToken
{
    /// 输出令牌名。
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result { formatter.write_str(&self.0) }
}

impl Serialize for ThemeToken
{
    /// 序列化为令牌名本身。
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> { serializer.serialize_str(&self.0) }
}

impl<'de> Deserialize<'de> for ThemeToken
{
    /// 反序列化时重新校验白名单。
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error>
    {
        let raw = String::deserialize(deserializer)?;
        Self::parse(&raw).map_err(D::Error::custom)
    }
}

/// 常见颜色关键字；出现在令牌位置时按裸色值处理，给出更贴近原因的报错。
const NAMED_COLORS: &[&str] = &["white", "black", "red", "green", "blue", "gray", "grey", "silver", "orange", "yellow", "purple", "pink", "brown", "transparent", "currentcolor"];

/// 判断一个字符串是否像裸色值。
fn looks_like_bare_color(value: &str) -> bool
{
    let lower = value.trim().to_ascii_lowercase();
    value.trim().starts_with('#') || lower.starts_with("rgb(") || lower.starts_with("rgba(") || lower.starts_with("hsl(") || lower.starts_with("hsla(") || lower.starts_with("oklch(") || lower.starts_with("color(") || NAMED_COLORS.contains(&lower.as_str())
}
