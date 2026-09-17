use super::{EnvironmentNotification, wait_for_notification};
use std::sync::mpsc;
use std::time::Duration;
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Threading::{CreateEventW, INFINITE, SetEvent};

/// 空闲等待不会自行返回，环境事件到达后立即唤醒。
#[test]
fn environment_wait_blocks_until_event()
{
    let stop = unsafe { CreateEventW(None, false, false, None) }.unwrap();
    let environment = unsafe { CreateEventW(None, false, false, None) }.unwrap();
    let visibility = unsafe { CreateEventW(None, false, false, None) }.unwrap();
    let raw = [stop.0 as isize, environment.0 as isize, visibility.0 as isize]; // 等待线程借用，主线程统一关闭。
    let (completed, result) = mpsc::sync_channel(0);
    let worker = std::thread::spawn(move || completed.send(wait_for_notification(windows::Win32::Foundation::HANDLE(raw[0] as *mut _), windows::Win32::Foundation::HANDLE(raw[1] as *mut _), windows::Win32::Foundation::HANDLE(raw[2] as *mut _), INFINITE)).unwrap());
    assert!(matches!(result.recv_timeout(Duration::from_millis(100)), Err(mpsc::RecvTimeoutError::Timeout)));
    unsafe { SetEvent(environment) }.unwrap();
    assert!(matches!(result.recv_timeout(Duration::from_secs(1)).unwrap().unwrap(), Some(EnvironmentNotification::Environment)));
    worker.join().unwrap();
    for event in [visibility, environment, stop]
    {
        unsafe { CloseHandle(event) }.unwrap();
    }
}
