use super::*;

#[test]
fn cancellation_releases_session_even_when_credential_status_fails()
{
    let cancelled = Arc::new(AtomicBool::new(false)); // 模拟在途请求持有的取消标记。
    let draft = Arc::new(Credentials { host: String::new(), developer_id: String::new(), project_id: String::new(), credential_id: String::new(), private_key: "test-draft".into() }); // 不访问真实账户。
    let released = Arc::downgrade(&draft); // 验证会话释放私钥所有权。
    let state = WeatherState {
        target: String::new(), // 空目标使 Windows 凭据读取确定失败，不改变系统服务。
        inner: Mutex::new(Inner { session: Some(Session { id: 2, credentials: draft, cancelled: Arc::clone(&cancelled), cities: Vec::new(), verified: true }), busy: true, ..Default::default() }),
    }; // 已验证但仍有在途请求的会话。
    assert!(credentials::configured(&state.target).is_err());
    assert!(state.execute(Command::Cancel { session_id: 1 }, 0).is_err());
    assert!(!cancelled.load(Ordering::Acquire));
    assert!(released.upgrade().is_some());
    assert!(state.execute(Command::Cancel { session_id: 2 }, 0).is_err());
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
    let state = WeatherState { target: target.clone(), inner: Mutex::new(Inner::default()) }; // 不使用真实应用配置路径。
    let execute = |command| state.execute(command, state.window_epoch().unwrap()); // 默认使用当前窗口代次。
    credentials::clear(&target).unwrap();
    assert!(!execute(Command::Status).unwrap().configured);
    assert!(execute(Command::Save { session_id: 1 }).is_err());
    let draft = Credentials { host: "test.qweatherapi.com".into(), developer_id: "Q12345ABCD".into(), project_id: "test".into(), credential_id: "test".into(), private_key: "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIJ1hsZ3v/VpguoRK9JLsLMREScVpezJpGXA7rAMcrn9g\n-----END PRIVATE KEY-----".into() }; // 公开测试种子。
    let first = execute(Command::Begin { credentials: draft.clone() }).unwrap().session_id.unwrap(); // 第一份草稿。
    assert!(execute(Command::Save { session_id: first }).is_err());
    let second = execute(Command::Begin { credentials: draft.clone() }).unwrap().session_id.unwrap(); // 新草稿使旧会话失效。
    execute(Command::Cancel { session_id: first }).unwrap();
    assert_eq!(state.inner.lock().unwrap().session.as_ref().unwrap().id, second);
    credentials::save(&target, &draft).unwrap();
    assert!(execute(Command::Status).unwrap().configured);
    let mut oversized = draft.clone(); // 模拟不能写入的替换，旧记录必须保留。
    oversized.private_key = "x".repeat(3000);
    assert!(credentials::save(&target, &oversized).is_err());
    assert!(credentials::configured(&target).unwrap());
    let old_epoch = state.window_epoch().unwrap(); // 模拟窗口关闭前已进入队列的请求。
    state.cancel_all();
    assert!(state.execute(Command::Begin { credentials: draft }, old_epoch).is_err());
    assert!(execute(Command::Save { session_id: second }).is_err());
    execute(Command::Clear).unwrap();
    assert!(!credentials::configured(&target).unwrap());
}
