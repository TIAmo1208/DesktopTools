use std::ffi::c_void;
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command};
use std::thread;
use std::time::Duration;

use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{
    CREATE_NEW_PROCESS_GROUP, CreateEventW, EVENT_MODIFY_STATE, INFINITE, OpenEventW, OpenProcess,
    PROCESS_SYNCHRONIZE, SYNCHRONIZATION_ACCESS_RIGHTS, SetEvent, WaitForMultipleObjects,
    WaitForSingleObject,
};
use windows::core::PCWSTR;

use crate::platform::windows::{
    ShellDesktopState, discover_shell_desktop, redraw_shell_desktop, set_desktop_items_visible,
};

/// 独立 guardian 子进程与正常停止事件。
pub struct GuardianProcess {
    /// 持续监视正式主进程的 guardian。
    child: Option<Child>,
    /// 正常退出时通知 guardian 停止的事件。
    shutdown_event: isize,
}

/// 管理 guardian 正常停止流程。
impl GuardianProcess {
    /// 启动 guardian 并等待其完成就绪握手。
    pub fn start(
        guardian_path: &Path,
        main_process_id: u32,
        shell: ShellDesktopState,
    ) -> Result<Self, String> {
        // guardian 就绪事件名称。
        let ready_name = format!("Local\\DesktopToolsGuardianReady_{main_process_id}");
        // guardian 正常停止事件名称。
        let shutdown_name = format!("Local\\DesktopToolsGuardianShutdown_{main_process_id}");
        // guardian 就绪事件名称 UTF-16 缓冲区。
        let ready_name_wide = wide(&ready_name);
        // guardian 停止事件名称 UTF-16 缓冲区。
        let shutdown_name_wide = wide(&shutdown_name);
        // 主进程等待的 guardian 就绪事件。
        let ready_event = unsafe {
            CreateEventW(None, true, false, PCWSTR(ready_name_wide.as_ptr()))
                .map_err(|error| format!("创建 guardian 就绪事件失败: {error}"))?
        };
        // 正常退出时触发的 guardian 停止事件。
        let shutdown_event = unsafe {
            CreateEventW(None, true, false, PCWSTR(shutdown_name_wide.as_ptr()))
                .map_err(|error| format!("创建 guardian 停止事件失败: {error}"))?
        };
        // 独立 guardian 子进程。
        let mut child = Command::new(guardian_path)
            .args([
                "monitor",
                "--pid",
                &main_process_id.to_string(),
                "--def-view",
                &shell.def_view.to_string(),
                "--visible",
                if shell.original_items_visible {
                    "1"
                } else {
                    "0"
                },
                "--ready-event",
                &ready_name,
                "--shutdown-event",
                &shutdown_name,
            ])
            .creation_flags(CREATE_NEW_PROCESS_GROUP.0)
            .spawn()
            .map_err(|error| format!("启动 guardian 失败: {error}"))?;
        // guardian 就绪握手等待结果。
        let wait_result = unsafe { WaitForSingleObject(ready_event, 5000) };
        unsafe {
            let _ = CloseHandle(ready_event);
        }
        if wait_result != WAIT_OBJECT_0 {
            let _ = child.kill();
            unsafe {
                let _ = CloseHandle(shutdown_event);
            }
            return Err(format!("guardian 就绪握手超时，等待结果：{wait_result:?}"));
        }

        crate::log_info!("恢复：guardian 已就绪，进程标识：{}", child.id());
        Ok(Self {
            child: Some(child),
            shutdown_event: shutdown_event.0 as isize,
        })
    }

    /// 通知 guardian 当前主进程已完成正常退出恢复。
    pub fn shutdown(&mut self) {
        if self.shutdown_event != 0 {
            unsafe {
                // 通知 guardian 正常停止的事件句柄。
                let event = HANDLE(self.shutdown_event as *mut c_void);
                let _ = SetEvent(event);
                let _ = CloseHandle(event);
            }
            self.shutdown_event = 0;
        }
        // 等待正常停止的 guardian 子进程。
        let Some(child) = &mut self.child else {
            return;
        };
        for _ in 0..20 {
            if child.try_wait().ok().flatten().is_some() {
                return;
            }
            thread::sleep(Duration::from_millis(50));
        }
        let _ = child.kill();
        let _ = child.wait();
        self.child = None;
    }

    /// 保留 guardian 继续监视主进程退出并关闭父进程持有的句柄。
    pub fn continue_recovery(mut self) {
        if self.shutdown_event != 0 {
            unsafe {
                // 仅关闭主进程副本并保留 guardian 监视的事件句柄。
                let event = HANDLE(self.shutdown_event as *mut c_void);
                let _ = CloseHandle(event);
            }
            self.shutdown_event = 0;
        }
        self.child = None;
    }
}

/// 在默认资源释放路径中执行 guardian 正常停止契约。
impl Drop for GuardianProcess {
    /// 在主进程正常资源释放时停止 guardian。
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// 执行 guardian 命令行入口。
pub fn run_guardian() -> Result<(), String> {
    // guardian 收到的全部命令行参数。
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    if arguments.first().map(String::as_str) != Some("monitor") {
        return Err("guardian 命令无效".to_string());
    }

    monitor_main_process(&arguments)
}

/// 将 Rust 字符串转换为带结尾空字符的 UTF-16 缓冲区。
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain([0]).collect()
}

/// 读取一个命令行选项后紧随的值。
fn argument_value(arguments: &[String], name: &str) -> Option<String> {
    arguments
        .windows(2)
        .find(|pair| pair[0] == name)
        .map(|pair| pair[1].clone())
}

/// 监视正式主进程，并在异常退出时恢复 Explorer 桌面项目。
fn monitor_main_process(arguments: &[String]) -> Result<(), String> {
    // 需要监视的正式主进程标识。
    let main_process_id = argument_value(arguments, "--pid")
        .ok_or_else(|| "缺少 --pid".to_string())?
        .parse::<u32>()
        .map_err(|error| format!("--pid 无效: {error}"))?;
    // 接管前 DefView 句柄。
    let original_def_view = argument_value(arguments, "--def-view")
        .ok_or_else(|| "缺少 --def-view".to_string())?
        .parse::<isize>()
        .map_err(|error| format!("--def-view 无效: {error}"))?;
    // 接管前 Explorer 桌面项目可见状态。
    let original_visibility =
        argument_value(arguments, "--visible").ok_or_else(|| "缺少 --visible".to_string())? == "1";
    // guardian 就绪事件名称。
    let ready_event_name = argument_value(arguments, "--ready-event")
        .ok_or_else(|| "缺少 --ready-event".to_string())?;
    // guardian 正常停止事件名称。
    let shutdown_event_name = argument_value(arguments, "--shutdown-event")
        .ok_or_else(|| "缺少 --shutdown-event".to_string())?;
    // guardian 就绪事件名称 UTF-16 缓冲区。
    let ready_event_name_wide = wide(&ready_event_name);
    // guardian 停止事件名称 UTF-16 缓冲区。
    let shutdown_event_name_wide = wide(&shutdown_event_name);

    // 只申请等待权限的正式主进程句柄。
    let main_process = unsafe {
        OpenProcess(PROCESS_SYNCHRONIZE, false, main_process_id)
            .map_err(|error| format!("打开主进程失败: {error}"))?
    };
    // 就绪事件需要修改状态权限。
    let ready_event = unsafe {
        OpenEventW(
            EVENT_MODIFY_STATE,
            false,
            PCWSTR(ready_event_name_wide.as_ptr()),
        )
        .map_err(|error| format!("打开 guardian 就绪事件失败: {error}"))?
    };
    // 正常退出停止事件需要同步等待权限。
    let shutdown_access = SYNCHRONIZATION_ACCESS_RIGHTS(EVENT_MODIFY_STATE.0 | 0x0010_0000);
    // guardian 正常停止事件句柄。
    let shutdown_event = unsafe {
        OpenEventW(
            shutdown_access,
            false,
            PCWSTR(shutdown_event_name_wide.as_ptr()),
        )
        .map_err(|error| format!("打开 guardian 停止事件失败: {error}"))?
    };
    unsafe {
        SetEvent(ready_event).map_err(|error| format!("guardian 就绪握手失败: {error}"))?;
        let _ = CloseHandle(ready_event);
    }
    crate::log_info!("恢复：guardian 开始监视，主进程标识：{main_process_id}");

    // 主进程结束与正常停止事件的竞态等待结果。
    let wait_result =
        unsafe { WaitForMultipleObjects(&[main_process, shutdown_event], false, INFINITE) };
    unsafe {
        let _ = CloseHandle(main_process);
        let _ = CloseHandle(shutdown_event);
    }
    if wait_result == WAIT_OBJECT_0 {
        // Explorer 重启后重新发现的当前桌面层。
        let shell = discover_shell_desktop().unwrap_or(ShellDesktopState {
            def_view: original_def_view,
            explorer_host: original_def_view,
            original_items_visible: original_visibility,
        });
        // 优先恢复当前 DefView。
        let restored = set_desktop_items_visible(shell.def_view, original_visibility);
        redraw_shell_desktop(shell);
        crate::log_info!(
            "恢复：guardian 已恢复 Explorer，成功：{}",
            u8::from(restored)
        );
        if !restored {
            return Err("guardian 恢复 Explorer 桌面项目失败".to_string());
        }
    } else if wait_result.0 == WAIT_OBJECT_0.0 + 1 {
        crate::log_info!("恢复：guardian 收到正常停止事件");
    } else {
        return Err(format!("guardian 等待失败，结果：{wait_result:?}"));
    }
    Ok(())
}
