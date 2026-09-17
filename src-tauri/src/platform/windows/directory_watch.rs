use std::collections::HashMap;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::sync::{
    LazyLock, Mutex,
    atomic::{AtomicU64, Ordering},
};
use tauri::Emitter;
use windows::Win32::Foundation::{HANDLE, WAIT_OBJECT_0};
use windows::Win32::Storage::FileSystem::{CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OVERLAPPED, FILE_LIST_DIRECTORY, FILE_NOTIFY_CHANGE_ATTRIBUTES, FILE_NOTIFY_CHANGE_DIR_NAME, FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_NOTIFY_CHANGE_SIZE, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, ReadDirectoryChangesW};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows::Win32::System::Threading::{CreateEventW, INFINITE, SetEvent, WaitForMultipleObjects};
use windows::core::{HSTRING, Owned};

/// 单个门户订阅的取消事件和显示器归属。
struct Subscription
{
    monitor: String,
    stop: OwnedHandle, // 标准库提供跨线程安全的内核句柄所有权。
}
static WATCHES: LazyLock<Mutex<HashMap<u64, Subscription>>> = LazyLock::new(Default::default); // 持有取消事件直到监听线程完成。
static NEXT_ID: AtomicU64 = AtomicU64::new(1); // 进程内唯一订阅标识。

/// 监听已通过命令层范围校验的目录，通知只表示应重新读取事实。
pub(crate) fn start(app: tauri::AppHandle, monitor: String, path: std::path::PathBuf) -> Result<u64, String>
{
    let directory = unsafe { CreateFileW(&HSTRING::from(path.as_os_str()), FILE_LIST_DIRECTORY.0, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, None, OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED, None) }.map_err(|error| error.to_string())?; // 可取消目录句柄。
    let directory_owner = unsafe { OwnedHandle::from_raw_handle(directory.0) }; // 所有失败路径关闭句柄。
    let parent_owner = path.parent().filter(|parent| *parent != path).and_then(|parent| unsafe { CreateFileW(&HSTRING::from(parent.as_os_str()), FILE_LIST_DIRECTORY.0, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, None, OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED, None) }.map(|handle| unsafe { OwnedHandle::from_raw_handle(handle.0) }).map_err(|error| crate::log_warn!("门户：父目录监听不可用，目录={}，错误={error}", parent.display())).ok()); // 父目录的名称变化用于发现当前目录自身删除或改名。
    let stop = unsafe { CreateEventW(None, true, false, None) }.map_err(|error| error.to_string())?; // 退订唤醒事件。
    let stop = unsafe { OwnedHandle::from_raw_handle(stop.0) }; // 保持至线程退出。
    let stop_raw = stop.as_raw_handle() as usize; // 线程借用，所有者由注册表保持。
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed); // 本次订阅身份。
    WATCHES.lock().map_err(|_| "目录监听锁不可用")?.insert(id, Subscription { monitor, stop });
    let (ready, initialized) = std::sync::mpsc::sync_channel(1); // 首次 I/O 已提交后才确认订阅成功。
    // ponytail: 每个活动门户一个阻塞线程；大量同路径门户时按规范化路径共享监听。
    std::thread::spawn(move || {
        let directory = &HANDLE(directory_owner.as_raw_handle()); // 内核句柄所有者保持至未决 I/O 结束。
        let parent = parent_owner.as_ref().map(|owner| HANDLE(owner.as_raw_handle())); // 仅在当前目录有可访问父级时监听。
        let result = (|| -> windows::core::Result<()> {
            unsafe {
                let changed = Owned::new(CreateEventW(None, false, false, None)?); // I/O 完成事件。
                let parent_changed = parent.as_ref().map(|_| CreateEventW(None, false, false, None)).transpose()?.map(|event| Owned::new(event)); // 父目录完成事件。
                let mut buffer = [0_u32; 16384]; // 64 KiB，四字节对齐且兼容远程目录限制。
                let mut parent_buffer = [0_u32; 16384]; // 父目录只读取名称变更。
                loop
                {
                    let mut overlap = OVERLAPPED { hEvent: *changed, ..Default::default() }; // 缓冲区与请求活到 I/O 完成。
                    ReadDirectoryChangesW(*directory, buffer.as_mut_ptr().cast(), size_of_val(&buffer) as u32, false, FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_DIR_NAME | FILE_NOTIFY_CHANGE_SIZE | FILE_NOTIFY_CHANGE_LAST_WRITE | FILE_NOTIFY_CHANGE_ATTRIBUTES, None, Some(&mut overlap), None)?;
                    let mut parent_overlap = parent_changed.as_ref().map(|event| OVERLAPPED { hEvent: **event, ..Default::default() }); // 与子目录独立的未决 I/O。
                    if let (Some(parent), Some(parent_overlap)) = (parent, parent_overlap.as_mut())
                        && let Err(error) = ReadDirectoryChangesW(parent, parent_buffer.as_mut_ptr().cast(), size_of_val(&parent_buffer) as u32, false, FILE_NOTIFY_CHANGE_DIR_NAME, None, Some(parent_overlap), None)
                    {
                        let mut bytes = 0;
                        let _ = CancelIoEx(*directory, Some(&overlap));
                        let _ = GetOverlappedResult(*directory, &overlap, &mut bytes, true);
                        return Err(error);
                    }
                    let _ = ready.try_send(Ok(())); // 满队列或已接收均不阻塞后续监听。
                    let mut waits = vec![HANDLE(stop_raw as *mut _), *changed]; // 停止、内容和父目录事件共用阻塞等待。
                    if let Some(event) = &parent_changed
                    {
                        waits.push(**event);
                    }
                    let wait = WaitForMultipleObjects(&waits, false, INFINITE); // 无变化时不轮询。
                    let mut bytes = 0; // 完成字节数，零值也触发完整重读。
                    if wait == WAIT_OBJECT_0 || wait != windows::Win32::Foundation::WAIT_EVENT(WAIT_OBJECT_0.0 + 1) && (parent.is_none() || wait != windows::Win32::Foundation::WAIT_EVENT(WAIT_OBJECT_0.0 + 2))
                    {
                        let _ = CancelIoEx(*directory, Some(&overlap));
                        let _ = GetOverlappedResult(*directory, &overlap, &mut bytes, true);
                        if let (Some(parent), Some(parent_overlap)) = (parent, parent_overlap.as_ref())
                        {
                            let _ = CancelIoEx(parent, Some(parent_overlap));
                            let _ = GetOverlappedResult(parent, parent_overlap, &mut bytes, true);
                        }
                        if wait == WAIT_OBJECT_0
                        {
                            return Ok(());
                        }
                        return Err(windows::core::Error::from_win32());
                    }
                    if wait == windows::Win32::Foundation::WAIT_EVENT(WAIT_OBJECT_0.0 + 1)
                    {
                        let result = GetOverlappedResult(*directory, &overlap, &mut bytes, false);
                        if let (Some(parent), Some(parent_overlap)) = (parent, parent_overlap.as_ref())
                        {
                            let _ = CancelIoEx(parent, Some(parent_overlap));
                            let _ = GetOverlappedResult(parent, parent_overlap, &mut bytes, true);
                        }
                        result?;
                    }
                    else if let (Some(parent), Some(parent_overlap)) = (parent, parent_overlap.as_ref())
                    {
                        let result = GetOverlappedResult(parent, parent_overlap, &mut bytes, false);
                        let _ = CancelIoEx(*directory, Some(&overlap));
                        let _ = GetOverlappedResult(*directory, &overlap, &mut bytes, true);
                        result?;
                    }
                    let _ = app.emit("folder-portal-changed", serde_json::json!({"subscription": id}));
                }
            }
        })(); // 溢出零字节不解释增量，直接通知完整重读。
        if let Err(error) = result
        {
            let _ = ready.try_send(Err(error.to_string()));
            crate::log_warn!("门户：目录监听失效，目录={}，错误={error}", path.display());
            let _ = app.emit("folder-portal-changed", serde_json::json!({"subscription": id,"error":error.to_string()}));
        }
        if let Ok(mut watches) = WATCHES.lock()
        {
            watches.remove(&id);
        }
    });
    initialized.recv().map_err(|_| "目录监听初始化线程已退出".to_string())??;
    Ok(id)
}

/// 退订只发取消信号，线程完成未决 I/O 后释放注册项。
pub(crate) fn stop(id: u64)
{
    if let Ok(watches) = WATCHES.lock()
        && let Some(watch) = watches.get(&id)
    {
        let _ = unsafe { SetEvent(HANDLE(watch.stop.as_raw_handle())) };
    }
}

/// 宿主销毁或重载时取消全部所属目录监听。
pub(crate) fn clear_monitor(monitor: &str)
{
    if let Ok(watches) = WATCHES.lock()
    {
        for watch in watches.values().filter(|watch| watch.monitor == monitor)
        {
            let _ = unsafe { SetEvent(HANDLE(watch.stop.as_raw_handle())) };
        }
    }
}
