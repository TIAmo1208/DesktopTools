use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant};
use windows::Media::Control::{GlobalSystemMediaTransportControlsSession as Session, GlobalSystemMediaTransportControlsSessionManager as SessionManager, GlobalSystemMediaTransportControlsSessionPlaybackStatus as PlaybackStatus};
pub const MUSIC_SESSION_CHANGED_EVENT: &str = "music-session-changed";

// 保留 COM 会话对象并分配代次，避免同一应用的不同会话被视作同一目标。
static CURRENT_SESSION: Mutex<(u64, Option<Session>)> = Mutex::new((0, None));
static ACTION_BUSY: AtomicBool = AtomicBool::new(false); // 两个 transport 共用的单次命令门闩。
static SESSION_MANAGER: OnceLock<SessionManager> = OnceLock::new(); // 会话身份仅在同一个管理器内稳定。

/// 动作线程退出或展开时释放门闩，超时调用方不能提前放行另一动作。
struct ActionGuard;
impl Drop for ActionGuard
{
    /// 仅在实际动作线程结束后允许下一次命令。
    fn drop(&mut self) { ACTION_BUSY.store(false, Ordering::Release); }
}

// 在统一期限内等待 WinRT 操作，超时请求取消且不重放命令。
macro_rules! wait_media {
    ($operation:expr, $deadline:expr) => {{
        let operation = $operation?; // 本次可取消的 WinRT 操作。
        loop
        {
            if Instant::now() >= $deadline
            {
                let _ = operation.Cancel();
                break Err(windows::core::Error::new(windows::core::HRESULT(0x800705B4u32 as i32), "媒体操作超时，结果未确认"));
            }
            if operation.Status()?.0 != 0
            {
                break operation.GetResults();
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }};
}

/// 读取与动作共用同一管理器，初始化失败允许后续重试。
fn session_manager(deadline: Instant) -> windows::core::Result<SessionManager>
{
    if let Some(manager) = SESSION_MANAGER.get()
    {
        return Ok(manager.clone());
    }
    let manager = wait_media!(SessionManager::RequestAsync(), deadline)?; // 首次初始化的管理器。
    let _ = SESSION_MANAGER.set(manager);
    Ok(SESSION_MANAGER.get().expect("媒体管理器已经初始化").clone())
}

/// 当前系统媒体会话的可序列化事实，不包含播放器推测值。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MusicSnapshot
{
    pub session_id: String,          // 当前保留会话的实例代次。
    pub source: String,              // Windows 提供的来源身份。
    pub title: String,               // 媒体标题，缺失时为空。
    pub artist: String,              // 艺人，缺失时为空。
    pub playback: &'static str,      // 已确认的播放状态。
    pub can_previous: bool,          // 当前会话是否支持上一曲。
    pub can_play_pause: bool,        // 当前状态对应动作是否支持。
    pub can_next: bool,              // 当前会话是否支持下一曲。
    pub artwork_url: Option<String>, // 当前封面数据 URL。
}

/// 在专用 WinRT 线程读取媒体，两个 transport 共用此入口。
pub fn snapshot() -> Result<serde_json::Value, String>
{
    static WORKER: OnceLock<mpsc::SyncSender<mpsc::Sender<Result<serde_json::Value, String>>>> = OnceLock::new(); // 每个进程只有一个有界读取队列。
    let worker = WORKER.get_or_init(|| {
        let (sender, receiver) = mpsc::sync_channel::<mpsc::Sender<Result<serde_json::Value, String>>>(1); // 最多保留一次等待中的读取。
        std::thread::spawn(move || {
            use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize};
            let initialized = unsafe { RoInitialize(RO_INIT_MULTITHREADED) }; // WinRT 生命周期限定于工作线程。
            for reply in receiver
            {
                let result = match &initialized
                {
                    Ok(()) => read_current_session().map(|snapshot| serde_json::json!({ "media": snapshot })).map_err(|error| format!("读取媒体失败：{error}")),
                    Err(error) => Err(format!("初始化媒体控制失败：{error}")),
                }; // 初始化失败不能进入媒体 API。
                let _ = reply.send(result);
            }
            if initialized.is_ok()
            {
                unsafe { RoUninitialize() };
            }
        });
        sender
    }); // 惰性创建，不在未使用音乐功能时连接播放器。
    let (reply, response) = mpsc::channel(); // 当前调用独立的返回通道。
    worker.try_send(reply).map_err(|_| "媒体读取仍在处理中".to_string())?;
    response.recv_timeout(Duration::from_secs(5)).map_err(|_| "媒体连接超时".to_string())?
}

/// 只在媒体快照变化时发布事件，前端不再自行轮询。
pub fn start_monitor(app: tauri::AppHandle)
{
    std::thread::spawn(move || {
        let mut previous = None;
        loop
        {
            let current = snapshot().unwrap_or_else(|error| serde_json::json!({ "media": null, "error": error })); // 故障也参与状态变化与恢复发布。
            if previous.as_ref() != Some(&current)
            {
                if let Some(error) = current.get("error").and_then(|value| value.as_str())
                {
                    let _ = crate::tools::log::warn(format!("媒体同步失败：{error}"));
                }
                previous = Some(current.clone());
                use tauri::Emitter;
                let _ = app.emit(MUSIC_SESSION_CHANGED_EVENT, current);
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    });
}

/// 在已经初始化 WinRT 的工作线程读取当前媒体；无会话与 API 故障分别返回。
pub fn read_current_session() -> windows::core::Result<Option<MusicSnapshot>>
{
    let deadline = Instant::now() + Duration::from_secs(4); // 为外层五秒返回期限预留调度空间。
    let manager = session_manager(deadline)?; // 与动作共用同一个管理器。
    let session = match manager.GetCurrentSession()
    {
        Ok(session) => session,
        Err(error) if is_absent_session(&error) =>
        {
            CURRENT_SESSION.lock().unwrap_or_else(|error| error.into_inner()).1 = None;
            return Ok(None);
        }
        Err(error) => return Err(error),
    }; // windows-core 将成功调用返回的空接口映射为 Error::empty()，仅此情况代表无会话。
    let session_id = {
        let mut current = CURRENT_SESSION.lock().unwrap_or_else(|error| error.into_inner()); // 当前会话保留记录。
        if current.1.as_ref() != Some(&session)
        {
            current.0 += 1;
            current.1 = Some(session.clone());
        }
        current.0.to_string()
    }; // 会话对象变化时更新代次，不使用来源名称识别实例。
    let properties = wait_media!(session.TryGetMediaPropertiesAsync(), deadline)?; // 当前曲目元数据。
    let info = session.GetPlaybackInfo()?; // 播放状态与能力的系统快照。
    let controls = info.Controls()?; // 当前播放器声明的控制能力。
    let status = info.PlaybackStatus()?; // 当前确定的播放状态。
    let playback = match status
    {
        PlaybackStatus::Playing => "playing",
        PlaybackStatus::Paused => "paused",
        PlaybackStatus::Stopped => "stopped",
        _ => "unknown",
    }; // 未知状态不推测播放方向。
    let artwork_url = read_artwork(&properties, deadline).ok().flatten(); // 封面错误不能使有效控制失效。
    if manager.GetCurrentSession().ok().as_ref() != Some(&session)
    {
        return Err(windows::core::Error::new(windows::core::HRESULT(0x80004004u32 as i32), "播放来源已变化，正在重新同步媒体"));
    }
    Ok(Some(MusicSnapshot {
        session_id,
        source: session.SourceAppUserModelId()?.to_string(),
        title: properties.Title()?.to_string(),
        artist: properties.Artist()?.to_string(),
        playback,
        can_previous: controls.IsPreviousEnabled()?,
        can_play_pause: match status
        {
            PlaybackStatus::Playing => controls.IsPauseEnabled()?,
            PlaybackStatus::Paused | PlaybackStatus::Stopped => controls.IsPlayEnabled()?,
            _ => false,
        },
        can_next: controls.IsNextEnabled()?,
        artwork_url,
    }))
}

/// 仅识别 Windows 绑定对空接口的转换错误，不吞掉真实 HRESULT 故障。
fn is_absent_session(error: &windows::core::Error) -> bool { error.code().0 == 0 }

#[cfg(test)]
mod empty_session_tests
{
    /// 图片类型不依赖播放器提供的 MIME 列表，非图片不得通过。
    #[test]
    fn artwork_signatures()
    {
        assert_eq!(super::artwork_mime(&[0xff, 0xd8, 0xff]), Some("image/jpeg"));
        assert_eq!(super::artwork_mime(b"\x89PNG\r\n\x1a\n"), Some("image/png"));
        assert_eq!(super::artwork_mime(b"RIFF1234WEBP"), Some("image/webp"));
        assert_eq!(super::artwork_mime(b"RIFF1234WAVE"), None);
        assert_eq!(super::artwork_mime(b"<svg/>"), None);
        assert_eq!(super::artwork_mime(b""), None);
    }
    /// 对比同管理器和跨管理器身份，并检查真实封面流属性，不控制播放器。
    #[test]
    #[ignore = "只读媒体诊断，需显式执行"]
    fn diagnose_session_identity_and_artwork() -> windows::core::Result<()>
    {
        use super::SessionManager;
        use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize};
        unsafe {
            RoInitialize(RO_INIT_MULTITHREADED)?;
        }
        let result = (|| -> windows::core::Result<()> {
            let first = SessionManager::RequestAsync()?.get()?; // 第一次管理器。
            let second = SessionManager::RequestAsync()?.get()?; // 模拟控制线程重新创建管理器。
            let session = first.GetCurrentSession()?; // 当前媒体对象。
            eprintln!("同管理器身份相同={}，跨管理器身份相同={}", session == first.GetCurrentSession()?, session == second.GetCurrentSession()?);
            let properties = session.TryGetMediaPropertiesAsync()?.get()?; // 当前媒体元数据。
            match properties.Thumbnail()
            {
                Ok(reference) =>
                {
                    let stream = reference.OpenReadAsync()?.get()?; // 原始封面流。
                    eprintln!("封面字节={}，MIME={:?}", stream.Size()?, stream.ContentType()?.to_string());
                }
                Err(error) => eprintln!("封面引用错误={error}"),
            }
            Ok(())
        })();
        unsafe {
            RoUninitialize();
        }
        result
    }
    use super::is_absent_session;
    use std::cell::Cell;
    use std::time::{Duration, Instant};
    use windows::Media::Control::GlobalSystemMediaTransportControlsSession;
    use windows::core::{Error, HRESULT, Type};

    /// 复现 GetCurrentSession 成功返回空指针时绑定层产生的真实错误。
    #[test]
    fn null_session_is_empty_not_connection_failure()
    {
        let error = unsafe { GlobalSystemMediaTransportControlsSession::from_abi(std::ptr::null_mut()) }.unwrap_err(); // 与生成绑定相同的空接口转换。
        assert!(is_absent_session(&error));
        assert!(!is_absent_session(&Error::from_hresult(HRESULT(0x80070005u32 as i32))));
    }

    /// 只模拟 WinRT 等待协议，验证超时确实取消且不读取未完成结果。
    struct PendingOperation
    {
        cancelled: Cell<bool>,
    }
    #[allow(non_snake_case)]
    impl PendingOperation
    {
        /// 返回持续未完成状态。
        fn Status(&self) -> windows::core::Result<(i32,)> { Ok((0,)) }
        /// 记录取消请求。
        fn Cancel(&self) -> windows::core::Result<()>
        {
            self.cancelled.set(true);
            Ok(())
        }
        /// 未完成操作不得读取结果。
        fn GetResults(&self) -> windows::core::Result<bool> { panic!("未完成操作不应读取结果") }
    }

    /// 超时只取消本次操作，不把未知结果当成功。
    #[test]
    fn pending_operation_is_cancelled_at_deadline() -> windows::core::Result<()>
    {
        let operation = PendingOperation { cancelled: Cell::new(false) }; // 永不完成的操作。
        let deadline = Instant::now() + Duration::from_millis(1); // 测试用短期限。
        let result = wait_media!(Ok::<_, Error>(&operation), deadline); // 执行生产等待逻辑。
        assert_eq!(result.unwrap_err().code(), HRESULT(0x800705B4u32 as i32));
        assert!(operation.cancelled.get());
        Ok(())
    }

    /// 非法动作在接触真实播放器之前拒绝。
    #[test]
    fn invalid_action_is_rejected_before_system_access()
    {
        assert!(super::execute_action("play-pause", "1").is_err());
        assert!(super::execute_action("play", "").is_err());
    }

    /// 按需验证当前机器真实 GSMTC 只读路径，不控制播放器。
    #[test]
    #[ignore = "只读实机验证，需显式执行"]
    fn current_machine_media_snapshot()
    {
        let snapshot = super::snapshot().expect("读取实际 Windows 媒体快照"); // 当前系统的真实快照。
        assert!(snapshot.get("media").is_some());
        if !snapshot["media"].is_null()
        {
            assert!(snapshot["media"]["sessionId"].as_str().is_some_and(|value| !value.is_empty()));
        }
        eprintln!("GSMTC 只读验证通过：存在会话={}", !snapshot["media"].is_null());
        let next = super::snapshot().expect("重复读取媒体"); // 同一会话连续读取身份必须稳定。
        assert_eq!(snapshot["media"]["sessionId"], next["media"]["sessionId"]);
        if !snapshot["media"].is_null()
        {
            std::thread::spawn(|| -> windows::core::Result<()> {
                use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize};
                unsafe {
                    RoInitialize(RO_INIT_MULTITHREADED)?;
                }
                let manager = super::session_manager(Instant::now() + Duration::from_secs(4))?; // 模拟控制线程。
                let session = manager.GetCurrentSession()?; // 不发送控制命令。
                assert!(super::CURRENT_SESSION.lock().unwrap().1.as_ref() == Some(&session));
                drop(session);
                drop(manager);
                unsafe {
                    RoUninitialize();
                }
                Ok(())
            })
            .join()
            .unwrap()
            .unwrap();
            eprintln!("跨线程目标校验通过，封面存在={}", snapshot["media"]["artworkUrl"].is_string());
        }
    }
}

/// 将 GSMTC 封面转换为前端可使用的数据 URL。
fn read_artwork(properties: &windows::Media::Control::GlobalSystemMediaTransportControlsSessionMediaProperties, deadline: Instant) -> windows::core::Result<Option<String>>
{
    use windows::Storage::Streams::DataReader;
    let reference = match properties.Thumbnail()
    {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };
    let stream = match wait_media!(reference.OpenReadAsync(), deadline)
    {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };
    let size = stream.Size()?; // 先检查完整大小，不截断图片编码。
    if size == 0 || size > 2 * 1024 * 1024
    {
        return Ok(None);
    }
    let reader = DataReader::CreateDataReader(&stream)?;
    if wait_media!(reader.LoadAsync(size as u32), deadline)? != size as u32
    {
        return Ok(None);
    }
    let mut bytes = vec![0; size as usize];
    reader.ReadBytes(&mut bytes)?;
    let Some(mime) = artwork_mime(&bytes)
    else
    {
        return Ok(None);
    }; // 以文件签名识别格式，兼容非标准 MIME 列表。
    Ok(Some(format!("data:{mime};base64,{}", base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes))))
}

/// 只允许已支持的栅格图片文件签名。
fn artwork_mime(bytes: &[u8]) -> Option<&'static str>
{
    if bytes.starts_with(&[0xff, 0xd8, 0xff])
    {
        Some("image/jpeg")
    }
    else if bytes.starts_with(b"\x89PNG\r\n\x1a\n")
    {
        Some("image/png")
    }
    else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")
    {
        Some("image/gif")
    }
    else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP")
    {
        Some("image/webp")
    }
    else if bytes.starts_with(b"BM")
    {
        Some("image/bmp")
    }
    else
    {
        None
    }
}

/// 对当前 GSMTC 会话执行一个已声明的媒体动作。
pub fn execute_action(action: &str, expected_session: &str) -> Result<(), String>
{
    if !matches!(action, "previous" | "play" | "pause" | "next") || expected_session.is_empty()
    {
        return Err("媒体动作或会话参数无效".to_string());
    }
    let action = action.to_string(); // 发送到工作线程的明确动作。
    let expected_session = expected_session.to_string(); // 用户点击时的会话代次。
    if ACTION_BUSY.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_err()
    {
        return Err("媒体动作仍在处理中，请稍后重新操作".to_string());
    }
    let (reply, response) = mpsc::channel(); // 调用方有界等待，不 join 潜在卡住的 COM 调用。
    std::thread::spawn(move || {
        let _guard = ActionGuard; // 在线程真实结束时解锁。
        let result = (|| -> Result<(), String> {
            use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize};
            unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.map_err(|error| format!("初始化媒体控制失败：{error}"))?;
            let result = (|| -> windows::core::Result<()> {
                let deadline = Instant::now() + Duration::from_secs(4); // 整条动作链共用期限，不排队重放。
                let manager = session_manager(deadline)?; // 与快照共用管理器，避免身份误判。
                let session = manager.GetCurrentSession()?; // 执行前重新获取实际目标。
                let target_matches = {
                    let current = CURRENT_SESSION.lock().unwrap_or_else(|error| error.into_inner()); // 最近发布的会话实例。
                    current.0.to_string() == expected_session && current.1.as_ref() == Some(&session)
                }; // 应用身份相同但实例不同也拒绝。
                if !target_matches
                {
                    return Err(windows::core::Error::new(windows::core::HRESULT(0x80004004u32 as i32), "播放来源已变化，请重新操作"));
                }
                let controls = session.GetPlaybackInfo()?.Controls()?; // 执行时重新校验播放器能力。
                let enabled = match action.as_str()
                {
                    "previous" => controls.IsPreviousEnabled()?,
                    "play" => controls.IsPlayEnabled()?,
                    "pause" => controls.IsPauseEnabled()?,
                    "next" => controls.IsNextEnabled()?,
                    _ => false,
                }; // 与前端显示的明确动作一致。
                if !enabled
                {
                    return Err(windows::core::Error::new(windows::core::HRESULT(0x80004001u32 as i32), "播放器不支持当前动作"));
                }
                if Instant::now() >= deadline
                {
                    return Err(windows::core::Error::new(windows::core::HRESULT(0x800705B4u32 as i32), "媒体操作超时，未发送动作"));
                }
                let _ = crate::tools::log::debug(format!("发送媒体动作：动作={action}，会话={expected_session}"), file!(), line!());
                let operation = match action.as_str()
                {
                    "previous" => session.TrySkipPreviousAsync(),
                    "play" => session.TryPlayAsync(),
                    "pause" => session.TryPauseAsync(),
                    "next" => session.TrySkipNextAsync(),
                    _ => unreachable!(),
                }; // 动作只发出一次，超时也不能重试。
                if wait_media!(operation, deadline)?
                {
                    Ok(())
                }
                else
                {
                    Err(windows::core::Error::new(windows::core::HRESULT(0x80004005u32 as i32), "播放器拒绝了媒体动作"))
                }
            })();
            unsafe { RoUninitialize() };
            result.map_err(|error| format!("媒体控制未完成：{error}，正在重新同步媒体"))
        })(); // 初始化失败也通过同一结果通道返回。
        if let Err(error) = &result
        {
            let _ = crate::tools::log::warn(error.clone());
        }
        let _ = reply.send(result);
    });
    response.recv_timeout(Duration::from_secs(5)).map_err(|_| "媒体动作结果未确认，正在重新同步媒体".to_string())?
}
