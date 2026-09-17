use super::{GuardianEvent, wait_for_guardian_event};
use std::sync::mpsc;
use std::time::Duration;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Threading::{CreateEventW, SetEvent};

/// guardian 空闲等待不会自行返回，设置事件到达后立即唤醒。
#[test]
fn guardian_wait_blocks_until_setting_event()
{
    let main = unsafe { CreateEventW(None, false, false, None) }.unwrap();
    let shutdown = unsafe { CreateEventW(None, false, false, None) }.unwrap();
    let visibility = unsafe { CreateEventW(None, false, false, None) }.unwrap();
    let raw = [main.0 as isize, shutdown.0 as isize, visibility.0 as isize]; // 等待线程借用，主线程统一关闭。
    let (completed, result) = mpsc::sync_channel(0);
    let worker = std::thread::spawn(move || completed.send(wait_for_guardian_event(HANDLE(raw[0] as *mut _), HANDLE(raw[1] as *mut _), HANDLE(raw[2] as *mut _))).unwrap());
    assert!(matches!(result.recv_timeout(Duration::from_millis(100)), Err(mpsc::RecvTimeoutError::Timeout)));
    unsafe { SetEvent(visibility) }.unwrap();
    assert!(matches!(result.recv_timeout(Duration::from_secs(1)).unwrap().unwrap(), GuardianEvent::VisibilityChanged));
    worker.join().unwrap();
    for event in [visibility, shutdown, main]
    {
        unsafe { CloseHandle(event) }.unwrap();
    }
}
