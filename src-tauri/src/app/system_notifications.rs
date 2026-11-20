//! 系统通知监听驱动：授权后轮询通知中心，把只读条目并入同一消息通道。
use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Manager};

/// 启动监听线程；未授权或读取失败只记录一次，不阻塞应用其余功能。
pub(crate) fn start(app: AppHandle)
{
    std::thread::spawn(move || {
        use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize};
        let initialized = unsafe { RoInitialize(RO_INIT_MULTITHREADED) }; // 监听线程独占 WinRT 生命周期。
        if initialized.is_err()
        {
            crate::log_warn!("系统通知：监听线程初始化失败，跳过系统通知接入");
            return;
        }
        if !request_access(&app)
        {
            crate::log_info!("系统通知：未获得通知访问授权，浮岛只接收本程序消息");
            unsafe { RoUninitialize() };
            return;
        }
        crate::log_info!("系统通知：已获得通知访问授权，开始按秒读取通知中心");
        let mut failure_logged = false; // 同一故障只记一次，避免每秒刷日志。
        loop
        {
            match crate::platform::windows::system_notifications::read()
            {
                Ok(list) =>
                {
                    failure_logged = false;
                    if !list.is_empty()
                    {
                        let _ = crate::tools::log::debug(format!("系统通知：读取到 {} 条当前通知", list.len()), file!(), line!());
                    }
                    let entries = list.into_iter().map(|item| crate::notifications::ExternalEntry { key: item.key, source: item.source, content: item.content, received_at_ms: item.received_at_ms, icon_url: item.icon_url }).collect::<Vec<_>>();
                    if let Some(manager) = app.try_state::<Arc<crate::notifications::ToastManager>>()
                    {
                        manager.sync_external(entries); // 与原生 Toast 共用同一投影与广播。
                    }
                }
                Err(error) if !failure_logged =>
                {
                    failure_logged = true;
                    crate::log_warn!("系统通知：读取通知中心失败，保持本程序消息，错误={error}");
                }
                Err(_) => {}
            }
            std::thread::sleep(Duration::from_secs(2)); // 通知中心没有可靠的新增事件，按固定节奏轮询快照。
        }
    });
}

/// 授权请求必须回到 UI 线程，结果通过通道传回监听线程。
fn request_access(app: &AppHandle) -> bool
{
    let (sender, receiver) = std::sync::mpsc::channel();
    let host = app.clone();
    if host.run_on_main_thread(move || {
        let allowed = crate::platform::windows::system_notifications::request_access().unwrap_or(false);
        let _ = sender.send(allowed);
    })
    .is_err()
    {
        return false;
    }
    receiver.recv_timeout(Duration::from_secs(20)).unwrap_or(false)
}
