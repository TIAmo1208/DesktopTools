//! 系统通知监听：读取当前用户的现代 Toast 通知，映射为消息通道的只读条目。
//! 只做读取与身份归一，不写系统通知中心；权限、轮询与合并由应用层驱动。
use windows::Foundation::DateTime;
use windows::Storage::Streams::{DataReader, RandomAccessStreamReference};
use windows::UI::Notifications::Management::{UserNotificationListener, UserNotificationListenerAccessStatus};
use windows::UI::Notifications::{KnownNotificationBindings, NotificationKinds, UserNotification};

/// 一条系统通知的只读投影；字段与消息通道的只读条目一一对应。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SystemNotification
{
    pub key: String,              // 应用身份与系统通知 Id 组成的稳定身份。
    pub source: String,           // 应用显示名。
    pub content: String,          // 标题与正文合并后的文本。
    pub received_at_ms: u64,      // 系统通知创建时刻的墙钟毫秒。
    pub icon_url: Option<String>, // 应用 Logo 数据 URL。
}

/// 读取当前通知中心里的 Toast 通知；调用方只在已授权时使用。
pub fn read() -> Result<Vec<SystemNotification>, String>
{
    let listener = UserNotificationListener::Current().map_err(|error| error.to_string())?;
    let operation = listener.GetNotificationsAsync(NotificationKinds::Toast).map_err(|error| error.to_string())?;
    let notifications = wait(&operation)?;
    let mut result = Vec::new(); // 逐条容错：单条读取失败不影响其余通知。
    for notification in notifications
    {
        if let Some(mapped) = map_notification(&notification)
        {
            result.push(mapped);
        }
    }
    Ok(result)
}

/// 请求通知监听授权；未授权或拒绝返回 false，不当作错误反复重试。
pub fn request_access() -> Result<bool, String>
{
    let listener = UserNotificationListener::Current().map_err(|error| error.to_string())?;
    if let Ok(UserNotificationListenerAccessStatus::Allowed) = listener.GetAccessStatus()
    {
        return Ok(true);
    }
    let operation = listener.RequestAccessAsync().map_err(|error| error.to_string())?;
    let status = wait(&operation)?;
    Ok(status == UserNotificationListenerAccessStatus::Allowed)
}

/// 等待 WinRT 异步操作完成；调用方是专用监听线程，阻塞不影响界面。
fn wait<T>(operation: &windows_future::IAsyncOperation<T>) -> Result<T, String>
where
    T: windows::core::RuntimeType,
{
    operation.get().map_err(|error| error.to_string())
}

/// 映射单条系统通知；缺应用名、缺文本或身份无效的条目直接跳过。
fn map_notification(notification: &UserNotification) -> Option<SystemNotification>
{
    let id = notification.Id().ok()?;
    let app = notification.AppInfo().ok()?;
    let app_id = app.AppUserModelId().map(|value| value.to_string()).unwrap_or_default();
    let display = app.DisplayInfo().ok()?;
    let source = display.DisplayName().map(|value| value.to_string()).unwrap_or_default();
    if source.trim().is_empty()
    {
        return None;
    }
    let content = read_text(notification)?;
    let received_at_ms = notification.CreationTime().ok().map(datetime_to_unix_ms).unwrap_or_else(now_ms);
    let icon_url = display.GetLogo(windows::Foundation::Size { Width: 64.0, Height: 64.0 }).ok().and_then(|logo| read_logo(&logo));
    Some(SystemNotification { key: format!("{app_id}#{id}"), source, content, received_at_ms, icon_url })
}

/// 标题与正文都来自 ToastGeneric 的文本元素；按显示顺序用分隔点串成一行。
fn read_text(notification: &UserNotification) -> Option<String>
{
    let visual = notification.Notification().ok()?.Visual().ok()?;
    let binding = visual.GetBinding(&KnownNotificationBindings::ToastGeneric().ok()?).ok()?;
    let texts = binding.GetTextElements().ok()?;
    let mut parts = Vec::new();
    for text in texts
    {
        let value = text.Text().map(|value| value.to_string()).unwrap_or_default();
        if !value.trim().is_empty()
        {
            parts.push(value);
        }
    }
    if parts.is_empty()
    {
        return None;
    }
    Some(parts.join(" · "))
}

/// 应用 Logo 转数据 URL；尺寸与格式不合格时退回没有 Logo 的条目。
fn read_logo(reference: &RandomAccessStreamReference) -> Option<String>
{
    let stream = wait(&reference.OpenReadAsync().ok()?).ok()?;
    let size = stream.Size().ok()?;
    if size == 0 || size > 512 * 1024
    {
        return None;
    }
    let reader = DataReader::CreateDataReader(&stream).ok()?;
    wait(&reader.LoadAsync(size as u32).ok()?).ok()?;
    let mut bytes = vec![0; size as usize];
    reader.ReadBytes(&mut bytes).ok()?;
    let mime = artwork_mime(&bytes)?;
    Some(format!("data:{mime};base64,{}", base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes)))
}

/// 只接受已支持的栅格图片签名，避免把未知字节塞进页面。
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

/// WinRT DateTime 以 1601 起的 100ns 为单位，换算成 Unix 毫秒。
fn datetime_to_unix_ms(value: DateTime) -> u64
{
    let ticks = value.UniversalTime as i128;
    const EPOCH_DIFFERENCE_TICKS: i128 = 116_444_736_000_000_000; // 1601-01-01 到 1970-01-01。
    let milliseconds = (ticks - EPOCH_DIFFERENCE_TICKS) / 10_000;
    u64::try_from(milliseconds.max(0)).unwrap_or(0)
}

/// 创建时刻缺失时的回落：用当前墙钟。
fn now_ms() -> u64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64 }
