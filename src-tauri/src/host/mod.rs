use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::json;
use tauri::{AppHandle, Emitter, Manager};

use crate::commands::dispatch_native_command;
use crate::contracts::{DesktopHostContextSnapshot, DesktopHostError, DesktopHostReadiness, DesktopHostSnapshot, FrontendHostReadiness, HitRegion, TakeoverPhase};
use crate::platform::windows::{NativeCommandHandler, NativeDesktopHost, ShellDesktopState, apply_host_hit_regions, create_desktop_hosts, desktop_items_setting_visible, desktop_items_visible, destroy_desktop_hosts, discover_shell_desktop, enumerate_desktop_monitors, load_desktop_hosts, probe_host_pixels, redraw_shell_desktop, reload_desktop_host, set_desktop_items_visible, set_host_visible, suspend_host_hit_regions};
use crate::recovery::GuardianProcess;

/// Renderer 心跳停止后触发轻量重载的等待时间。
const RENDERER_HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(5);

/// Renderer 未恢复时再次尝试轻量重载的最小间隔。
const RENDERER_RELOAD_RETRY: Duration = Duration::from_secs(5);

/// Explorer 或显示器适配器连续失败后进入阻塞恢复的轮询次数。
const ENVIRONMENT_FAILURE_LIMIT: u32 = 20;

/// 协调全局 guardian 门槛、桌面接管提交和 Explorer 恢复阶段。
struct TakeoverCoordinator
{
    /// 接管前 Explorer 桌面项目可见状态。
    original_items_visible: bool,
    /// 独立 guardian 是否已经完成握手。
    guardian_ready: bool,
    /// 当前接管阶段。
    phase: TakeoverPhase,
}

/// 提供正式宿主启动与恢复状态转换。
impl TakeoverCoordinator
{
    /// 创建等待接管门槛的协调器。
    fn new(original_items_visible: bool) -> Self { Self { original_items_visible, guardian_ready: false, phase: TakeoverPhase::Starting } }

    /// 标记全局 guardian 门槛已经通过。
    fn mark_guardian_ready(&mut self) { self.guardian_ready = true; }

    /// 更新正常退出、超时和主进程异常恢复使用的最近用户设置。
    fn observe_user_visibility(&mut self, visible: bool) { self.original_items_visible = visible; }

    /// 在全部逐屏门槛满足时进入等待提交状态。
    fn mark_hosts_ready(&mut self)
    {
        if self.phase == TakeoverPhase::Starting
        {
            self.phase = TakeoverPhase::Ready;
        }
    }

    /// 获得本轮桌面接管提交权。
    fn begin_commit(&mut self) -> bool
    {
        if self.phase != TakeoverPhase::Ready
        {
            return false;
        }
        self.phase = TakeoverPhase::Committing;
        true
    }

    /// 返回当前桌面接管阶段。
    fn phase(&self) -> TakeoverPhase { self.phase }

    /// 返回桌面项目接管是否已经提交。
    fn is_committed(&self) -> bool { self.phase == TakeoverPhase::Committed }

    /// 记录 Explorer 隐藏和正式宿主显示已经完成。
    fn mark_committed(&mut self) { self.phase = TakeoverPhase::Committed; }

    /// 记录桌面接管提交失败。
    fn mark_failed(&mut self) { self.phase = TakeoverPhase::Failed; }

    /// 进入正常退出恢复阶段并返回原始 Explorer 可见性。
    fn restore(&mut self) -> bool
    {
        self.phase = TakeoverPhase::Restored;
        self.original_items_visible
    }
}

/// 一台显示器宿主独立拥有的原生资源、门槛、命中图和拖动状态。
struct DesktopHostContext
{
    /// 当前显示器对应的原生 Windowed WebView2 宿主。
    host: NativeDesktopHost,
    /// 当前宿主独立的接管准备状态与输入诊断状态。
    readiness: DesktopHostReadiness,
    /// 当前宿主最近一次完整语义命中图。
    hit_regions: Vec<HitRegion>,
    /// 当前宿主是否正在执行连续输入拖动。
    input_drag_active: bool,
    /// 当前 Renderer 最近一次前端心跳时间。
    last_renderer_heartbeat: Instant,
    /// 当前 Renderer 最近一次轻量重载请求时间。
    renderer_reload_requested_at: Option<Instant>,
}

/// 正式宿主持续持有的协调状态与原生资源记录。
struct DesktopHostRuntimeData
{
    /// 接管门槛与阶段协调器。
    coordinator: TakeoverCoordinator,
    /// 当前每台显示器对应的独立宿主上下文。
    hosts: Vec<DesktopHostContext>,
    /// 接管前 Explorer 桌面状态。
    shell: ShellDesktopState,
    /// 独立恢复进程。
    guardian: Option<GuardianProcess>,
    /// 正常退出资源释放是否已经执行。
    cleanup_complete: bool,
    /// 显示环境整组重建是否已经排队到 UI 线程。
    rebuild_in_progress: bool,
}

/// 持有单显示器透明宿主、接管门槛和 Explorer 恢复契约。
pub struct DesktopHostRuntime
{
    /// 跨命令和 watchdog 共享的正式宿主状态。
    data: Mutex<DesktopHostRuntimeData>,
}

/// 提供正式宿主启动、命中图发布、接管和恢复能力。
impl DesktopHostRuntime
{
    /// 创建正式宿主、启动 guardian 并导航 React 页面。
    pub fn start(app: &AppHandle) -> Result<Arc<Self>, String>
    {
        // 接管前 Explorer 桌面层状态。
        let shell = discover_shell_desktop()?;
        // guardian 可执行文件路径。
        let guardian_path = std::env::current_exe().map_err(|error| format!("读取主程序路径失败: {error}"))?.parent().ok_or_else(|| "主程序路径缺少父目录".to_string())?.join("desktop-tools-guardian.exe");
        if !guardian_path.is_file()
        {
            return Err(format!("缺少 guardian 可执行文件: {}", guardian_path.display()));
        }
        // 已完成握手的独立 guardian。
        let guardian = GuardianProcess::start(&guardian_path, std::process::id(), shell)?;
        // 页面命令处理器等待的正式宿主运行时。
        let runtime_slot = Arc::new(OnceLock::<Arc<Self>>::new());
        // 正式宿主页面调用的命令处理器。
        let command_handler: NativeCommandHandler = {
            // 本地命令分发使用的 Tauri 应用句柄。
            let command_app = app.clone();
            // 本地命令分发等待初始化完成的正式宿主槽位。
            let command_runtime = Arc::clone(&runtime_slot);
            // 将本地协议命令送入正式应用命令边界。
            Arc::new(move |command, arguments| {
                // 已完成初始化的正式宿主运行时。
                let runtime = command_runtime.get().ok_or_else(|| {
                    json!({
                        "code": "desktop-host-starting",
                        "message": "桌面宿主正在启动"
                    })
                })?;
                dispatch_native_command(&command_app, runtime, command, arguments)
            })
        };
        let monitors = enumerate_desktop_monitors()?; // 当前全部显示器物理状态。
        let hosts = create_desktop_hosts(app, shell, command_handler, &monitors)?; // 每台显示器独立的正式宿主。
        // 保存接管前 Explorer 可见性的门槛协调器。
        let mut coordinator = TakeoverCoordinator::new(shell.original_items_visible);
        coordinator.mark_guardian_ready();
        // 正式宿主共享运行时。
        let runtime = Arc::new(Self { data: Mutex::new(DesktopHostRuntimeData { coordinator, hosts: hosts.into_iter().map(|host| DesktopHostContext { host, readiness: DesktopHostReadiness { guardian: true, ..Default::default() }, hit_regions: Vec::new(), input_drag_active: false, last_renderer_heartbeat: Instant::now(), renderer_reload_requested_at: None }).collect(), shell, guardian: Some(guardian), cleanup_complete: false, rebuild_in_progress: false }) });
        runtime_slot.set(Arc::clone(&runtime)).map_err(|_| "正式宿主运行时重复初始化".to_string())?;
        load_desktop_hosts()?;
        crate::log_info!("宿主：等待全部逐屏页面、系统图像、像素和命中图门槛，宿主数量={}", monitors.len());
        Ok(runtime)
    }

    /// 返回当前正式宿主不可变快照。
    pub fn snapshot(&self) -> Result<DesktopHostSnapshot, DesktopHostError>
    {
        // 当前正式宿主共享状态。
        let data = self.data.lock().map_err(|_| DesktopHostError::state_unavailable())?;
        Ok(snapshot_from(&data))
    }

    /// 记录逐屏 Renderer 存活心跳，并结束当前轻量重载等待。
    pub fn record_renderer_heartbeat(&self, monitor_id: &str) -> Result<(), DesktopHostError>
    {
        let mut data = self.data.lock().map_err(|_| DesktopHostError::state_unavailable())?;
        let context = host_context_mut(&mut data, monitor_id)?; // 当前心跳所属的逐屏 Renderer。
        if context.renderer_reload_requested_at.take().is_some()
        {
            crate::log_info!("恢复：Renderer 轻量重载已恢复，显示器={monitor_id}");
        }
        context.last_renderer_heartbeat = Instant::now();
        Ok(())
    }

    /// 返回当前重新发现的 Explorer 桌面层状态。
    pub fn shell_state(&self) -> Result<ShellDesktopState, DesktopHostError> { self.data.lock().map(|data| data.shell).map_err(|_| DesktopHostError::state_unavailable()) }

    /// 校验逐屏页面提交的显示器稳定身份仍属于当前宿主集合。
    pub fn validate_monitor_id(&self, monitor_id: &str) -> Result<(), DesktopHostError>
    {
        let data = self.data.lock().map_err(|_| DesktopHostError::state_unavailable())?;
        host_context(&data, monitor_id).map(|_| ())
    }

    /// 校验并提交 React 双帧、系统图像和交互命中图证据。
    pub fn publish_frontend_readiness(&self, report: FrontendHostReadiness) -> Result<DesktopHostSnapshot, DesktopHostError>
    {
        let monitor_id = report.monitor_id.clone(); // 当前页面所属的显示器稳定身份。
        // 当前正式宿主记录。
        let host = {
            // 等待前端证据校验的正式宿主状态。
            let data = self.data.lock().map_err(|_| DesktopHostError::state_unavailable())?;
            let context = host_context(&data, &monitor_id)?; // 当前页面对应的逐屏宿主上下文。
            validate_frontend_readiness(&report, &context.host)?;
            context.host.clone()
        };
        // 已应用到 HWND 的语义区域数量。
        let region_count = apply_host_hit_regions(&host, report.hit_regions.clone()).map_err(|error| DesktopHostError::platform_failed(&error))?;
        // 像素仅作诊断，空屏或 PrintWindow 捕获失败不阻塞接管。
        let visible_pixels = probe_host_pixels(&host).unwrap_or_else(|error| {
            crate::log_debug!("宿主：像素诊断未取得可见内容，显示器={}，原因={error}", monitor_id);
            0
        });

        // 日志中用于验证输入坐标的首个语义命中区域。
        let first_region = report.hit_regions.first().map(|region| format!("{}:[{:.0}, {:.0}, {:.0}, {:.0}]", region.id, region.left, region.top, region.width, region.height)).unwrap_or_else(|| "无".to_string());
        // 将前端门槛写入当前逐屏宿主状态。
        {
            // 等待前端证据写入的正式宿主状态。
            let mut data = self.data.lock().map_err(|_| DesktopHostError::state_unavailable())?;
            let context = host_context_mut(&mut data, &monitor_id)?; // 等待写入前端证据的逐屏上下文。
            context.hit_regions = report.hit_regions;
            context.readiness.page = true;
            context.readiness.system_images = true;
            context.readiness.pixels = visible_pixels > 0;
            context.readiness.hit_map = true;
            if all_hosts_ready(&data)
            {
                data.coordinator.mark_hosts_ready();
            }
        }
        crate::log_debug!("宿主：逐屏前端门槛已通过，显示器={}，命中区域：{}，首个区域：{}，系统图像：{}/{}，真实像素：{}", monitor_id, region_count, first_region, report.loaded_system_images, report.total_system_images, visible_pixels);
        self.commit_if_ready()
    }

    /// 原子替换页面布局变化后的完整语义命中图。
    pub fn update_hit_map(&self, monitor_id: &str, hit_regions: Vec<HitRegion>) -> Result<DesktopHostSnapshot, DesktopHostError>
    {
        // 当前正式宿主与 CSS 视口尺寸。
        let host = {
            // 等待命中图校验的正式宿主状态。
            let mut data = self.data.lock().map_err(|_| DesktopHostError::state_unavailable())?;
            let context = host_context_mut(&mut data, monitor_id)?; // 当前命中图所属的逐屏上下文。
            validate_hit_regions(&hit_regions, context.host.monitor.work_width as f64 / context.host.monitor.scale_factor, context.host.monitor.work_height as f64 / context.host.monitor.scale_factor)?;
            if context.hit_regions == hit_regions
            {
                return Ok(snapshot_from(&data));
            }
            if context.input_drag_active
            {
                crate::platform::windows::cache_host_hit_regions(monitor_id, hit_regions.clone()).map_err(|error| DesktopHostError::platform_failed(&error))?;
                context.hit_regions = hit_regions;
                return Ok(snapshot_from(&data));
            } // 连续拖动期间只更新语义图，结束时统一恢复窗口裁剪。
            context.host.clone()
        };
        apply_host_hit_regions(&host, hit_regions.clone()).map_err(|error| DesktopHostError::platform_failed(&error))?;
        // 等待替换命中图的正式宿主状态。
        let mut data = self.data.lock().map_err(|_| DesktopHostError::state_unavailable())?;
        host_context_mut(&mut data, monitor_id)?.hit_regions = hit_regions;
        Ok(snapshot_from(&data))
    }

    /// 在连续拖动期间暂停语义命中区域裁剪。
    pub fn begin_input_drag(&self, monitor_id: &str) -> Result<DesktopHostSnapshot, DesktopHostError>
    {
        // 当前拖动所属的正式宿主记录。
        let host = self.data.lock().map_err(|_| DesktopHostError::state_unavailable())?.hosts.iter().find(|context| context.host.monitor.id == monitor_id).ok_or_else(|| DesktopHostError::invalid_readiness("显示器宿主不存在"))?.host.clone();
        suspend_host_hit_regions(&host).map_err(|error| DesktopHostError::platform_failed(&error))?;
        let mut data = self.data.lock().map_err(|_| DesktopHostError::state_unavailable())?;
        host_context_mut(&mut data, monitor_id)?.input_drag_active = true;
        crate::log_debug!("宿主：连续拖动命中区域已暂停，显示器={}，窗口句柄={:#x}", monitor_id, host.hwnd);
        Ok(snapshot_from(&data))
    }

    /// 结束连续拖动并恢复最近一份完整语义命中图。
    pub fn end_input_drag(&self, monitor_id: &str) -> Result<DesktopHostSnapshot, DesktopHostError>
    {
        // 等待恢复的正式宿主和最近命中图。
        let (host, hit_regions) = {
            // 当前受锁保护的正式宿主状态。
            let data = self.data.lock().map_err(|_| DesktopHostError::state_unavailable())?;
            let context = host_context(&data, monitor_id)?; // 当前结束拖动的逐屏上下文。
            (context.host.clone(), context.hit_regions.clone())
        };
        apply_host_hit_regions(&host, hit_regions).map_err(|error| DesktopHostError::platform_failed(&error))?;
        let mut data = self.data.lock().map_err(|_| DesktopHostError::state_unavailable())?;
        host_context_mut(&mut data, monitor_id)?.input_drag_active = false;
        crate::log_debug!("宿主：连续拖动命中区域已恢复，显示器={}，窗口句柄={:#x}", monitor_id, host.hwnd);
        Ok(snapshot_from(&data))
    }

    /// 返回 Shell 变化协调是否需要等待当前拖动结束。
    pub fn is_input_drag_active(&self) -> bool { self.data.lock().map(|data| data.hosts.iter().any(|context| context.input_drag_active)).unwrap_or(false) }

    /// 取消全部逐屏连续拖动并恢复最近完整命中图。
    pub fn cancel_input_drags(&self) -> Result<DesktopHostSnapshot, DesktopHostError>
    {
        let hosts = {
            let data = self.data.lock().map_err(|_| DesktopHostError::state_unavailable())?;
            data.hosts.iter().filter(|context| context.input_drag_active).map(|context| (context.host.clone(), context.hit_regions.clone())).collect::<Vec<_>>()
        }; // 当前需要恢复命中图的逐屏拖动宿主。
        for (host, hit_regions) in hosts
        {
            apply_host_hit_regions(&host, hit_regions).map_err(|error| DesktopHostError::platform_failed(&error))?;
        }
        let mut data = self.data.lock().map_err(|_| DesktopHostError::state_unavailable())?;
        for context in &mut data.hosts
        {
            context.input_drag_active = false;
        }
        Ok(snapshot_from(&data))
    }

    /// 记录 React 项目点击回传，供正式宿主输入诊断使用。
    pub fn record_input_probe(&self, monitor_id: &str, item_id: &str) -> Result<DesktopHostSnapshot, DesktopHostError>
    {
        {
            // 等待输入诊断状态写入的正式宿主状态。
            let mut data = self.data.lock().map_err(|_| DesktopHostError::state_unavailable())?;
            let context = host_context_mut(&mut data, monitor_id)?; // 当前输入回传所属的逐屏上下文。
            if !context.hit_regions.iter().any(|region| region.kind == "desktop-item" && region.id == item_id)
            {
                return Err(DesktopHostError::invalid_readiness("输入探针未命中已发布的桌面项目区域"));
            }
            context.readiness.input = true;
            if all_hosts_ready(&data)
            {
                data.coordinator.mark_hosts_ready();
            }
        }
        crate::log_debug!("宿主：最小输入探针已回传，显示器={}，项目引用：{}", monitor_id, item_id);
        self.commit_if_ready()
    }

    /// 正常退出时恢复 Explorer、停止 guardian 并销毁正式宿主。
    pub fn cleanup(&self)
    {
        // 正常恢复所需的资源快照。
        let cleanup = {
            // 等待正常恢复转换的正式宿主状态。
            let mut data = match self.data.lock()
            {
                Ok(data) => data,
                Err(error) =>
                {
                    crate::log_warn!("宿主：运行状态锁已中毒，继续执行退出恢复");
                    error.into_inner()
                }
            };
            if data.cleanup_complete
            {
                return;
            }
            data.cleanup_complete = true;
            // 正常退出恢复使用的接管前 Explorer 可见状态。
            let original_visibility = data.coordinator.restore();
            (data.hosts.iter().map(|context| context.host.clone()).collect::<Vec<_>>(), data.shell, original_visibility, data.guardian.take())
        };
        // 正常恢复使用的正式宿主、Shell 状态和 guardian。
        let (hosts, original_shell, restore_visibility, guardian) = cleanup;
        let host_hidden = hosts.iter().all(|host| set_host_visible(host.hwnd, false)); // 正常退出时撤下全部逐屏宿主的结果。
        let (restored, environment_verified) = restore_explorer_environment(original_shell, restore_visibility); // 正常退出时恢复并复核 Explorer。
        crate::log_info!("宿主：正常退出恢复，宿主隐藏：{}，Explorer 恢复：{}，环境检查：{}", u8::from(host_hidden), u8::from(restored), u8::from(environment_verified));
        if restored && environment_verified
        {
            if let Some(mut guardian) = guardian
            {
                guardian.shutdown();
            }
        }
        else if let Some(guardian) = guardian
        {
            crate::log_error!("恢复：Explorer 恢复失败，guardian 继续监视主进程退出");
            guardian.continue_recovery();
        }
        destroy_desktop_hosts();
    }

    /// 监视逐屏前端心跳，并按原型路径轻量重载失效 Renderer。
    pub fn start_renderer_watchdog(app: AppHandle, runtime: Arc<Self>)
    {
        thread::spawn(move || {
            loop
            {
                thread::sleep(Duration::from_secs(1));
                let reloads = {
                    let mut data = match runtime.data.lock()
                    {
                        Ok(data) => data,
                        Err(_) => return,
                    };
                    if data.cleanup_complete || !data.coordinator.is_committed() || matches!(data.coordinator.phase(), TakeoverPhase::Failed | TakeoverPhase::Restored)
                    {
                        if data.cleanup_complete || matches!(data.coordinator.phase(), TakeoverPhase::Failed | TakeoverPhase::Restored)
                        {
                            return;
                        }
                        continue;
                    }
                    let now = Instant::now(); // 本轮 Renderer 心跳超时判断基准。
                    data.hosts
                        .iter_mut()
                        .filter_map(|context| {
                            let heartbeat_age = now.saturating_duration_since(context.last_renderer_heartbeat); // 当前 Renderer 未上报心跳的时间。
                            let retry_age = context.renderer_reload_requested_at.map(|requested| now.saturating_duration_since(requested)); // 最近轻量重载后的等待时间。
                            if !renderer_restart_due(true, heartbeat_age, retry_age)
                            {
                                return None;
                            }
                            context.renderer_reload_requested_at = Some(now);
                            context.input_drag_active = false;
                            Some((context.host.clone(), context.hit_regions.clone()))
                        })
                        .collect::<Vec<_>>()
                };
                if reloads.is_empty()
                {
                    continue;
                }
                let reload_app = app.clone(); // UI 线程重载与状态事件使用的应用句柄。
                let reload_runtime = Arc::clone(&runtime); // UI 线程失败恢复使用的宿主运行时。
                if let Err(error) = app.run_on_main_thread(move || {
                    for (host, hit_regions) in reloads
                    {
                        let result = reload_desktop_host(&host.monitor.id).and_then(|_| apply_host_hit_regions(&host, hit_regions).map(|_| ()));
                        match result
                        {
                            Ok(()) => crate::log_warn!("恢复：Renderer 心跳超时，已轻量重载并保留命中图，显示器={}", host.monitor.id),
                            Err(error) => reload_runtime.fail_host_adapter(&format!("Renderer 轻量重载失败，显示器={}，错误={error}", host.monitor.id)),
                        }
                    }
                    if let Ok(snapshot) = reload_runtime.snapshot()
                    {
                        let _ = reload_app.emit(crate::contracts::DESKTOP_HOST_SNAPSHOT_CHANGED_EVENT, snapshot);
                    }
                })
                {
                    runtime.fail_host_adapter(&format!("Renderer 重载调度失败，错误={error}"));
                }
            }
        });
    }

    /// 按原型路径监视显示器布局、拓扑和 Explorer 宿主身份变化。
    pub fn start_environment_watchdog(app: AppHandle, runtime: Arc<Self>)
    {
        thread::spawn(move || {
            let mut environment_failures = 0_u32; // 连续无法读取 Explorer 或显示器环境的次数。
            loop
            {
                thread::sleep(Duration::from_millis(500));
                let observed_visibility = desktop_items_setting_visible(); // 当前用户“显示桌面图标”设置。
                let takeover_def_view = {
                    let mut data = match runtime.data.lock()
                    {
                        Ok(data) => data,
                        Err(_) => return,
                    };
                    if data.cleanup_complete || data.coordinator.phase() == TakeoverPhase::Failed
                    {
                        return;
                    }
                    data.coordinator.observe_user_visibility(observed_visibility);
                    data.shell.original_items_visible = observed_visibility;
                    (data.coordinator.is_committed() && desktop_items_visible(data.shell.def_view)).then_some(data.shell.def_view)
                }; // 用户更新设置后是否需要继续维持本轮临时桌面接管隐藏。
                if let Some(def_view) = takeover_def_view
                    && set_desktop_items_visible(def_view, false)
                {
                    crate::log_info!("宿主：已记录用户桌面图标设置并继续维持接管隐藏，恢复可见：{}", u8::from(observed_visibility));
                }
                let latest_shell = match discover_shell_desktop()
                {
                    Ok(shell) => shell,
                    Err(error) =>
                    {
                        environment_failures = environment_failures.saturating_add(1);
                        if environment_failure_is_blocking(environment_failures)
                        {
                            let failure_app = app.clone(); // UI 线程恢复与快照发布使用的应用句柄。
                            let failure_runtime = Arc::clone(&runtime); // UI 线程进入阻塞恢复的宿主运行时。
                            let reason = format!("Explorer 桌面宿主连续发现失败：{error}"); // 用户可检索的兼容适配器失败原因。
                            if app
                                .run_on_main_thread(move || {
                                    failure_runtime.fail_host_adapter(&reason);
                                    if let Ok(snapshot) = failure_runtime.snapshot()
                                    {
                                        let _ = failure_app.emit(crate::contracts::DESKTOP_HOST_SNAPSHOT_CHANGED_EVENT, snapshot);
                                    }
                                })
                                .is_err()
                            {
                                runtime.fail_host_adapter("Explorer 失败恢复调度失败");
                            }
                            return;
                        }
                        continue;
                    }
                }; // 当前重新发现的 Explorer 桌面宿主身份。
                let latest_monitors = match enumerate_desktop_monitors()
                {
                    Ok(monitors) => monitors,
                    Err(error) =>
                    {
                        environment_failures = environment_failures.saturating_add(1);
                        if environment_failure_is_blocking(environment_failures)
                        {
                            let failure_app = app.clone(); // UI 线程恢复与快照发布使用的应用句柄。
                            let failure_runtime = Arc::clone(&runtime); // UI 线程进入阻塞恢复的宿主运行时。
                            let reason = format!("显示器适配器连续枚举失败：{error}"); // 用户可检索的显示环境失败原因。
                            if app
                                .run_on_main_thread(move || {
                                    failure_runtime.fail_host_adapter(&reason);
                                    if let Ok(snapshot) = failure_runtime.snapshot()
                                    {
                                        let _ = failure_app.emit(crate::contracts::DESKTOP_HOST_SNAPSHOT_CHANGED_EVENT, snapshot);
                                    }
                                })
                                .is_err()
                            {
                                runtime.fail_host_adapter("显示器失败恢复调度失败");
                            }
                            return;
                        }
                        continue;
                    }
                }; // 当前全部显示器物理布局与 DPI。
                environment_failures = 0;
                let should_rebuild = {
                    let mut data = match runtime.data.lock()
                    {
                        Ok(data) => data,
                        Err(_) => return,
                    };
                    if data.cleanup_complete || data.coordinator.phase() == TakeoverPhase::Failed
                    {
                        return;
                    }
                    let current_monitors = data.hosts.iter().map(|context| context.host.monitor.clone()).collect::<Vec<_>>(); // 当前正式宿主使用的显示器状态。
                    let changed = shell_hosts_changed(data.shell, latest_shell) || monitor_layout_changed(&current_monitors, &latest_monitors);
                    if changed && !data.rebuild_in_progress
                    {
                        data.rebuild_in_progress = true;
                        true
                    }
                    else
                    {
                        false
                    }
                };
                if !should_rebuild
                {
                    continue;
                }
                let rebuild_app = app.clone(); // UI 线程重建和快照发布使用的应用句柄。
                let rebuild_runtime = Arc::clone(&runtime); // UI 线程更新的正式宿主运行时。
                if let Err(error) = app.run_on_main_thread(move || {
                    let result = rebuild_runtime.rebuild_environment(&rebuild_app, latest_shell, latest_monitors); // 本轮显示环境整组重建结果。
                    if let Err(error) = &result
                    {
                        crate::log_error!("宿主：显示环境重建失败，错误：{error}");
                        rebuild_runtime.fail_host_adapter(&format!("显示环境重建事务失败：{error}"));
                    }
                    if let Ok(snapshot) = rebuild_runtime.snapshot()
                    {
                        let _ = rebuild_app.emit(crate::contracts::DESKTOP_HOST_SNAPSHOT_CHANGED_EVENT, snapshot);
                    }
                    if let Ok(mut data) = rebuild_runtime.data.lock()
                    {
                        data.rebuild_in_progress = false;
                    }
                })
                {
                    crate::log_error!("宿主：显示环境重建调度失败，错误：{error}");
                    runtime.fail_host_adapter(&format!("显示环境重建调度失败：{error}"));
                }
            }
        });
    }

    /// 在 UI 线程按最新 Shell 和显示器状态完整重建逐屏宿主。
    fn rebuild_environment(self: &Arc<Self>, app: &AppHandle, mut latest_shell: ShellDesktopState, latest_monitors: Vec<crate::contracts::DesktopHostMonitor>) -> Result<(), String>
    {
        let (previous_shell, previous_monitors, original_visibility, was_committed) = {
            let data = self.data.lock().map_err(|_| "桌面宿主状态暂时不可用".to_string())?;
            (data.shell, data.hosts.iter().map(|context| context.host.monitor.clone()).collect::<Vec<_>>(), data.coordinator.original_items_visible, data.coordinator.is_committed())
        }; // 重建前用于分类变化和保留接管状态的环境快照。
        let shell_changed = shell_hosts_changed(previous_shell, latest_shell); // Explorer 桌面宿主身份是否变化。
        let topology_changed = monitor_topology_changed(&previous_monitors, &latest_monitors); // 显示器持久身份或物理边界是否变化。
        latest_shell.original_items_visible = original_visibility;
        let command_handler: NativeCommandHandler = {
            let command_app = app.clone(); // 新宿主本地协议使用的应用句柄。
            let command_runtime = Arc::clone(self); // 新宿主本地协议使用的正式运行时。
            Arc::new(move |command, arguments| crate::commands::dispatch_native_command(&command_app, &command_runtime, command, arguments))
        }; // 重建后全部 WebView2 共用的命令边界。
        let previous_hosts = {
            let data = self.data.lock().map_err(|_| "桌面宿主状态暂时不可用".to_string())?; // 复用前的原生宿主身份。
            if shell_changed
            {
                Vec::new()
            }
            else
            {
                data.hosts.iter().map(|context| context.host.clone()).collect::<Vec<_>>()
            }
        };
        let hosts = match crate::platform::windows::reconcile_desktop_hosts(app, latest_shell, command_handler, &latest_monitors, &previous_hosts)
        {
            Ok(hosts) => hosts,
            Err(error) =>
            {
                self.fail_host_adapter(&format!("显示环境重建创建宿主失败：{error}"));
                return Err(error);
            }
        }; // 最新环境中的逐屏原生宿主集合。
        {
            let mut data = self.data.lock().map_err(|_| "桌面宿主状态暂时不可用".to_string())?;
            data.shell = latest_shell;
            let mut previous_contexts = std::mem::take(&mut data.hosts); // 按 HWND 保留未重建屏幕的全部就绪与输入状态。
            data.hosts = hosts
                .into_iter()
                .map(|host| {
                    if let Some(index) = previous_contexts.iter().position(|context| !shell_changed && context.host.hwnd == host.hwnd && context.host.monitor == host.monitor)
                    {
                        return previous_contexts.remove(index);
                    }
                    DesktopHostContext { host, readiness: DesktopHostReadiness { guardian: data.coordinator.guardian_ready, ..Default::default() }, hit_regions: Vec::new(), input_drag_active: false, last_renderer_heartbeat: Instant::now(), renderer_reload_requested_at: None }
                })
                .collect();
            if shell_changed
            {
                data.coordinator.phase = TakeoverPhase::Starting;
            }
            else if was_committed
            {
                data.coordinator.phase = TakeoverPhase::Committed;
            }
        }
        crate::log_info!("宿主：显示环境已重建，Shell变化={}，拓扑变化={}，DPI或工作区变化={}，宿主数量={}，保留接管={}", u8::from(shell_changed), u8::from(topology_changed), u8::from(!shell_changed && !topology_changed), latest_monitors.len(), u8::from(was_committed && !shell_changed));
        let item_snapshot = crate::commands::refresh_desktop_item_snapshot(&app.state::<crate::shell::DesktopItemState>()).map_err(|error| error.message)?; // 环境重建后重新枚举的 Shell 最终项目集合。
        let workspace_snapshot = crate::commands::reconcile_workspace_environment(&app.state::<crate::workspace::WorkspaceState>(), self, &item_snapshot, shell_changed || topology_changed, &[]).map_err(|error| error.message)?; // 当前环境变化协调后的逐屏工作区快照。
        let _ = app.emit(crate::contracts::DESKTOP_ITEM_SNAPSHOT_CHANGED_EVENT, &item_snapshot);
        let _ = app.emit(crate::contracts::WORKSPACE_SNAPSHOT_CHANGED_EVENT, &workspace_snapshot);
        Ok(())
    }

    /// 宿主或兼容适配器失效时撤下 Desktop Tools 并恢复最近用户设置。
    fn fail_host_adapter(&self, reason: &str)
    {
        let recovery = {
            let mut data = match self.data.lock()
            {
                Ok(data) => data,
                Err(error) =>
                {
                    crate::log_warn!("宿主：运行状态锁已中毒，继续执行适配器失败恢复");
                    error.into_inner()
                }
            };
            if matches!(data.coordinator.phase(), TakeoverPhase::Failed | TakeoverPhase::Restored)
            {
                return;
            }
            data.coordinator.mark_failed();
            data.rebuild_in_progress = false;
            (data.hosts.iter().map(|context| context.host.clone()).collect::<Vec<_>>(), data.shell, data.coordinator.original_items_visible)
        }; // 失败恢复使用的最后宿主、Shell 和用户设置。
        let (hosts, shell, restore_visibility) = recovery;
        let host_hidden = hosts.iter().all(|host| set_host_visible(host.hwnd, false)); // 失败后撤下全部正式宿主的结果。
        let (explorer_restored, environment_verified) = restore_explorer_environment(shell, restore_visibility); // 恢复后实际 Explorer 环境检查。
        crate::log_error!("恢复：宿主适配器进入阻塞状态，原因={}，布局保留=1，宿主隐藏={}，Explorer 恢复={}，环境检查={}", reason, u8::from(host_hidden), u8::from(explorer_restored), u8::from(environment_verified));
    }

    /// 全部门槛通过时切换 Explorer 与正式宿主可见性。
    fn commit_if_ready(&self) -> Result<DesktopHostSnapshot, DesktopHostError>
    {
        // 获得提交权后使用的资源快照。
        let commit = {
            // 等待接管提交权转换的正式宿主状态。
            let mut data = self.data.lock().map_err(|_| DesktopHostError::state_unavailable())?;
            if !all_hosts_ready(&data)
            {
                return Ok(snapshot_from(&data));
            }
            data.coordinator.mark_hosts_ready();
            if !data.coordinator.begin_commit()
            {
                return Ok(snapshot_from(&data));
            }
            (data.hosts.iter().map(|context| (context.host.clone(), context.hit_regions.clone())).collect::<Vec<_>>(), data.shell)
        };
        // 已获得提交权的全部逐屏宿主、Shell 状态与命中图。
        let (hosts, shell) = commit;
        let host_hidden = hosts.iter().all(|(host, _)| set_host_visible(host.hwnd, false)); // 正式宿主切换前的整体隐藏结果。
        // Explorer 桌面项目隐藏结果。
        let explorer_hidden = host_hidden && set_desktop_items_visible(shell.def_view, false);
        if explorer_hidden
        {
            redraw_shell_desktop(shell);
        }
        let host_shown = explorer_hidden && hosts.iter().all(|(host, _)| set_host_visible(host.hwnd, true)); // 全部逐屏宿主接管显示结果。
        let hit_map_applied = host_shown && hosts.iter().all(|(host, hit_regions)| apply_host_hit_regions(host, hit_regions.clone()).is_ok()); // 全部宿主重新应用完整命中图的结果。
        // 四项原生切换步骤的整体提交结果。
        let committed = host_hidden && explorer_hidden && host_shown && hit_map_applied;
        // 等待提交接管结果的正式宿主状态。
        let mut data = self.data.lock().map_err(|_| DesktopHostError::state_unavailable())?;
        if committed
        {
            data.coordinator.mark_committed();
            crate::log_info!("宿主：桌面接管已统一提交，Explorer 桌面项目已隐藏，宿主数量={}", hosts.len());
            return Ok(snapshot_from(&data));
        }

        let _ = set_desktop_items_visible(shell.def_view, shell.original_items_visible);
        redraw_shell_desktop(shell);
        data.coordinator.mark_failed();
        crate::log_error!("宿主：桌面接管提交失败，宿主隐藏：{}，Explorer 隐藏：{}，宿主显示：{}，命中图：{}", u8::from(host_hidden), u8::from(explorer_hidden), u8::from(host_shown), u8::from(hit_map_applied));
        Err(DesktopHostError::platform_failed(&"桌面接管提交失败"))
    }
}

/// 从正式宿主运行状态生成不可变快照。
fn snapshot_from(data: &DesktopHostRuntimeData) -> DesktopHostSnapshot { DesktopHostSnapshot { phase: data.coordinator.phase(), committed: data.coordinator.is_committed(), hosts: data.hosts.iter().map(|context| DesktopHostContextSnapshot { monitor: context.host.monitor.clone(), readiness: context.readiness }).collect() } }

/// 返回指定显示器的只读宿主上下文。
fn host_context<'a>(data: &'a DesktopHostRuntimeData, monitor_id: &str) -> Result<&'a DesktopHostContext, DesktopHostError> { data.hosts.iter().find(|context| context.host.monitor.id == monitor_id).ok_or_else(|| DesktopHostError::invalid_readiness("显示器宿主不存在")) }

/// 返回指定显示器的可变宿主上下文。
fn host_context_mut<'a>(data: &'a mut DesktopHostRuntimeData, monitor_id: &str) -> Result<&'a mut DesktopHostContext, DesktopHostError> { data.hosts.iter_mut().find(|context| context.host.monitor.id == monitor_id).ok_or_else(|| DesktopHostError::invalid_readiness("显示器宿主不存在")) }

/// 判断 guardian 与每台显示器的页面、图像和命中图是否全部就绪；像素仅供诊断。
fn all_hosts_ready(data: &DesktopHostRuntimeData) -> bool { data.coordinator.guardian_ready && !data.hosts.is_empty() && data.hosts.iter().all(|context| context.readiness.page && context.readiness.system_images && context.readiness.hit_map) }

/// 判断 Explorer 是否已经替换桌面宿主身份。
fn shell_hosts_changed(previous: ShellDesktopState, latest: ShellDesktopState) -> bool { previous.def_view != latest.def_view || previous.explorer_host != latest.explorer_host || previous.list_view != latest.list_view }

/// 判断显示器数量、物理几何、工作区或逐屏缩放是否变化。
fn monitor_layout_changed(previous: &[crate::contracts::DesktopHostMonitor], latest: &[crate::contracts::DesktopHostMonitor]) -> bool { previous != latest }

/// 判断显示器身份或物理边界变化是否需要重新协调持久归属。
fn monitor_topology_changed(previous: &[crate::contracts::DesktopHostMonitor], latest: &[crate::contracts::DesktopHostMonitor]) -> bool { previous.len() != latest.len() || previous.iter().zip(latest).any(|(previous, latest)| previous.id != latest.id || previous.primary != latest.primary || previous.x != latest.x || previous.y != latest.y || previous.width != latest.width || previous.height != latest.height) }

/// 判断已提交桌面的 Renderer 是否需要首次或再次轻量重载。
fn renderer_restart_due(committed: bool, heartbeat_age: Duration, retry_age: Option<Duration>) -> bool { committed && heartbeat_age >= RENDERER_HEARTBEAT_TIMEOUT && retry_age.is_none_or(|age| age >= RENDERER_RELOAD_RETRY) }

/// 判断连续平台发现失败是否已经越过恢复等待窗口。
fn environment_failure_is_blocking(failures: u32) -> bool { failures >= ENVIRONMENT_FAILURE_LIMIT }

/// 恢复 Explorer 桌面项目并检查当前窗口环境与目标一致。
fn restore_explorer_environment(shell: ShellDesktopState, visible: bool) -> (bool, bool)
{
    let current_shell = discover_shell_desktop().unwrap_or(shell); // Explorer 重启后优先使用当前桌面层。
    let restored = set_desktop_items_visible(current_shell.def_view, visible); // 当前目标可见性的 Win32 恢复结果。
    redraw_shell_desktop(current_shell);
    let verified = discover_shell_desktop().is_ok_and(|current| desktop_items_visible(current.def_view) == visible); // 恢复后的实际列表窗口状态。
    (restored, verified)
}

/// 校验 React 准备证据与正式宿主工作区一致。
fn validate_frontend_readiness(report: &FrontendHostReadiness, host: &NativeDesktopHost) -> Result<(), DesktopHostError>
{
    // 正式宿主对应的 CSS 视口宽度。
    let expected_width = host.monitor.work_width as f64 / host.monitor.scale_factor;
    // 正式宿主对应的 CSS 视口高度。
    let expected_height = host.monitor.work_height as f64 / host.monitor.scale_factor;
    if (report.viewport_width - expected_width).abs() > 2.0 || (report.viewport_height - expected_height).abs() > 2.0 || (report.device_pixel_ratio - host.monitor.scale_factor).abs() > 0.05
    {
        return Err(DesktopHostError::invalid_readiness("页面视口或设备像素比例与正式宿主工作区不一致"));
    }
    if report.loaded_system_images != report.total_system_images
    {
        return Err(DesktopHostError::invalid_readiness(format!("系统图像仍在加载或包含失败项: {}/{}", report.loaded_system_images, report.total_system_images)));
    }
    validate_hit_regions(&report.hit_regions, expected_width, expected_height)
}

/// 校验前端发布的完整语义命中图。
pub(crate) fn validate_hit_regions(hit_regions: &[HitRegion], viewport_width: f64, viewport_height: f64) -> Result<(), DesktopHostError>
{
    if hit_regions.is_empty()
    {
        return Err(DesktopHostError::invalid_readiness("交互命中图为空"));
    }
    for region in hit_regions
    {
        // 当前区域需要逐项校验的坐标与尺寸。
        let values = [region.left, region.top, region.width, region.height];
        if region.id.is_empty() || region.kind.is_empty() || values.iter().any(|value| !value.is_finite()) || region.left < 0.0 || region.top < 0.0 || region.width <= 0.0 || region.height <= 0.0 || region.left + region.width > viewport_width + 1.0 || region.top + region.height > viewport_height + 1.0
        {
            return Err(DesktopHostError::invalid_readiness(format!("交互命中图包含越界或无效区域: {} [{:.1}, {:.1}, {:.1}, {:.1}]", region.id, region.left, region.top, region.width, region.height)));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests
{
    use super::{DesktopHostContext, DesktopHostRuntimeData, RENDERER_HEARTBEAT_TIMEOUT, RENDERER_RELOAD_RETRY, TakeoverCoordinator, TakeoverPhase, all_hosts_ready, environment_failure_is_blocking, monitor_topology_changed, renderer_restart_due};
    use crate::contracts::{DesktopHostMonitor, DesktopHostReadiness};
    use crate::platform::windows::{NativeDesktopHost, ShellDesktopState};

    /// 验证 guardian 与全部逐屏门槛完成后才允许提交。
    #[test]
    fn takeover_waits_for_guardian_and_every_host()
    {
        // 保存接管前 Explorer 可见状态的协调器。
        let mut coordinator = TakeoverCoordinator::new(true);
        assert!(!coordinator.begin_commit());

        coordinator.mark_guardian_ready();
        coordinator.mark_hosts_ready();
        assert!(coordinator.begin_commit());
        assert_eq!(coordinator.phase(), TakeoverPhase::Committing);
    }

    /// 创建逐屏门槛测试使用的显示器状态。
    fn monitor(id: &str, scale_factor: f64) -> DesktopHostMonitor { DesktopHostMonitor { id: id.to_string(), name: id.to_string(), primary: id == "DISPLAY1", x: if id == "DISPLAY1" { 0 } else { 1920 }, y: 0, width: 1920, height: 1080, work_x: if id == "DISPLAY1" { 0 } else { 1920 }, work_y: 0, work_width: 1920, work_height: 1040, scale_factor, dpi: (96.0 * scale_factor).round() as u32 } }

    /// 验证任一自动逐屏门槛缺失都会阻止统一桌面接管，输入诊断不阻塞提交。
    #[test]
    fn every_host_must_pass_every_automatic_gate()
    {
        let mut coordinator = TakeoverCoordinator::new(true); // 当前全局接管协调器。
        coordinator.mark_guardian_ready();
        let ready = DesktopHostReadiness { guardian: true, page: true, system_images: true, pixels: true, input: true, hit_map: true }; // 当前完整逐屏门槛。
        let mut data = DesktopHostRuntimeData { coordinator, hosts: [monitor("DISPLAY1", 1.0), monitor("DISPLAY2", 1.5)].into_iter().enumerate().map(|(index, monitor)| DesktopHostContext { host: NativeDesktopHost { hwnd: index as isize + 1, monitor }, readiness: ready, hit_regions: Vec::new(), input_drag_active: false, last_renderer_heartbeat: std::time::Instant::now(), renderer_reload_requested_at: None }).collect(), shell: ShellDesktopState { def_view: 1, explorer_host: 1, list_view: 2, original_items_visible: true }, guardian: None, cleanup_complete: false, rebuild_in_progress: false }; // 当前双屏宿主运行状态。
        data.hosts[1].readiness.hit_map = false;
        assert!(!all_hosts_ready(&data));
        data.hosts[1].readiness.hit_map = true;
        data.hosts[1].readiness.input = false;
        data.hosts[1].readiness.pixels = false;
        assert!(all_hosts_ready(&data));
        data.hosts[1].readiness.page = false;
        assert!(!all_hosts_ready(&data));
    }

    /// 验证 DPI-only 变化不被分类为持久显示器拓扑变化。
    #[test]
    fn dpi_only_change_preserves_monitor_topology()
    {
        let previous = [monitor("DISPLAY1", 1.0), monitor("DISPLAY2", 1.25)]; // 变化前显示器环境。
        let latest = [monitor("DISPLAY1", 1.0), monitor("DISPLAY2", 1.5)]; // 仅次屏 DPI 变化后的环境。
        assert!(!monitor_topology_changed(&previous, &latest));

        let disconnected = [monitor("DISPLAY1", 1.0)]; // 次屏断开后的真实拓扑。
        assert!(monitor_topology_changed(&previous, &disconnected));
    }

    /// 验证 Renderer 只在已接管、心跳超时且超过重试间隔时重载。
    #[test]
    fn renderer_restart_waits_for_committed_stale_heartbeat()
    {
        assert!(!renderer_restart_due(false, RENDERER_HEARTBEAT_TIMEOUT, None));
        assert!(renderer_restart_due(true, RENDERER_HEARTBEAT_TIMEOUT, None));
        assert!(!renderer_restart_due(true, RENDERER_HEARTBEAT_TIMEOUT, Some(RENDERER_RELOAD_RETRY - std::time::Duration::from_millis(1))));
        assert!(renderer_restart_due(true, RENDERER_HEARTBEAT_TIMEOUT, Some(RENDERER_RELOAD_RETRY)));
    }

    /// 验证环境瞬时发现失败不会立即撤下宿主，连续失败才进入阻塞恢复。
    #[test]
    fn environment_failure_uses_a_bounded_recovery_window()
    {
        assert!(!environment_failure_is_blocking(1));
        assert!(!environment_failure_is_blocking(19));
        assert!(environment_failure_is_blocking(20));

        let mut coordinator = TakeoverCoordinator::new(true); // 已进入阻塞恢复的宿主协调器。
        coordinator.mark_failed();
        assert_eq!(coordinator.phase(), TakeoverPhase::Failed);
    }

    /// 验证正常退出和异常恢复使用最近观察到的用户图标设置。
    #[test]
    fn recovery_uses_the_latest_observed_user_visibility()
    {
        let mut coordinator = TakeoverCoordinator::new(true); // 启动时桌面图标可见的恢复状态。
        coordinator.observe_user_visibility(false);

        assert!(!coordinator.restore());
        assert_eq!(coordinator.phase(), TakeoverPhase::Restored);
    }
}
