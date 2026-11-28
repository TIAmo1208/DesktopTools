//! `render_tree::tokens` 单测：桌面层令牌通过，裸色值与界面调色板令牌被拒绝。

use super::tokens::{FENCE_TOKENS, ThemeToken, TokenError};

/// 契约里列出的桌面层令牌都能通过校验。
#[test]
fn fence_tokens_are_accepted()
{
    for name in FENCE_TOKENS
    {
        let token = ThemeToken::parse(name).unwrap_or_else(|error| panic!("{name} 应通过：{error}"));
        assert_eq!(token.as_str(), *name);
    }
}

/// 各种裸色值写法都要被识别为裸色值，而不是当成未知令牌。
#[test]
fn bare_colors_are_rejected()
{
    for value in ["#fff", "#11223344", "rgb(1, 2, 3)", "rgba(0, 0, 0, .5)", "hsl(0 0% 0%)", "oklch(0.5 0.1 100)", "white", "Transparent", "currentColor"]
    {
        match ThemeToken::parse(value)
        {
            Err(TokenError::BareColor(_)) =>
            {}
            other => panic!("{value} 应判为裸色值，实际：{other:?}"),
        }
    }
}

/// 界面调色板与菜单令牌不属于桌面层，必须拒绝。
#[test]
fn palette_tokens_are_rejected_for_desktop_layer()
{
    for value in ["--color-surface", "--color-text", "--menu-surface", "--fence-unknown", "--desktop-accent"]
    {
        match ThemeToken::parse(value)
        {
            Err(TokenError::Unknown(_)) =>
            {}
            other => panic!("{value} 应判为不在白名单，实际：{other:?}"),
        }
    }
}

/// 令牌序列化后是原始名字，反序列化重新校验。
#[test]
fn token_survives_json_round_trip()
{
    let token = ThemeToken::parse("--fence-active").expect("合法令牌");
    let encoded = serde_json::to_string(&token).expect("序列化失败");
    assert_eq!(encoded, "\"--fence-active\"");
    let decoded: ThemeToken = serde_json::from_str(&encoded).expect("反序列化失败");
    assert_eq!(decoded, token);
}

/// 直接反序列化裸色值必须报错，绕不过白名单。
#[test]
fn deserializing_a_bare_color_fails()
{
    assert!(serde_json::from_str::<ThemeToken>("\"#123456\"").is_err());
    assert!(serde_json::from_str::<ThemeToken>("\"--fence-text\"").is_ok());
}
