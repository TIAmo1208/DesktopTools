use super::credentials::QweatherCredentials;
use super::*;

/// 测试专用目录：每个用例独立，不与其他用例共用城市列表文件。
fn test_directory(name: &str) -> std::path::PathBuf
{
    let directory = std::env::temp_dir().join(format!("desktop-tools-weather-{}-{}", name, std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    directory
}

/// Windows 凭据存储是进程外共享资源：并行读写会让彼此读到瞬时错误，涉凭据的用例统一排队执行。
static CREDENTIAL_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 取得凭据访问令牌，持到用例结束。
fn serialize_credentials() -> std::sync::MutexGuard<'static, ()> { CREDENTIAL_LOCK.lock().unwrap_or_else(|error| error.into_inner()) }

/// 和风城市样例；详情链接与标识匹配，可复用现有校验规则。
fn qweather_city(id: &str) -> City { City { id: id.into(), name: "北京".into(), adm1: "北京".into(), adm2: "北京".into(), country: "中国".into(), lat: "39.90".into(), lon: "116.40".into(), tz: "Asia/Shanghai".into(), fx_link: format!("https://www.qweather.com/weather/beijing-{id}.html") } }

#[test]
fn probe_does_not_reenter_the_state_lock()
{
    let _credentials = serialize_credentials(); // 与其它涉凭据用例串行。
    // 探测曾在本函数已持有状态锁时调用同样加锁的 configuration()，同一线程二次加锁直接死锁：
    // 探测线程永久阻塞，之后任何读取状态锁的命令都把调用线程挂住（含同步命令 list_weather_sources），整个程序失去响应。
    // 两个来源共用同一条探测分支，因此两条都要跑：只覆盖和风时，Foreca 侧的同名缺陷不会被发现。
    for source in Source::ALL
    {
        let target = format!("DesktopTools/Test/Weather/probe-{}-{pid}", source.key(), pid = std::process::id()); // 仅写入本测试专用目标。
        let directory = test_directory(&format!("probe-lock-{}", source.key()));
        let state = WeatherState::for_test(&directory, &target);
        let draft = match source
        {
            Source::Qweather => Credentials::Qweather(QweatherCredentials { host: "invalid host".into(), developer_id: "Q12345ABCD".into(), project_id: "test".into(), credential_id: "test".into(), private_key: "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIJ1hsZ3v/VpguoRK9JLsLMREScVpezJpGXA7rAMcrn9g\n-----END PRIVATE KEY-----".into() }),
            Source::Foreca => Credentials::Foreca(credentials::ForecaCredentials { api_key: "test key with spaces".into() }), // 非法密钥使请求在建立连接前失败，不访问网络。
        };
        credentials::save(&target, &draft).unwrap();
        let epoch = state.window_epoch().unwrap();
        let (sender, receiver) = std::sync::mpsc::channel(); // 用例自己承担超时，死锁表现为超时失败而不是挂住整个测试进程。
        std::thread::spawn(move || {
            let outcome = state.execute(Command::Probe { source }, epoch).map(|_| ()).map_err(|message| message);
            let _ = sender.send(outcome);
        });
        let outcome = receiver.recv_timeout(std::time::Duration::from_secs(30)).unwrap_or_else(|_| panic!("{} 探测命令在状态锁上死锁", source.key()));
        credentials::clear(&target).unwrap();
        let _ = std::fs::remove_dir_all(&directory);
        assert!(outcome.is_err(), "{} 的非法配置必须让探测返回失败：{outcome:?}", source.key());
    }
}

#[test]
fn credential_record_names_are_derived_in_one_place()
{
    // 读写两侧各写一遍取名规则时，任何一侧改名都会出现“写 A 读 B”：已配置却显示未配置，且旧记录残留在凭据管理器里。
    for source in Source::ALL
    {
        assert_eq!(credentials::record_name("DesktopTools/Weather", source), credentials::target(source));
        assert_eq!(WeatherState::for_test(std::path::Path::new("."), &credentials::target(Source::Qweather)).record(source), credentials::target(source));
    }
}

/// Foreca 城市样例；该来源没有城市天气网页，详情链接固定为空。
fn foreca_city(id: &str, name: &str) -> City { City { id: id.into(), name: name.into(), adm1: "北京市".into(), adm2: "北京市".into(), country: "中国".into(), lat: "39.90".into(), lon: "116.41".into(), tz: "Asia/Shanghai".into(), fx_link: String::new() } }

#[test]
fn module_retries_preserve_partial_failures_and_server_waits()
{
    let mut retry = ForecastRetry::default(); // 不访问网络，验证实际请求策略。
    let network = Failure { retryable: true, ..Failure::from("连接失败") }; // 文案不决定是否可恢复。
    for delay in [60, 120, 300, 900, 1800, 1800]
    {
        retry.record(retry_key(Source::Qweather, "city-a", "hourly"), Some(&network), 100);
        assert_eq!(retry.modules[&retry_key(Source::Qweather, "city-a", "hourly")].retry_at, Some(100 + delay));
        retry.record(retry_key(Source::Qweather, "city-a", "current"), None, 100);
        assert!(retry.modules.contains_key(&retry_key(Source::Qweather, "city-a", "hourly")));
    }
    let limited = Failure { retry_at: Some(100_000), retryable: true, ..Failure::from("限流") }; // 服务端等待可以超过 30 分钟。
    retry.record(retry_key(Source::Foreca, "city-b", "hourly"), Some(&limited), 100);
    assert_eq!(retry.modules[&retry_key(Source::Foreca, "city-b", "hourly")].retry_at, Some(100_000));
    retry.record(retry_key(Source::Qweather, "city-a", "daily"), Some(&Failure::from("权限不足")), 100);
    assert!(retry.modules[&retry_key(Source::Qweather, "city-a", "daily")].retry_at.is_none());
    retry.record(retry_key(Source::Qweather, "city-a", "hourly"), None, 100);
    assert!(!retry.modules.contains_key(&retry_key(Source::Qweather, "city-a", "hourly")));
    assert!(retry.modules.contains_key(&retry_key(Source::Foreca, "city-b", "hourly")));
    retry.record(retry_key(Source::Qweather, "city-a", "hourly"), Some(&network), 200);
    assert_eq!(retry.modules[&retry_key(Source::Qweather, "city-a", "hourly")].retry_at, Some(260));
}

#[test]
fn same_city_id_from_two_sources_keeps_independent_retry()
{
    let mut retry = ForecastRetry::default(); // 同名城市跨来源必须各归各家。
    let network = Failure { retryable: true, ..Failure::from("连接失败") };
    retry.record(retry_key(Source::Qweather, "101010100", "current"), Some(&network), 100);
    retry.record(retry_key(Source::Foreca, "101010100", "current"), None, 100);
    assert!(retry.modules.contains_key(&retry_key(Source::Qweather, "101010100", "current")));
    assert!(!retry.modules.contains_key(&retry_key(Source::Foreca, "101010100", "current"))); // 另一来源的失败不被同名城市清除。
}

#[test]
fn blank_secret_field_keeps_the_saved_value_and_never_overwrites_a_new_one()
{
    let saved = Credentials::Qweather(QweatherCredentials { host: "test.qweatherapi.com".into(), developer_id: "Q12345ABCD".into(), project_id: "test".into(), credential_id: "test".into(), private_key: "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIJ1hsZ3v/VpguoRK9JLsLMREScVpezJpGXA7rAMcrn9g\n-----END PRIVATE KEY-----".into() }); // 公开测试种子，不访问真实凭据。
    let mut edited = match saved.clone()
    {
        Credentials::Qweather(value) => value,
        Credentials::Foreca(_) => unreachable!("测试凭据固定为和风"),
    };
    edited.private_key = String::new(); // 编辑态未重新输入私钥。
    edited.developer_id = "Q999999999".into(); // 只改了非敏感字段。
    let merged = match merge_saved_secrets(Credentials::Qweather(edited.clone()), Some(saved.clone()))
    {
        Credentials::Qweather(value) => value,
        Credentials::Foreca(_) => unreachable!("合并不改变来源"),
    };
    assert!(merged.private_key.contains("MC4CAQAwBQYDK2Vw")); // 留空表示沿用已保存私钥。
    assert_eq!(merged.developer_id, "Q999999999"); // 非敏感字段按草稿写入。
    assert!(provider::check_credentials(&Credentials::Qweather(merged)).is_ok()); // 合并后通过格式校验，可直接保存。

    edited.private_key = "-----BEGIN PRIVATE KEY-----\nreplaced\n-----END PRIVATE KEY-----".into(); // 用户重新输入私钥。
    let replaced = match merge_saved_secrets(Credentials::Qweather(edited), Some(saved))
    {
        Credentials::Qweather(value) => value,
        Credentials::Foreca(_) => unreachable!("合并不改变来源"),
    };
    assert!(replaced.private_key.contains("replaced")); // 不覆盖用户输入。
    assert!(merge_saved_secrets(Credentials::Foreca(credentials::ForecaCredentials { api_key: String::new() }), None).source() == Source::Foreca); // 没有已保存记录时保留空值，交由格式校验拒绝。
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
    let _credentials = serialize_credentials(); // 与其它涉凭据用例串行。
    let cancelled = Arc::new(AtomicBool::new(false)); // 模拟在途请求持有的取消标记。
    let draft = Arc::new(Credentials::Qweather(QweatherCredentials { host: String::new(), developer_id: String::new(), project_id: String::new(), credential_id: String::new(), private_key: "test-draft".into() })); // 不访问真实账户。
    let released = Arc::downgrade(&draft); // 验证会话释放私钥所有权。
    let state = WeatherState::for_test(std::path::Path::new("."), ""); // 空记录名使 Windows 凭据读取确定失败，不改变系统服务。
    state.inner.lock().unwrap().session = Some(Session { id: 2, credentials: draft, cancelled: Arc::clone(&cancelled), cities: Vec::new() }); // 仍有在途请求的会话。
    state.inner.lock().unwrap().busy = true;
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
    let _credentials = serialize_credentials(); // 与其它涉凭据用例串行。
    let target = format!("DesktopTools/Test/Weather/{}", std::process::id()); // 仅写入本测试专用目标，不读取用户凭据。
    let state = WeatherState::for_test(std::path::Path::new("."), &target); // 记录名指向测试专用目标，不使用真实应用记录。
    let execute = |command| state.execute(command, state.window_epoch().unwrap()); // 默认使用当前窗口代次。
    credentials::clear(&target).unwrap();
    assert!(execute(Command::Status).unwrap().sources.is_empty());
    assert!(execute(Command::Save { session_id: 1 }).is_err());
    let draft = Credentials::Qweather(QweatherCredentials { host: "test.qweatherapi.com".into(), developer_id: "Q12345ABCD".into(), project_id: "test".into(), credential_id: "test".into(), private_key: "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIJ1hsZ3v/VpguoRK9JLsLMREScVpezJpGXA7rAMcrn9g\n-----END PRIVATE KEY-----".into() }); // 公开测试种子。
    let first = execute(Command::Begin { credentials: draft.clone() }).unwrap().session_id.unwrap(); // 第一份草稿。
    let second = execute(Command::Begin { credentials: draft.clone() }).unwrap().session_id.unwrap(); // 新草稿使旧会话失效。
    assert!(execute(Command::Save { session_id: first }).is_err()); // 过期身份不得写入凭据。
    assert_eq!(state.inner.lock().unwrap().session.as_ref().unwrap().id, second);
    execute(Command::Save { session_id: second }).unwrap(); // 保存只要求草稿身份有效，不再要求先完成能力验证。
    assert_eq!(execute(Command::Status).unwrap().sources.len(), 1); // 未验证的草稿同样写入该来源的凭据记录。
    let mut oversized = draft.clone(); // 模拟不能写入的替换，旧记录必须保留。
    let Credentials::Qweather(value) = &mut oversized
    else
    {
        unreachable!("测试草稿固定为和风凭据")
    };
    value.private_key = "x".repeat(3000);
    assert!(credentials::save(&target, &oversized).is_err());
    assert!(credentials::configured(&target).unwrap());
    let previous = state.configuration(Source::Qweather).cancelled; // 模拟旧配置在途请求持有的身份。
    state.forecast_retry.lock().unwrap().modules.insert("test:current".into(), ModuleRetry { retry_at: Some(i64::MAX), ..Default::default() });
    let third = execute(Command::Begin { credentials: draft.clone() }).unwrap().session_id.unwrap(); // 保存会消费草稿，替换写入使用新草稿。
    state.inner.lock().unwrap().session.as_mut().unwrap().credentials = Arc::new(oversized);
    assert!(execute(Command::Save { session_id: third }).is_err());
    assert!(!previous.load(Ordering::Acquire));
    assert_eq!(state.config_revision(Source::Qweather), 1); // 失败的替换不产生新的配置代次。
    assert_eq!(state.forecast_retry.lock().unwrap().modules["test:current"].retry_at, Some(i64::MAX));
    state.inner.lock().unwrap().session.as_mut().unwrap().credentials = Arc::new(draft.clone());
    execute(Command::Save { session_id: third }).unwrap();
    assert!(previous.load(Ordering::Acquire));
    assert_eq!(state.config_revision(Source::Qweather), 2);
    let saved = state.configuration(Source::Qweather).cancelled; // 取消设置草稿不会取消正在使用的配置。
    let old_epoch = state.window_epoch().unwrap(); // 模拟窗口关闭前已进入队列的请求。
    state.cancel_all();
    assert!(state.execute(Command::Begin { credentials: draft }, old_epoch).is_err());
    assert!(execute(Command::Save { session_id: second }).is_err());
    execute(Command::Remove { source: Source::Qweather }).unwrap();
    assert!(saved.load(Ordering::Acquire));
    assert_eq!(state.config_revision(Source::Qweather), 3); // 删除凭据同样作废该来源的旧配置身份。
    assert!(!credentials::configured(&target).unwrap());
}

#[test]
fn city_lists_are_separate_files_deduplicated_by_id()
{
    let _credentials = serialize_credentials(); // 与其它涉凭据用例串行。
    let directory = test_directory("city-list");
    let state = WeatherState::for_test(&directory, ""); // 空记录名不触碰真实凭据。
    assert!(state.saved_city(Source::Qweather).unwrap().is_none()); // 尚未添加城市。
    assert!(state.saved_city(Source::Foreca).unwrap().is_none());
    assert_eq!(state.save_city(Source::Qweather, qweather_city("101010100")).unwrap().id, "101010100");
    let other = qweather_city("101010300"); // 同一来源的另一个城市。
    assert_eq!(state.save_city(Source::Qweather, other.clone()).unwrap().id, other.id);
    assert_eq!(state.save_city(Source::Qweather, qweather_city("101010100")).unwrap().name, "北京"); // 重复添加同一标识不产生重复项。
    let qweather = state.cities(Source::Qweather).unwrap().iter().map(|city| city.id.clone()).collect::<Vec<_>>();
    assert_eq!(qweather.len(), 2);
    assert!(qweather.contains(&"101010100".to_string()) && qweather.contains(&other.id));
    assert!(qweather.contains(&state.saved_city(Source::Qweather).unwrap().unwrap().id)); // 取出的城市来自该来源列表。
    assert!(std::fs::read_to_string(Source::Qweather.city_path(&directory)).unwrap().starts_with('[')); // 文件只保存城市对象数组。
    assert_eq!(state.cities(Source::Foreca).unwrap().len(), 0); // 另一来源不受影响。
    let foreca = foreca_city("100184745", "北京");
    state.save_city(Source::Foreca, foreca.clone()).unwrap();
    assert_eq!(state.cities(Source::Foreca).unwrap()[0].id, foreca.id);
    assert_ne!(Source::Qweather.city_path(&directory), Source::Foreca.city_path(&directory)); // 两个来源各自成文件。
    assert!(Source::Foreca.city_path(&directory).exists());
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn foreca_city_saves_without_detail_link_and_keeps_forecast()
{
    let directory = test_directory("foreca-city");
    let state = WeatherState::for_test(&directory, ""); // 空记录名不触碰真实凭据。
    let city = foreca_city("100184745", "北京");
    assert!(state.save_city(Source::Foreca, city.clone()).is_ok()); // 空详情链接不再被判为城市数据无效。
    assert_eq!(state.saved_city(Source::Foreca).unwrap().unwrap().fx_link, "");
    let now = chrono::Utc::now().timestamp_millis(); // 构造该城市已落盘的预报缓存。
    state.write_cached_forecast(&serde_json::json!({ "city": city, "sourceKey": "foreca-configuration", "moduleFetchedAt": { "current": now, "hourly": now, "daily": now }, "current": { "marker": "foreca" }, "hourly": {}, "daily": {} })).unwrap();
    let forecast = state.forecast(Source::Foreca, city.clone()).unwrap(); // 真实入口读取该来源缓存。
    assert_eq!(forecast["current"]["marker"], "foreca");
    assert_eq!(forecast["city"]["id"], city.id);
    assert!(state.save_city(Source::Qweather, city.clone()).is_err()); // 和风仍要求匹配标识的详情链接。
    assert!(state.save_city(Source::Qweather, qweather_city("101010100")).is_ok());
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn forecast_cache_is_scoped_by_city_and_expiry()
{
    let directory = test_directory("forecast-cache");
    let state = WeatherState::for_test(&directory, ""); // 空记录名不触碰真实凭据。
    let path = directory.join("weather-forecast.json"); // 与生产缓存文件名一致。
    let city = qweather_city("101010100");
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
fn city_removal_keeps_other_source_retry_and_configuration_generation()
{
    let _credentials = serialize_credentials(); // 与其它涉凭据用例串行。
    let namespace = format!("DesktopTools/Test/Weather/removal-{}", std::process::id()); // 两个来源的记录名都由该测试命名空间派生，不读写用户凭据。
    let directory = test_directory("removal");
    let state = WeatherState::for_test(&directory, &namespace);
    let execute = |command| state.execute(command, state.window_epoch().unwrap());
    let qweather_record = state.record(Source::Qweather); // 和风记录名沿用测试命名空间。
    let foreca_record = state.record(Source::Foreca);
    credentials::clear(&qweather_record).unwrap();
    credentials::clear(&foreca_record).unwrap();
    state.save_city(Source::Qweather, qweather_city("101010100")).unwrap();
    state.save_city(Source::Foreca, foreca_city("101010100", "北京")).unwrap(); // 两个来源使用同名城市标识。
    let qweather_forecast = Failure { retryable: true, ..Failure::from("连接失败") };
    let foreca_forecast = Failure { retryable: true, ..Failure::from("连接失败") };
    state.forecast_retry.lock().unwrap().record(retry_key(Source::Qweather, "101010100", "current"), Some(&qweather_forecast), 100);
    state.forecast_retry.lock().unwrap().record(retry_key(Source::Foreca, "101010100", "current"), Some(&foreca_forecast), 100);
    state.configuration(Source::Foreca); // 使另一来源也拥有配置代次。
    execute(Command::Remove { source: Source::Qweather }).unwrap(); // 删除和风城市所属来源的凭据。
    let retry = state.forecast_retry.lock().unwrap();
    assert!(!retry.modules.contains_key(&retry_key(Source::Qweather, "101010100", "current"))); // 被删除来源的退避已清空。
    assert!(retry.modules.contains_key(&retry_key(Source::Foreca, "101010100", "current"))); // 另一来源的同名城市保留退避。
    drop(retry);
    assert_eq!(state.config_revision(Source::Qweather), 1);
    assert_eq!(state.config_revision(Source::Foreca), 0); // 另一来源的配置代次不受影响。
    assert!(state.cities(Source::Foreca).unwrap().iter().any(|city| city.id == "101010100")); // 删除来源不影响另一来源的城市列表。
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn source_listing_reports_availability_and_cities_without_current_selection()
{
    let _credentials = serialize_credentials(); // 与其它涉凭据用例串行。
    let directory = test_directory("source-listing");
    let namespace = format!("DesktopTools/Test/Weather/list-{}", std::process::id()); // 两个来源的记录名都由该测试命名空间派生，与其他用例互不干扰。
    let state = WeatherState::for_test(&directory, &namespace);
    let listing = || {
        let inner = state.inner.lock().unwrap(); // 与命令路径相同的一次短事务。
        state.sources(&inner.availability).unwrap()
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
    assert!(items[0].cities.is_empty()); // 尚未添加城市时列表为空。
    state.save_city(Source::Foreca, foreca_city("100184745", "北京")).unwrap();
    assert_eq!(listing()[0].cities.iter().map(|city| city.id.as_str()).collect::<Vec<_>>(), ["100184745"]); // 来源状态携带该来源的城市列表。
    state.observe(Source::Foreca, None);
    assert_eq!(listing()[0].availability, Availability::Available);
    state.observe(Source::Foreca, Some(Failure::from("网络不可用或请求超时，未完成验证")));
    assert_eq!(listing()[0].availability, Availability::Unavailable);
    credentials::clear(&foreca).unwrap();
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn session_city_command_writes_verified_city_into_the_source_list()
{
    let _credentials = serialize_credentials(); // 与其它涉凭据用例串行。
    let directory = test_directory("session-city");
    let state = WeatherState::for_test(&directory, ""); // 空记录名不触碰真实凭据。
    let execute = |command| state.execute(command, state.window_epoch().unwrap()); // 默认使用当前窗口代次。
    assert!(state.cities(Source::Qweather).unwrap().is_empty());
    assert!(execute(Command::SaveCity { session_id: 1, city_id: "101010100".into() }).is_err()); // 没有草稿时拒绝写入。
    let draft = Credentials::Qweather(QweatherCredentials { host: "test.qweatherapi.com".into(), developer_id: "Q12345ABCD".into(), project_id: "test".into(), credential_id: "test".into(), private_key: "draft-key".into() }); // 不访问网络的草稿凭据。
    let session = state.inner.lock().unwrap().revision + 1; // 与真实草稿相同的下一个会话身份。
    state.inner.lock().unwrap().revision = session;
    state.inner.lock().unwrap().session = Some(Session { id: session, credentials: Arc::new(draft), cancelled: Arc::new(AtomicBool::new(false)), cities: vec![qweather_city("101010100")] }); // 搜索命中后的会话状态。
    assert!(execute(Command::SaveCity { session_id: session, city_id: "101010199".into() }).is_err()); // 不接受本次搜索之外的城市。
    let snapshot = execute(Command::SaveCity { session_id: session, city_id: "101010100".into() }).unwrap();
    assert_eq!(snapshot.cities.iter().map(|city| city.id.as_str()).collect::<Vec<_>>(), ["101010100"]); // 命令返回本次写入的城市。
    assert_eq!(state.cities(Source::Qweather).unwrap()[0].name, "北京"); // 城市写入该草稿来源的列表。
    assert!(state.cities(Source::Foreca).unwrap().is_empty()); // 其他来源不受影响。
    assert_eq!(state.validated_city(Source::Qweather, "101010100").unwrap().id, "101010100"); // 写入后按来源可以解析。
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn city_removal_returns_the_updated_list_and_keeps_other_sources()
{
    let _credentials = serialize_credentials(); // 与其它涉凭据用例串行。
    let directory = test_directory("city-removal");
    let state = WeatherState::for_test(&directory, ""); // 空记录名不触碰真实凭据。
    state.save_city(Source::Qweather, qweather_city("101010100")).unwrap();
    state.save_city(Source::Qweather, qweather_city("101010300")).unwrap(); // 删除只作用于提交的标识。
    state.save_city(Source::Foreca, foreca_city("101010100", "北京")).unwrap(); // 两个来源使用同名城市标识。
    let remaining = state.remove_city(Source::Qweather, "101010100").unwrap();
    assert_eq!(remaining.iter().map(|city| city.id.as_str()).collect::<Vec<_>>(), ["101010300"]); // 返回更新后的完整列表，添加顺序不变。
    assert_eq!(state.cities(Source::Qweather).unwrap().len(), 1); // 删除已落盘。
    assert!(state.validated_city(Source::Qweather, "101010100").is_err()); // 已移除的标识无法再解析。
    assert_eq!(state.cities(Source::Foreca).unwrap().len(), 1); // 另一来源的同名城市不受影响。
    assert!(state.remove_city(Source::Qweather, "101010100").is_err()); // 标识不存在时明确失败。
    assert_eq!(state.cities(Source::Qweather).unwrap().len(), 1); // 失败不改变原列表。
    let _ = std::fs::remove_dir_all(directory);
}

/// 跨来源限流门控只拦截被服务端要求等待的来源，已过期的时间戳不再拦截。
#[test]
fn retry_gate_is_scoped_to_the_limited_source()
{
    let future = chrono::Utc::now().timestamp() + 60; // 尚未到达的服务端重试时间。
    let foreca_only = std::collections::BTreeMap::from([(Source::Foreca, future)]); // 仅 Foreca 被限流。
    let error = retry_gate(&foreca_only, Source::Foreca).expect_err("被限流的来源必须被拦截");
    assert!(error.contains("foreca"), "错误文案必须指明被限流的来源：{error}");
    assert!(retry_gate(&foreca_only, Source::Qweather).is_ok(), "另一个来源不受限流影响");
    let qweather_only = std::collections::BTreeMap::from([(Source::Qweather, future)]); // 反向：仅和风被限流。
    let error = retry_gate(&qweather_only, Source::Qweather).expect_err("被限流的来源必须被拦截");
    assert!(error.contains("qweather"), "错误文案必须指明被限流的来源：{error}");
    assert!(retry_gate(&qweather_only, Source::Foreca).is_ok(), "另一个来源不受限流影响");
    let expired = std::collections::BTreeMap::from([(Source::Foreca, chrono::Utc::now().timestamp() - 1)]); // 已离开等待期的时间戳。
    assert!(retry_gate(&expired, Source::Foreca).is_ok(), "已过期的等待不得继续拦截");
    assert!(retry_gate(&std::collections::BTreeMap::new(), Source::Qweather).is_ok(), "没有等待记录时不拦截");
}

/// 真实搜索入口遵守该来源的等待期，另一个来源仍进入自己的凭据读取。
#[test]
fn saved_search_respects_the_source_retry_gate()
{
    let _credentials = serialize_credentials(); // 与其它涉凭据用例串行。
    let directory = test_directory("saved-search-gate");
    let state = WeatherState::for_test(&directory, ""); // 空记录名不触碰真实凭据。
    state.inner.lock().unwrap().retry_at.insert(Source::Foreca, chrono::Utc::now().timestamp() + 60); // 仅 Foreca 被限流。
    let error = state.search_saved(Source::Foreca, "北京".into()).err().expect("被限流的来源必须先被拒绝");
    assert!(error.contains("foreca") && error.contains("等待"), "错误文案必须指向该来源的等待期：{error}");
    let qweather = state.search_saved(Source::Qweather, "北京".into()).err().expect("空记录名无法读取凭据");
    assert!(!qweather.contains("等待"), "未限流的来源不得被门控拦截，应进入自己的凭据读取：{qweather}"); // 空记录名不触网。
    let _ = std::fs::remove_dir_all(directory);
}

/// 探测位置优先取该来源第一座城市，空列表退回固定占位位置。
#[test]
fn probe_uses_the_first_city_or_the_placeholder()
{
    let first = qweather_city("101010100"); // 该来源列表中的第一座城市。
    let probed = probe_city_from(vec![first.clone(), qweather_city("101010300")]);
    assert_eq!(probed.id, first.id, "探测必须使用列表第一座城市的标识");
    assert_eq!(probed.name, first.name, "探测必须使用列表第一座城市的名称");
    let placeholder = probe_city_from(Vec::new()); // 尚未添加城市时的占位位置。
    assert_eq!(placeholder.id, "probe");
    assert_eq!((placeholder.lat.as_str(), placeholder.lon.as_str()), ("39.90", "116.40"));
}

/// Begin 合并已保存私钥后 Save 端到端保留敏感值，同时按草稿更新非敏感字段。
#[test]
fn begin_and_save_keep_the_saved_private_key()
{
    let _credentials = serialize_credentials(); // 与其它涉凭据用例串行。
    let target = format!("DesktopTools/Test/Weather/begin-save-{}", std::process::id()); // 仅写入本测试专用目标。
    let directory = test_directory("begin-save");
    let state = WeatherState::for_test(&directory, &target);
    let execute = |command| state.execute(command, state.window_epoch().unwrap()); // 默认使用当前窗口代次。
    credentials::clear(&target).unwrap();
    let first_key = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIJ1hsZ3v/VpguoRK9JLsLMREScVpezJpGXA7rAMcrn9g\n-----END PRIVATE KEY-----"; // 公开测试种子，不读取真实凭据。
    let first = execute(Command::Begin { credentials: Credentials::Qweather(QweatherCredentials { host: "test.qweatherapi.com".into(), developer_id: "Q12345ABCD".into(), project_id: "test".into(), credential_id: "test".into(), private_key: first_key.into() }) }).unwrap().session_id.unwrap();
    execute(Command::Save { session_id: first }).unwrap(); // 第一次保存写入完整凭据。
    let second = execute(Command::Begin { credentials: Credentials::Qweather(QweatherCredentials { host: "test.qweatherapi.com".into(), developer_id: "Q999999999".into(), project_id: "test".into(), credential_id: "test".into(), private_key: String::new() }) }).unwrap().session_id.unwrap(); // 编辑态未重新输入私钥。
    execute(Command::Save { session_id: second }).unwrap(); // 第二次保存必须沿用已保存私钥。
    let saved = credentials::load(Source::Qweather, &target).unwrap().expect("保存后必须存在测试记录");
    let Credentials::Qweather(saved) = saved
    else
    {
        unreachable!("测试记录固定为和风凭据")
    };
    assert_eq!(saved.private_key, first_key, "留空的私钥必须沿用第一次保存的值");
    assert_eq!(saved.developer_id, "Q999999999", "非敏感字段按第二次草稿更新");
    credentials::clear(&target).unwrap();
    let _ = std::fs::remove_dir_all(directory);
}

/// 城市列表升级只读取当前按来源命名的文件，不迁移旧数据也不改动旧文件。
#[test]
fn legacy_city_file_is_not_migrated()
{
    let directory = test_directory("legacy-city");
    let legacy_path = directory.join("weather-city.json"); // 升级前所有来源共用的旧文件。
    let legacy = serde_json::to_vec(&[qweather_city("101010100")]).unwrap(); // 旧文件中的完整城市数组。
    std::fs::write(&legacy_path, &legacy).unwrap();
    let state = WeatherState::for_test(&directory, ""); // 空记录名不触碰真实凭据。
    let current_path = directory.join("weather-cities-qweather.json"); // 当前按来源命名的文件。
    assert!(state.cities(Source::Qweather).unwrap().is_empty(), "旧文件不得被读取为当前城市列表");
    assert!(!current_path.exists(), "仅读取不得创建当前格式文件");
    state.save_city(Source::Qweather, qweather_city("101010300")).unwrap();
    assert!(current_path.exists(), "保存后必须写入当前格式文件");
    assert_eq!(std::fs::read(&legacy_path).unwrap(), legacy, "升级路径不得迁移或改动旧数据");
    assert_eq!(state.cities(Source::Qweather).unwrap().iter().map(|city| city.id.as_str()).collect::<Vec<_>>(), ["101010300"]);
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn forecast_merge_keeps_cached_modules_and_clears_successful_errors()
{
    let city = qweather_city("101010100");
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
    let city = qweather_city("101010100");
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
