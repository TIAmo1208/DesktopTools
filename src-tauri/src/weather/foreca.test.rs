use super::*;
use chrono::{Duration, TimeZone};

/// 只构造内存位置，测试不调用供应商或持久保存位置。
fn city() -> City { City { id: "100184745".into(), name: "北京".into(), adm1: "北京市".into(), adm2: "北京市".into(), country: "中国".into(), lat: "39.90".into(), lon: "116.41".into(), tz: "Asia/Shanghai".into(), fx_link: String::new() } }

#[test]
fn symbol_codes_map_onto_the_shared_precipitation_range()
{
    assert_eq!(symbol_code("d000").ok(), Some(0)); // 晴。
    assert_eq!(symbol_code("n100").ok(), Some(1)); // 夜间几乎无云。
    assert_eq!(symbol_code("d210").ok(), Some(310)); // 半云量下的阵雨。
    assert_eq!(symbol_code("d421").ok(), Some(321)); // 阴天雨夹雪阵雨。
    assert_eq!(symbol_code("d432").ok(), Some(332)); // 阴天降雪。
    for symbol in ["", "x000", "d00", "d12a", "d0000"]
    {
        assert!(symbol_code(symbol).is_err(), "{symbol}");
    }
}

#[test]
fn location_payload_becomes_the_shared_city_shape()
{
    let value = json!({"id": 100184745, "name": "北京", "country": "中国", "timezone": "Asia/Shanghai", "adminArea": "北京市", "adminArea2": "北京市", "lon": 116.41, "lat": 39.9});
    let parsed = location(&value).ok().unwrap();
    assert_eq!(parsed.id, "100184745");
    assert_eq!(parsed.tz, "Asia/Shanghai");
    assert_eq!(parsed.adm1, "北京市");
    assert_eq!(parsed.lat, "39.90");
    assert!(parsed.fx_link.is_empty()); // 该来源没有城市天气网页。
    assert!(location(&json!({"id": 1, "name": "北京", "timezone": "Mars/Base", "lon": 116.4, "lat": 39.9})).is_err()); // 时区必须是 IANA 名称。
    assert!(location(&json!({"id": 1, "name": "北京", "timezone": "Asia/Shanghai", "lon": 200.0, "lat": 39.9})).is_err()); // 坐标越界。
    assert!(location(&json!({"name": "北京", "timezone": "Asia/Shanghai", "lon": 116.4, "lat": 39.9})).is_err()); // 缺少位置标识。
}

#[test]
fn current_and_hourly_payloads_pass_the_shared_window_check()
{
    let now = Utc.with_ymd_and_hms(2026, 1, 1, 10, 30, 0).unwrap(); // 固定时间边界。
    let current_value = json!({"current": {"time": "2026-01-01T18:30+08:00", "symbol": "d200", "symbolPhrase": "多云", "temperature": 3.5}});
    let normalized = current(&current_value).ok().unwrap();
    assert_eq!(normalized["temperature"]["unit"], "°C"); // 归一化后单位仍是摄氏。
    assert_eq!(normalized["condition"]["code"], "2"); // 无降水时保留云量分档，落在降水区间之外。
    assert!(validate_weather("current", &normalized, &city(), now).is_ok());

    let hours = (0..24).map(|index| json!({"time": (now - Duration::minutes(30) + Duration::hours(index)).to_rfc3339(), "symbol": "d200", "symbolPhrase": "多云", "temperature": 1.0 + index as f64})).collect::<Vec<_>>();
    let normalized = hourly(&json!({"forecast": hours})).ok().unwrap();
    assert_eq!(normalized["hours"].as_array().unwrap().len(), 24);
    assert!(validate_weather("hourly", &normalized, &city(), now).is_ok());

    let empty = hourly(&json!({"forecast": []})).ok().unwrap(); // 空数组不在归一化阶段补数。
    assert!(validate_weather("hourly", &empty, &city(), now).is_err());
}

#[test]
fn daily_payload_covers_seven_local_dates_through_dst()
{
    let zone = chrono_tz::America::New_York; // 覆盖 23 小时的夏令时开始日。
    let now = Utc.with_ymd_and_hms(2026, 3, 8, 15, 0, 0).unwrap(); // 城市当天上午。
    let mut selected = city(); // 临时测试位置。
    selected.tz = zone.to_string();
    let days = (8..15).map(|day| json!({"date": format!("2026-03-{day:02}"), "symbol": "d100", "symbolPhrase": "晴", "maxTemp": 12.0, "minTemp": 2.0})).collect::<Vec<_>>();
    let normalized = daily(&json!({"forecast": days}), &selected).ok().unwrap();
    assert_eq!(normalized["days"].as_array().unwrap().len(), 7);
    assert!(validate_weather("daily", &normalized, &selected, now).is_ok()); // 含今天的完整七天。
    assert!(validate_weather("daily", &normalized, &selected, now + Duration::days(1)).is_err()); // 隔天不再含今天。
    assert!(daily(&json!({"forecast": [{"date": "2026-13-01", "symbol": "d100", "symbolPhrase": "晴", "maxTemp": 1.0, "minTemp": 0.0}]}), &selected).is_err()); // 日期无效。
}
