use super::*;
use chrono::TimeZone;

/// RFC 8032 的公开测试种子，不是产品凭据。
fn credentials() -> QweatherCredentials { QweatherCredentials { host: "example.qweatherapi.com".into(), developer_id: "Q12345ABCD".into(), project_id: "ABCDE23456".into(), credential_id: "ABCDE12345".into(), private_key: "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIJ1hsZ3v/VpguoRK9JLsLMREScVpezJpGXA7rAMcrn9g\n-----END PRIVATE KEY-----".into() } }

/// 只构造内存城市，测试不调用供应商或持久保存位置。
fn city() -> City { City { id: "101011600".into(), name: "东城".into(), adm1: "北京".into(), adm2: "北京".into(), country: "中国".into(), lat: "39.91755".into(), lon: "116.41876".into(), tz: "Asia/Shanghai".into(), fx_link: "https://www.qweather.com/weather/dongcheng-101011600.html".into() } }

#[test]
fn host_and_private_key_are_checked_before_requests()
{
    let mut value = credentials(); // 测试草稿。
    for host in ["example.qweatherapi.com", "example.re.qweatherapi.com", "example.other.qweatherapi.com", "a.b.c.qweatherapi.com", "Example.OTHER.QWEATHERAPI.COM", "a-1.qweatherapi.com"]
    {
        value.host = host.into();
        assert!(check_qweather(&value).is_ok(), "{host}");
    }
    for host in ["", "https://example.qweatherapi.com", "qweatherapi.com", "fakeqweatherapi.com", "example.qweatherapi.com.evil.test", "evil.test@example.qweatherapi.com", "example.qweatherapi.com:443", "evil.example/path.qweatherapi.com", "evil.example\\path.qweatherapi.com", "evil.example?x.qweatherapi.com", "evil.example#x.qweatherapi.com", "example.qweatherapi.com/", ".qweatherapi.com", "a..qweatherapi.com", "a.qweatherapi.com.", "-a.qweatherapi.com", "a-.qweatherapi.com", "a_b.qweatherapi.com", "例.qweatherapi.com", "a%2eb.qweatherapi.com", " a.qweatherapi.com", "a.qweatherapi.com\n", "a\0.qweatherapi.com"]
    {
        value.host = host.into();
        assert!(check_qweather(&value).is_err(), "{host}");
    }
    value.host = format!("{}.{}.{}.{}.qweatherapi.com", "a".repeat(63), "b".repeat(63), "c".repeat(63), "d".repeat(45));
    assert_eq!(value.host.len(), 253);
    assert!(check_qweather(&value).is_ok());
    value.host = format!("{}.{}.{}.{}.qweatherapi.com", "a".repeat(63), "b".repeat(63), "c".repeat(63), "d".repeat(46));
    assert!(check_qweather(&value).is_err());
    value.host = format!("{}.qweatherapi.com", "a".repeat(64));
    assert!(check_qweather(&value).is_err());
    value = credentials();
    value.private_key = "not a private key".into();
    assert!(check_qweather(&value).is_err());
}

#[test]
fn credentials_are_classified_and_dispatched_by_source()
{
    assert!(check_credentials(&Credentials::Qweather(credentials())).is_ok());
    assert!(check_credentials(&Credentials::Foreca(ForecaCredentials { api_key: "example-api-key".into() })).is_ok());
    let issued = format!("{}.{}.{}", "a".repeat(36), "b".repeat(320), "c".repeat(43)); // 与 Foreca 实际签发的令牌长度一致。
    assert!(check_credentials(&Credentials::Foreca(ForecaCredentials { api_key: issued })).is_ok());
    let invisible = format!("token{}", '\u{200b}'); // 复制时混入的不可见字符。
    for key in ["", " token ", &"a".repeat(1025), invisible.as_str(), "Bearer token"]
    {
        assert!(check_credentials(&Credentials::Foreca(ForecaCredentials { api_key: key.into() })).is_err(), "{key}");
    }
    let foreca: Credentials = serde_json::from_str(r#"{"apiKey":"example-api-key"}"#).unwrap(); // 前端草稿按字段集合判定来源。
    assert!(matches!(foreca, Credentials::Foreca(_)));
    let qweather: Credentials = serde_json::from_str(r#"{"host":"example.qweatherapi.com","developerId":"Q12345ABCD","projectId":"ABCDE23456","credentialId":"ABCDE12345","privateKey":"draft"}"#).unwrap();
    assert!(matches!(qweather, Credentials::Qweather(_)));
    assert!(serde_json::from_str::<Credentials>(r#"{"host":"example.qweatherapi.com","apiKey":"example-api-key"}"#).is_err()); // 混合字段不得落入任一来源。
}

#[test]
fn jwt_has_required_claims_and_valid_ed25519_signature()
{
    use base64::Engine;
    let value = token(&credentials()).unwrap(); // 无真实凭据的签名测试。
    let public = base64::engine::general_purpose::STANDARD.decode("11qYAYKxCrfVS/7TyWQHOg7hcvPapiMlrwIaaPcHURo=").unwrap(); // RFC 8032 公钥。
    let mut validation = jsonwebtoken::Validation::new(Algorithm::EdDSA); // 校验签名和时限。
    validation.set_issuer(&["Q12345ABCD"]);
    validation.sub = Some("ABCDE23456".into());
    let decoded = jsonwebtoken::decode::<Value>(&value, &jsonwebtoken::DecodingKey::from_ed_der(&public), &validation).unwrap(); // 独立验签。
    assert_eq!(decoded.header.kid.as_deref(), Some("ABCDE12345"));
    assert!(decoded.header.typ.is_none());
    assert_eq!(decoded.claims["exp"].as_i64().unwrap() - decoded.claims["iat"].as_i64().unwrap(), 900);
}

#[test]
fn retry_after_supports_seconds_and_http_date()
{
    let mut headers = reqwest::header::HeaderMap::new(); // 模拟供应商响应头。
    headers.insert("retry-after", "5".parse().unwrap());
    assert!(retry_at(&headers).unwrap() >= Utc::now().timestamp() + 4);
    headers.insert("retry-after", "Wed, 21 Oct 2015 07:28:00 GMT".parse().unwrap());
    assert_eq!(retry_at(&headers), Some(1445412480));
}

#[test]
fn hourly_requires_continuous_current_window_and_celsius()
{
    let now = Utc.with_ymd_and_hms(2026, 1, 1, 10, 30, 0).unwrap(); // 固定时间边界。
    let mut value = json!({"hours":(0..24).map(|i|json!({"forecastTime":(now-chrono::Duration::minutes(30)+chrono::Duration::hours(i)).to_rfc3339(),"temperature":{"value":12,"unit":"°C"},"condition":{"text":"晴","code":"100"}})).collect::<Vec<_>>()}); // 连续样例。
    assert!(validate_weather("hourly", &value, &city(), now).is_ok());
    value["hours"][2]["forecastTime"] = value["hours"][1]["forecastTime"].clone();
    assert!(validate_weather("hourly", &value, &city(), now).is_err());
    assert!(validate_weather("current", &json!({"temperature":{"value":70,"unit":"°F"},"condition":{"text":"晴","code":"100"}}), &city(), now).is_err());
}

#[test]
fn current_weather_rejects_success_responses_without_valid_content()
{
    let now = Utc::now(); // 校验只使用内存响应，不访问真实账户。
    let valid = json!({"temperature":{"value":-5,"unit":"°C"},"condition":{"text":"晴","code":"100"}}); // 负温度仍是有效摄氏数据。
    assert!(validate_weather("current", &valid, &city(), now).is_ok());
    for value in [Value::Null, json!({"code":"200"}), json!({"temperature":{"value":"20","unit":"°C"},"condition":{"text":"晴","code":"100"}}), json!({"temperature":{"value":20,"unit":"°C"},"condition":{"text":"","code":"100"}})]
    {
        assert!(validate_weather("current", &value, &city(), now).is_err());
    }
}

#[test]
fn daily_uses_city_dates_through_dst_and_rejects_yesterday()
{
    let zone = chrono_tz::America::New_York; // 覆盖 23 小时的夏令时开始日。
    let now = Utc.with_ymd_and_hms(2026, 3, 8, 15, 0, 0).unwrap(); // 城市当天上午。
    let mut selected = city(); // 临时测试位置。
    selected.tz = zone.to_string();
    let value = json!({"days":(8..15).map(|day|json!({"forecastStartTime":zone.with_ymd_and_hms(2026,3,day,0,0,0).unwrap().to_rfc3339(),"forecastEndTime":zone.with_ymd_and_hms(2026,3,day+1,0,0,0).unwrap().to_rfc3339(),"temperatureMax":{"value":20,"unit":"°C"},"temperatureMin":{"value":10,"unit":"°C"},"daytime":{"condition":{"text":"晴","code":"100"}},"nighttime":{"condition":{"text":"晴","code":"150"}}})).collect::<Vec<_>>()}); // 按本地日历构造，不假设每天 24 小时。
    assert!(validate_weather("daily", &value, &selected, now).is_ok());
    assert!(validate_weather("daily", &value, &selected, now + chrono::Duration::days(1)).is_err());
    selected.fx_link = "https://www.qweather.com/weather/other-123.html".into();
    assert!(!valid_detail(&selected));
}
