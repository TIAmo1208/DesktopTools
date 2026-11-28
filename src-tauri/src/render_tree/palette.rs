//! 主题令牌 → 具体颜色的登记表。
//!
//! 渲染树里的颜色只以令牌名出现（见 [`crate::render_tree::tokens`] 白名单）；适配层需要把
//! 令牌映射成可绘制的颜色。仓库此前没有 Rust 侧令牌取值表，这里按 `src/styles/fenceTokens.css`
//! （桌面围栏令牌的唯一定义处）建立内置浅/深两套值，并提供 `set`/`with` 供主题切换与主题私有
//! 覆盖注入——阶段 4「主题数据化」把真实主题值喂进来即可，不需要改适配层。
//!
//! 表里缺某个令牌时**不猜默认色**：适配层会报「令牌不在颜色表」并带节点路径。

use std::collections::BTreeMap;

use crate::render::draw::Color;
use crate::render_tree::tokens::ThemeToken;

/// 令牌颜色表。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TokenPalette
{
    colors: BTreeMap<String, Color>,
}

impl TokenPalette
{
    /// 空表：所有令牌都查不到，用于测试「缺令牌报错」或按需逐项注入。
    pub fn empty() -> Self { Self::default() }

    /// 内置深色值（`fenceTokens.css` 的默认深色一套）。
    pub fn dark() -> Self
    {
        let mut palette = Self::empty();
        palette.set_rgba("--fence-border", 210, 224, 227, 97); // rgb(210 224 227 / 38%)
        palette.set_rgba("--fence-surface", 32, 38, 40, 148); // rgb(32 38 40 / 58%)
        palette.set_rgba("--fence-text", 232, 239, 240, 255); // #e8eff0
        palette.set_rgba("--fence-active", 69, 201, 140, 255); // #45c98c
        palette.set_rgba("--fence-warning", 232, 163, 63, 255); // #e8a33f
        palette.set_rgba("--fence-error", 240, 138, 128, 255); // #f08a80
        palette.set_rgba("--fence-muted", 168, 180, 182, 255); // #a8b4b6
        palette.set_rgba("--fence-focus-border", 37, 99, 235, 255); // var(--color-accent)，默认 #2563eb
        palette
    }

    /// 内置浅色值（`fenceTokens.css` 的 `data-desktop-theme="light"` 一套）。
    pub fn light() -> Self
    {
        let mut palette = Self::empty();
        palette.set_rgba("--fence-border", 55, 66, 69, 56); // rgb(55 66 69 / 22%)
        palette.set_rgba("--fence-surface", 185, 185, 185, 148); // rgb(185 185 185 / 58%)
        palette.set_rgba("--fence-text", 32, 37, 38, 255); // #202526
        palette.set_rgba("--fence-active", 11, 122, 82, 255); // #0b7a52
        palette.set_rgba("--fence-warning", 180, 119, 15, 255); // #b4770f
        palette.set_rgba("--fence-error", 163, 33, 24, 255); // #a32118
        palette.set_rgba("--fence-muted", 92, 102, 104, 255); // #5c6668
        palette.set_rgba("--fence-focus-border", 37, 99, 235, 255); // var(--color-focus)，默认 #2563eb
        palette
    }

    /// 覆盖/新增一个令牌的颜色。
    pub fn set(&mut self, token: &ThemeToken, color: Color) -> &mut Self
    {
        self.colors.insert(token.as_str().to_string(), color);
        self
    }

    /// 链式覆盖一个令牌的颜色。
    pub fn with(mut self, token: &ThemeToken, color: Color) -> Self
    {
        self.set(token, color);
        self
    }

    /// 按令牌名查颜色；不在表里返回 `None`。
    pub fn resolve_name(&self, token: &str) -> Option<Color> { self.colors.get(token).copied() }

    /// 按令牌查颜色；不在表里返回 `None`。
    pub fn resolve(&self, token: &ThemeToken) -> Option<Color> { self.resolve_name(token.as_str()) }

    /// 表里已登记多少个令牌。
    pub fn len(&self) -> usize { self.colors.len() }

    /// 表是否为空。
    pub fn is_empty(&self) -> bool { self.colors.is_empty() }

    /// 直接写入 `0xRRGGBBAA` 形式的颜色（内置值用）。
    fn set_rgba(&mut self, token: &str, red: u8, green: u8, blue: u8, alpha: u8) { self.colors.insert(token.to_string(), Color::rgba(red, green, blue, alpha)); }
}
