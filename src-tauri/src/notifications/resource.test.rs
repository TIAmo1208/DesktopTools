use super::*;
use std::time::Duration;
use windows::Win32::{
    Foundation::FILETIME,
    System::{
        ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX},
        Threading::{GR_GDIOBJECTS, GR_USEROBJECTS, GetCurrentProcess, GetGuiResources, GetProcessHandleCount, GetProcessTimes},
    },
};

/// 采集独立测试进程的累计 CPU、内存与 Windows 对象，不包含 DWM 或 WebView2。
fn resources() -> serde_json::Value
{
    let process = unsafe { GetCurrentProcess() }; // 当前测量进程。
    let mut created = FILETIME::default(); // API 必填的创建时间。
    let mut exited = FILETIME::default(); // API 必填的退出时间。
    let mut kernel = FILETIME::default(); // 内核 CPU 累计时间。
    let mut user = FILETIME::default(); // 用户 CPU 累计时间。
    let mut memory = PROCESS_MEMORY_COUNTERS_EX::default(); // 私有提交与整个工作集。
    let mut handles = 0; // 内核句柄数量。
    unsafe {
        GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user).unwrap();
        GetProcessMemoryInfo(process, (&mut memory as *mut PROCESS_MEMORY_COUNTERS_EX).cast(), std::mem::size_of_val(&memory) as u32).unwrap();
        GetProcessHandleCount(process, &mut handles).unwrap();
    }
    let cpu_ticks = ((kernel.dwHighDateTime as u64) << 32 | kernel.dwLowDateTime as u64) + ((user.dwHighDateTime as u64) << 32 | user.dwLowDateTime as u64); // 100ns 单位。
    serde_json::json!({"cpu_ms": cpu_ticks as f64 / 10000.0, "private_bytes": memory.PrivateUsage, "working_set_bytes": memory.WorkingSetSize, "handles": handles, "gdi": unsafe { GetGuiResources(process, GR_GDIOBJECTS) }, "user": unsafe { GetGuiResources(process, GR_USEROBJECTS) }})
}

/// 使用真实消息等待与分发，空闲时阻塞；计时器次数只统计 Toast 自有计时器。
fn sample(manager: Option<&ToastManager>, round: usize, scenario: &str, seconds: u64)
{
    let before = resources(); // 窗口开始前的累计值。
    let started = Instant::now(); // 实际墙钟起点。
    let duration = Duration::from_secs(seconds); // 有限测量窗口。
    let mut timers = 0; // WM_TIMER 实际交付次数。
    while started.elapsed() < duration
    {
        let remaining = duration.saturating_sub(started.elapsed()).as_millis().max(1) as u32; // 不使用空闲轮询。
        unsafe { MsgWaitForMultipleObjectsEx(None, remaining, QS_ALLINPUT, MWMO_INPUTAVAILABLE) };
        let mut message = MSG::default(); // 待分发的线程窗口消息。
        while unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) }.as_bool()
        {
            if message.message == WM_TIMER && manager.is_some_and(|manager| manager.hwnd() == message.hwnd)
            {
                timers += 1;
            }
            unsafe { DispatchMessageW(&message) };
        }
    }
    let elapsed = started.elapsed().as_secs_f64(); // 含真实绘制耗时。
    let after = resources(); // 窗口结束后的累计值。
    let cpu_ms = after["cpu_ms"].as_f64().unwrap() - before["cpu_ms"].as_f64().unwrap(); // 累计 CPU 差分。
    let surface_info = SURFACE.with(|surface| surface.borrow().as_ref().map(|surface| serde_json::json!({"width":surface.size.cx,"height":surface.size.cy,"scale":surface.scale}))); // 实际绘制尺寸与 DPI。
    let surface_bytes = SURFACE.with(|surface| surface.borrow().as_ref().map_or(0, |surface| surface.size.cx as u64 * surface.size.cy as u64 * 4)); // 自有 BGRA DIB，不含 D2D 和合成器副本。
    let hovered = manager.is_some_and(|manager| manager.runtime.lock().unwrap().queue.hovered_since.is_some()); // 记录真实指针是否干扰场景。
    println!("TOAST_RESOURCE {}", serde_json::json!({"round":round,"scenario":scenario,"seconds":elapsed,"cpu_ms":cpu_ms,"one_core_percent":cpu_ms / (elapsed * 10.0),"timers":timers,"surface_bytes":surface_bytes,"surface":surface_info,"hovered":hovered,"before":before,"after":after}));
}

/// 仅显式执行的 Release 原生资源测量，不启动应用、WebView2 或媒体订阅。
#[test]
#[ignore = "会短暂显示真实 Toast；使用 Release、单测试线程测量"]
fn native_toast_resource_usage()
{
    assert!(!cfg!(debug_assertions), "资源测量必须使用 --release");
    unsafe { windows::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) }.unwrap(); // 与 Tauri/Tao 的 Windows 10 初始化一致。
    sample(None, 0, "未创建窗口", 3);
    let preferences = ApplicationPreferences { follow_system_reduced_motion: false, interface_animations: true, ..Default::default() }; // 固定开启动画，不改变用户设置。
    let manager = ToastManager::install(preferences).unwrap(); // 与产品相同的原生 HWND。
    sample(Some(&manager), 0, "空窗口", 3);
    for round in 1..=3
    {
        for (scenario, count, loading, expanded) in [("普通消息", 1, false, false), ("单条loading", 1, true, false), ("五条loading展开", 5, true, true), ("loading折叠隐藏", 2, true, false)]
        {
            let before = resources(); // 包含表面重建的起点。
            let started = Instant::now(); // 首帧时间。
            {
                let mut runtime = manager.runtime.lock().unwrap(); // 入队后释放，避免窗口重入。
                for index in 0..count
                {
                    runtime.queue.enqueue(ToastRequest { r#type: "info".into(), content: format!("消息资源测量 {index}"), source: "resource-test".into(), dedupe_key: None, loading: Some(loading && (scenario != "loading折叠隐藏" || index == 0)), duration_ms: 60000 }, 64.0, Instant::now(), true);
                }
                if expanded
                {
                    runtime.queue.hover(true, Instant::now(), true);
                }
            }
            manager.paint().unwrap();
            println!("TOAST_FIRST_FRAME {}", serde_json::json!({"round":round,"scenario":scenario,"wall_ms":started.elapsed().as_secs_f64()*1000.0,"before":before,"after":resources()}));
            sample(Some(&manager), round, "进场过渡", 1);
            sample(Some(&manager), round, scenario, 5);
            manager.runtime.lock().unwrap().queue = ToastState::default();
            manager.paint().unwrap();
            assert!(SURFACE.with(|surface| surface.borrow().is_none()));
            sample(Some(&manager), round, "清空后", 1);
        }
    }
    manager.cleanup();
    sample(None, 4, "销毁窗口后", 3);
}
