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

/// 停止/退出任一到达后只监看等待立即返回；独立的设置事件不会唤醒或重新激活它。
#[test]
fn monitor_only_drops_settings_before_and_after_terminal()
{
    use super::{GuardianMode, observer_terminal};
    for mode in [GuardianMode::Takeover, GuardianMode::MonitorOnly]
    {
        assert_eq!(GuardianMode::parse(Some(&mode.verb().to_owned())), Ok(mode));
    }
    assert!(GuardianMode::parse(Some(&"unknown".into())).is_err());
    for (index, expected) in [(0, "主进程退出"), (1, "正常停止")]
    {
        let make = || unsafe { windows::core::Owned::new(CreateEventW(None, true, false, None).unwrap()) };
        let events = [make(), make(), make()];
        let raw = [events[0].0 as isize, events[1].0 as isize];
        let (tx, rx) = mpsc::channel();
        let worker = std::thread::spawn(move || tx.send(observer_terminal(HANDLE(raw[0] as *mut _), HANDLE(raw[1] as *mut _))).unwrap());
        unsafe { SetEvent(*events[2]) }.unwrap();
        assert!(matches!(rx.recv_timeout(Duration::from_millis(60)), Err(mpsc::RecvTimeoutError::Timeout)));
        unsafe { SetEvent(*events[index]) }.unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(1)).unwrap().unwrap(), expected);
        worker.join().unwrap();
        unsafe { SetEvent(*events[2]) }.unwrap();
        assert!(matches!(rx.try_recv(), Err(mpsc::TryRecvError::Disconnected)));
    }
}
