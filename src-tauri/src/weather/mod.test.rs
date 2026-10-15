use super::credentials::QweatherCredentials;
use super::*;

#[test]
fn module_retries_preserve_partial_failures_and_server_waits()
{
    let mut retry = ForecastRetry::default(); // 不访问网络，验证实际请求策略。
    let network = Failure { retryable: true, ..Failure::from("连接失败") }; // 文案不决定是否可恢复。
    for delay in [60, 120, 300, 900, 1800, 1800]
    {
        retry.record("city-a:hourly".into(), Some(&network), 100);
        assert_eq!(retry.modules["city-a:hourly"].retry_at, Some(100 + delay));
        retry.record("city-a:current".into(), None, 100);
        assert!(retry.modules.contains_key("city-a:hourly"));
    }
    let limited = Failure { retry_at: Some(100_000), retryable: true, ..Failure::from("限流") }; // 服务端等待可以超过 30 分钟。
    retry.record("city-b:hourly".into(), Some(&limited), 100);
    assert_eq!(retry.modules["city-b:hourly"].retry_at, Some(100_000));
    retry.record("city-a:daily".into(), Some(&Failure::from("权限不足")), 100);
    assert!(retry.modules["city-a:daily"].retry_at.is_none());
    retry.record("city-a:hourly".into(), None, 100);
    assert!(!retry.modules.contains_key("city-a:hourly"));
    assert!(retry.modules.contains_key("city-b:hourly"));
    retry.record("city-a:hourly".into(), Some(&network), 200);
    assert_eq!(retry.modules["city-a:hourly"].retry_at, Some(260));
}

#[test]
fn cache_identity_is_stable_and_covers_all_credential_fields()
{
    let original = Credentials::Qweather(QweatherCredentials { host: "test.qweatherapi.com".into(), developer_id: "developer".into(), project_id: "project".into(), credential_id: "credential".into(), private_key: "synthetic-test-key".into() }); // 仅测试指纹，不访问真实凭据或网络。
    let identity = credentials::cache_identity(&original).unwrap(); // 同一配置在重启后计算相同身份。
    assert_eq!(identity, credentials::cache_identity(&original.clone()).unwrap());
    assert_eq!(identity.len(), 43);
    for field in 0..5
    {
        let mut changed = original.clone(); // 每个配置字段独立改变都不得命中旧缓存。
        let Credentials::Qweather(value) = &mut changed
        else
        {
            unreachable!("测试草稿固定为和风凭据")
        };
        match field
        {
            0 => value.host.push('x'),
            1 => value.developer_id.push('x'),
            2 => value.project_id.push('x'),
            3 => value.credential_id.push('x'),
            _ => value.private_key.push('x'),
        }
        assert_ne!(identity, credentials::cache_identity(&changed).unwrap());
    }
}

#[test]
fn cancellation_releases_session_even_when_credential_status_fails()
{
    let cancelled = Arc::new(AtomicBool::new(false)); // 模拟在途请求持有的取消标记。
    let draft = Arc::new(Credentials::Qweather(QweatherCredentials { host: String::new(), developer_id: String::new(), project_id: String::new(), credential_id: String::new(), private_key: "test-draft".into() })); // 不访问真实账户。
    let released = Arc::downgrade(&draft); // 验证会话释放私钥所有权。
    let state = WeatherState {
        active: Mutex::new(Active { source: Source::Qweather, city_path: std::path::PathBuf::from("weather-test-city.json") }), // 只指向测试文件，不读写真实城市缓存。
        namespace: String::new(),                                                                                               // 空记录名使 Windows 凭据读取确定失败，不改变系统服务。
        directory: std::path::PathBuf::from("."),
        forecast_cache_path: std::path::PathBuf::from("weather-test-forecast.json"),
        forecast_guard: Mutex::new(()),
        forecast_retry: Mutex::new(ForecastRetry::default()),
        inner: Mutex::new(Inner { session: Some(Session { id: 2, credentials: draft, cancelled: Arc::clone(&cancelled), cities: Vec::new(), verified: true }), busy: true, ..Default::default() }),
    }; // 已验证但仍有在途请求的会话。
    assert!(state.execute(Command::Cancel { session_id: 1 }, 0).is_ok());
    assert!(!cancelled.load(Ordering::Acquire));
    assert!(released.upgrade().is_some());
    assert!(state.execute(Command::Cancel { session_id: 2 }, 0).is_ok());
    assert!(cancelled.load(Ordering::Acquire));
    assert!(released.upgrade().is_none());
    let inner = state.inner.lock().unwrap(); // 取消不提前释放网络互斥。
    assert!(inner.session.is_none());
    assert!(inner.busy);
}

#[test]
fn credential_store_and_authoritative_save_gate()
{
    let target = format!("DesktopTools/Test/Weather/{}", std::process::id()); // 仅写入本测试专用目标，不读取用户凭据。
    let state = WeatherState::for_test(std::path::Path::new("."), Source::Qweather, &target); // 记录名指向测试专用目标，不使用真实应用记录。
    let execute = |command| state.execute(command, state.window_epoch().unwrap()); // 默认使用当前窗口代次。
    credentials::clear(&target).unwrap();
    assert!(execute(Command::Status).unwrap().sources.is_empty());
    assert!(execute(Command::Save { session_id: 1 }).is_err());
    let draft = Credentials::Qweather(QweatherCredentials { host: "test.qweatherapi.com".into(), developer_id: "Q12345ABCD".into(), project_id: "test".into(), credential_id: "test".into(), private_key: "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIJ1hsZ3v/VpguoRK9JLsLMREScVpezJpGXA7rAMcrn9g\n-----END PRIVATE KEY-----".into() }); // 公开测试种子。
    let first = execute(Command::Begin { credentials: draft.clone() }).unwrap().session_id.unwrap(); // 第一份草稿。
    assert!(execute(Command::Save { session_id: first }).is_err());
    let second = execute(Command::Begin { credentials: draft.clone() }).unwrap().session_id.unwrap(); // 新草稿使旧会话失效。
    execute(Command::Cancel { session_id: first }).unwrap();
    assert_eq!(state.inner.lock().unwrap().session.as_ref().unwrap().id, second);
    credentials::save(&target, &draft).unwrap();
    assert_eq!(execute(Command::Status).unwrap().sources.len(), 1); // 已保存的测试记录出现在列表中。
    let mut oversized = draft.clone(); // 模拟不能写入的替换，旧记录必须保留。
    let Credentials::Qweather(value) = &mut oversized
    else
    {
        unreachable!("测试草稿固定为和风凭据")
    };
    value.private_key = "x".repeat(3000);
    assert!(credentials::save(&target, &oversized).is_err());
    assert!(credentials::configured(&target).unwrap());
    let previous = Arc::clone(&state.inner.lock().unwrap().configuration_cancelled); // 模拟旧配置在途请求持有的身份。
    state.forecast_retry.lock().unwrap().modules.insert("test:current".into(), ModuleRetry { retry_at: Some(i64::MAX), ..Default::default() });
    state.inner.lock().unwrap().session.as_mut().unwrap().verified = true;
    state.inner.lock().unwrap().session.as_mut().unwrap().credentials = Arc::new(oversized);
    assert!(execute(Command::Save { session_id: second }).is_err());
    assert!(!previous.load(Ordering::Acquire));
    assert_eq!(state.inner.lock().unwrap().configuration_revision, 0);
    assert_eq!(state.forecast_retry.lock().unwrap().modules["test:current"].retry_at, Some(i64::MAX));
    state.inner.lock().unwrap().session.as_mut().unwrap().credentials = Arc::new(draft.clone());
    execute(Command::Save { session_id: second }).unwrap();
    assert!(previous.load(Ordering::Acquire));
    assert_eq!(state.inner.lock().unwrap().configuration_revision, 1);
    assert!(state.forecast_retry.lock().unwrap().modules.is_empty());
    let saved = Arc::clone(&state.inner.lock().unwrap().configuration_cancelled); // 取消设置草稿不会取消正在使用的配置。
    let old_epoch = state.window_epoch().unwrap(); // 模拟窗口关闭前已进入队列的请求。
    state.cancel_all();
    assert!(state.execute(Command::Begin { credentials: draft }, old_epoch).is_err());
    assert!(execute(Command::Save { session_id: second }).is_err());
    execute(Command::Remove { source: Source::Qweather }).unwrap();
    assert!(saved.load(Ordering::Acquire));
    assert_eq!(state.inner.lock().unwrap().configuration_revision, 2);
    assert!(!credentials::configured(&target).unwrap());
}

#[test]
fn selected_city_round_trip_is_separate_from_credentials()
{
    let directory = std::env::temp_dir().join(format!("desktop-tools-weather-city-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let state = WeatherState::for_test(&directory, Source::Qweather, ""); // 空记录名不触碰真实凭据。
    let city = City { id: "101010100".into(), name: "北京".into(), adm1: "北京".into(), adm2: "北京".into(), country: "中国".into(), lat: "39.90".into(), lon: "116.40".into(), tz: "Asia/Shanghai".into(), fx_link: "https://www.qweather.com/weather/beijing-101010100.html".into() };
    assert!(state.saved_city().unwrap().is_none());
    assert_eq!(state.save_city(city.clone()).unwrap().id, city.id);
    assert_eq!(state.saved_city().unwrap().unwrap().name, "北京");
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn forecast_cache_is_scoped_by_city_and_expiry()
{
    let directory = std::env::temp_dir().join(format!("desktop-tools-weather-cache-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("weather-forecast.json"); // 与生产缓存文件名一致。
    let state = WeatherState::for_test(&directory, Source::Qweather, ""); // 空记录名不触碰真实凭据。
    let city = City { id: "101010100".into(), name: "北京".into(), adm1: "北京".into(), adm2: "北京".into(), country: "中国".into(), lat: "39.90".into(), lon: "116.40".into(), tz: "Asia/Shanghai".into(), fx_link: "https://www.qweather.com/weather/beijing-101010100.html".into() };
    let now = chrono::Utc::now().timestamp_millis(); // 构造两个城市、两个配置的独立缓存。
    let value = serde_json::json!({ "city": city, "sourceKey": "configuration-a", "moduleFetchedAt": { "current": now, "hourly": now, "daily": now }, "current": {}, "hourly": {}, "daily": {} });
    state.write_cached_forecast(&value).unwrap();
    assert!(state.read_cached_forecast(&city, Some("configuration-a")).unwrap().is_some());
    assert!(state.read_cached_forecast(&city, Some("configuration-b")).unwrap().is_none());
    let other = City { id: "101010300".into(), ..city.clone() };
    let mut other_value = value.clone(); // 更新另一位置不能覆盖第一位置。
    other_value["city"] = serde_json::to_value(&other).unwrap();
    state.write_cached_forecast(&other_value).unwrap();
    let mut replacement = value.clone(); // 同城更换配置也保留各自最近成功结果。
    replacement["sourceKey"] = serde_json::json!("configuration-b");
    replacement["current"] = serde_json::json!({"marker": "new"});
    state.write_cached_forecast(&replacement).unwrap();
    assert!(state.read_cached_forecast(&city, Some("configuration-a")).unwrap().is_some());
    assert!(state.read_cached_forecast(&other, Some("configuration-a")).unwrap().is_some());
    let latest = state.read_cached_forecast(&city, None).unwrap().unwrap(); // 清除凭据后的离线读取选择最近写入配置。
    assert_eq!(latest["current"]["marker"], "new");
    assert!(display_forecast(latest, now).get("sourceKey").is_none());
    state.write_cached_forecast(&replacement).unwrap();
    assert_eq!(state.cached_forecasts().unwrap().entries.len(), 3); // 重复刷新不积累历史版本。
    let mut expired = value; // 读取另一城市也清理过期模块。
    expired["moduleFetchedAt"]["current"] = serde_json::json!(now - 8 * 24 * 60 * 60 * 1000);
    state.write_cached_forecast(&expired).unwrap();
    assert!(state.read_cached_forecast(&other, Some("configuration-a")).unwrap().is_some());
    let disk = crate::persistence::load_json::<ForecastCache>(&path).unwrap().unwrap();
    assert!(disk.entries.iter().all(|value| value.get("city").is_none()));
    let retained = disk.entries.iter().find(|value| value["cityId"] == city.id && value["sourceKey"] == "configuration-a").unwrap();
    assert!(retained.get("current").is_none());
    assert!(retained["hourly"].is_object());
    assert!(retained["daily"].is_object());
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn source_listing_reports_availability_and_current_selection()
{
    let namespace = format!("DesktopTools/Test/Weather/list-{}", std::process::id()); // 测试专用记录，且与保存门槛测试互不干扰。
    let state = WeatherState::for_test(std::path::Path::new("."), Source::Qweather, &namespace);
    let listing = || {
        let inner = state.inner.lock().unwrap(); // 与命令路径相同的一次短事务。
        state.sources(&inner.availability)
    };
    let foreca = state.record(Source::Foreca); // 该来源的测试记录名。
    credentials::clear(&state.record(Source::Qweather)).unwrap();
    credentials::clear(&foreca).unwrap();
    assert!(listing().is_empty()); // 未配置的来源不出现在列表中。
    credentials::save(&foreca, &Credentials::Foreca(credentials::ForecaCredentials { api_key: "test-key".into() })).unwrap();
    let items = listing();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].address, "weatherapi.foreca.net"); // 该来源使用固定服务地址。
    assert_eq!(items[0].availability, Availability::Unconfirmed); // 已配置但尚未请求过。
    assert!(!items[0].enabled); // 当前启用来源仍是和风。
    state.observe(Source::Foreca, None);
    assert_eq!(listing()[0].availability, Availability::Available);
    state.observe(Source::Foreca, Some(Failure::from("网络不可用或请求超时，未完成验证")));
    assert_eq!(listing()[0].availability, Availability::Unavailable);
    assert!(state.set_source(Source::Foreca)); // 切换来源报告实际变化。
    assert!(listing()[0].enabled);
    assert!(!state.set_source(Source::Foreca)); // 相同来源不重复处理。
    credentials::clear(&foreca).unwrap();
}

#[test]
fn forecast_merge_keeps_cached_modules_and_clears_successful_errors()
{
    let city = City { id: "101010100".into(), name: "北京".into(), adm1: "北京".into(), adm2: "北京".into(), country: "中国".into(), lat: "39.90".into(), lon: "116.40".into(), tz: "Asia/Shanghai".into(), fx_link: "https://www.qweather.com/weather/beijing-101010100.html".into() };
    let cached = serde_json::json!({ "city": city, "current": { "temperature": { "value": 1 } }, "hourly": { "hours": [1] }, "daily": { "days": [2] }, "moduleFetchedAt": { "current": 1, "hourly": 2, "daily": 2 }, "attributions": ["和风"], "moduleErrors": { "current": "旧错误", "hourly": "保留错误", "daily": "旧错误" } }); // 包含未请求模块和失败模块的最近成功结果。
    let fresh = serde_json::json!({ "current": { "temperature": { "value": 2 } }, "hourly": null, "daily": null, "moduleFetchedAt": { "current": 3 }, "moduleErrors": { "daily": "新错误" }, "attributions": [], "fetchedAt": 3 }); // 与 Provider 的部分成功响应保持相同结构。
    let merged = merge_forecast(Some(cached), fresh, &city);
    assert_eq!(merged["current"]["temperature"]["value"], 2);
    assert_eq!(merged["hourly"]["hours"][0], 1);
    assert_eq!(merged["daily"]["days"][0], 2);
    assert_eq!(merged["moduleFetchedAt"], serde_json::json!({ "current": 3, "hourly": 2, "daily": 2 }));
    assert_eq!(merged["attributions"], serde_json::json!(["和风"]));
    assert!(merged["moduleErrors"].get("current").is_none());
    assert_eq!(merged["moduleErrors"]["hourly"], "保留错误");
    assert_eq!(merged["moduleErrors"]["daily"], "新错误");
    assert_eq!(merged["fetchedAt"], 2);
}

#[test]
fn fresh_timestamp_does_not_hide_incomplete_forecast_window()
{
    let now = chrono::Utc::now(); // 使用同一基准构造最近获取但内容窗口失效的缓存。
    let city = City { id: "101010100".into(), name: "北京".into(), adm1: "北京".into(), adm2: "北京".into(), country: "中国".into(), lat: "39.90".into(), lon: "116.40".into(), tz: "Asia/Shanghai".into(), fx_link: "https://www.qweather.com/weather/beijing-101010100.html".into() };
    let mut cached = serde_json::json!({ "city": city, "moduleFetchedAt": { "current": now.timestamp_millis(), "hourly": now.timestamp_millis(), "daily": now.timestamp_millis() }, "current": {"temperature":{"value":20,"unit":"°C"},"condition":{"text":"晴","code":"100"}}, "hourly":{"hours":(0..24).map(|i|serde_json::json!({"forecastTime":(now+chrono::Duration::hours(i)).to_rfc3339(),"temperature":{"value":12,"unit":"°C"},"condition":{"text":"晴","code":"100"}})).collect::<Vec<_>>()}}); // 真实模块结构，不访问账户。
    assert!(module_fresh(&cached, "current"));
    assert!(module_fresh(&cached, "hourly"));
    cached["hourly"]["hours"].as_array_mut().unwrap().pop();
    assert!(!module_fresh(&cached, "hourly"));
    let today = now.with_timezone(&chrono_tz::Asia::Shanghai).date_naive(); // 使用城市日期构造昨天起始的七天。
    cached["daily"] = serde_json::json!({"days":(-1..6).map(|i| { let start = today + chrono::Duration::days(i); serde_json::json!({"forecastStartTime":format!("{start}T00:00:00+08:00"),"forecastEndTime":format!("{}T00:00:00+08:00",start+chrono::Duration::days(1)),"temperatureMax":{"value":20,"unit":"°C"},"temperatureMin":{"value":10,"unit":"°C"},"daytime":{"condition":{"text":"晴","code":"100"}},"nighttime":{"condition":{"text":"晴","code":"150"}}}) }).collect::<Vec<_>>()});
    assert!(!module_fresh(&cached, "daily"));
    assert!(module_fresh(&cached, "current"));
}

#[test]
fn missing_or_future_module_timestamp_cannot_borrow_global_freshness()
{
    let now = chrono::Utc::now().timestamp_millis(); // 固定本次测试基准。
    let mut value = serde_json::json!({ "current": {}, "hourly": null, "fetchedAt": now, "moduleFetchedAt": { "hourly": now, "daily": now } }); // 时间戳和模块必须成对存在。
    assert!(!module_fresh(&value, "current"));
    assert!(!module_fresh(&value, "hourly"));
    assert!(!module_fresh(&value, "daily"));
    value["moduleFetchedAt"]["current"] = serde_json::json!(now + 60_000);
    assert!(!module_fresh(&value, "current"));
}

#[test]
fn retention_prunes_at_seven_days_without_touching_younger_modules()
{
    let now = 2_000_000_000_000_i64; // 固定时间使七天边界可重复。
    let week = 7 * 24 * 60 * 60 * 1000; // 毫秒保留期限。
    let mut value = serde_json::json!({ "current": {}, "hourly": {}, "daily": {}, "moduleFetchedAt": { "current": now - week, "hourly": now - week + 1, "daily": now + 1 }, "moduleErrors": { "current": "旧错误" }, "attributions": ["和风"] }); // 边界、边界内和未来时间。
    assert!(prune_forecast(&mut value, now));
    assert!(value.get("current").is_none());
    assert!(value.get("daily").is_none());
    assert!(value["hourly"].is_object());
    assert_eq!(value["moduleErrors"]["current"], "旧错误");
    assert_eq!(value["fetchedAt"], now - week + 1);
    assert!(!prune_forecast(&mut value, now));
    assert!(prune_forecast(&mut value, now + 1));
    assert!(value.get("hourly").is_none());
    assert!(value.get("attributions").is_none());
    assert!(value["fetchedAt"].is_null());
}

#[test]
fn current_display_expires_at_24_hours_without_discarding_disk_or_other_modules()
{
    let now = 2_000_000_000_000_i64; // 固定展示时间边界。
    let mut cached = serde_json::json!({ "current": { "temperature": { "value": 20 } }, "hourly": { "hours": [1] }, "daily": { "days": [2] }, "moduleFetchedAt": { "current": now - 24 * 60 * 60 * 1000 + 1 } }); // 距离展示期限还剩一毫秒。
    assert!(display_forecast(cached.clone(), now)["current"].is_object());
    let expired = display_forecast(cached.clone(), now + 1); // 到期后只过滤返回副本。
    assert!(expired.get("current").is_none());
    assert!(expired["moduleErrors"]["current"].is_string());
    assert_eq!(expired["hourly"], cached["hourly"]);
    assert_eq!(expired["daily"], cached["daily"]);
    assert!(cached["current"].is_object());
    cached["moduleErrors"] = serde_json::json!({ "current": "网络失败" });
    assert_eq!(display_forecast(cached.clone(), now + 1)["moduleErrors"]["current"], "网络失败");
    cached["moduleFetchedAt"] = serde_json::json!({});
    assert!(display_forecast(cached.clone(), now).get("current").is_none());
    cached["moduleFetchedAt"]["current"] = serde_json::json!(now + 1);
    assert!(display_forecast(cached, now).get("current").is_none());
}
