use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use serde_json::json;
use tauri::AppHandle;

use crate::commands::dispatch_native_command;
use crate::contracts::{
    DesktopHostError, DesktopHostReadiness, DesktopHostSnapshot, FrontendHostReadiness, HitRegion,
    ReadinessGate, TakeoverPhase,
};
use crate::platform::windows::{
    NativeCommandHandler, NativeDesktopHost, ShellDesktopState, apply_host_hit_regions,
    create_primary_desktop_host, destroy_primary_desktop_host, discover_shell_desktop,
    load_primary_desktop_host, probe_host_pixels, redraw_shell_desktop, set_desktop_items_visible,
    set_host_visible, suspend_host_hit_regions,
};
use crate::recovery::GuardianProcess;

/// 协调六项门槛、桌面接管提交和 Explorer 恢复阶段。
struct TakeoverCoordinator {
    /// 接管前 Explorer 桌面项目可见状态。
    original_items_visible: bool,
    /// 当前六项接管门槛。
    readiness: DesktopHostReadiness,
    /// 当前接管阶段。
    phase: TakeoverPhase,
}

/// 提供正式宿主启动与恢复状态转换。
impl TakeoverCoordinator {
    /// 创建等待接管门槛的协调器。
    fn new(original_items_visible: bool) -> Self {
        Self {
            original_items_visible,
            readiness: DesktopHostReadiness::default(),
            phase: TakeoverPhase::Starting,
        }
    }

    /// 标记一项启动门槛已经通过。
    fn mark_ready(&mut self, gate: ReadinessGate) {
        match gate {
            ReadinessGate::Guardian => self.readiness.guardian = true,
            ReadinessGate::Page => self.readiness.page = true,
            ReadinessGate::SystemImages => self.readiness.system_images = true,
            ReadinessGate::Pixels => self.readiness.pixels = true,
            ReadinessGate::Input => self.readiness.input = true,
            ReadinessGate::HitMap => self.readiness.hit_map = true,
        }
        if self.pending_gates().is_empty() && self.phase == TakeoverPhase::Starting {
            self.phase = TakeoverPhase::Ready;
        }
    }

    /// 返回仍在阻塞桌面接管的门槛。
    fn pending_gates(&self) -> Vec<ReadinessGate> {
        [
            (ReadinessGate::Guardian, self.readiness.guardian),
            (ReadinessGate::Page, self.readiness.page),
            (ReadinessGate::SystemImages, self.readiness.system_images),
            (ReadinessGate::Pixels, self.readiness.pixels),
            (ReadinessGate::Input, self.readiness.input),
            (ReadinessGate::HitMap, self.readiness.hit_map),
        ]
        .into_iter()
        .filter_map(|(gate, ready)| (!ready).then_some(gate))
        .collect()
    }

    /// 获得本轮桌面接管提交权。
    fn begin_commit(&mut self) -> bool {
        if self.phase != TakeoverPhase::Ready {
            return false;
        }
        self.phase = TakeoverPhase::Committing;
        true
    }

    /// 在启动超时时进入恢复状态并返回原始 Explorer 可见性。
    fn timeout(&mut self) -> Option<bool> {
        if matches!(
            self.phase,
            TakeoverPhase::Committed | TakeoverPhase::Restored
        ) {
            return None;
        }
        self.phase = TakeoverPhase::Restored;
        Some(self.original_items_visible)
    }

    /// 返回当前桌面接管阶段。
    fn phase(&self) -> TakeoverPhase {
        self.phase
    }

    /// 返回桌面项目接管是否已经提交。
    fn is_committed(&self) -> bool {
        self.phase == TakeoverPhase::Committed
    }

    /// 记录 Explorer 隐藏和正式宿主显示已经完成。
    fn mark_committed(&mut self) {
        self.phase = TakeoverPhase::Committed;
    }

    /// 记录桌面接管提交失败。
    fn mark_failed(&mut self) {
        self.phase = TakeoverPhase::Failed;
    }

    /// 进入正常退出恢复阶段并返回原始 Explorer 可见性。
    fn restore(&mut self) -> bool {
        self.phase = TakeoverPhase::Restored;
        self.original_items_visible
    }

    /// 返回当前六项门槛快照。
    fn readiness(&self) -> DesktopHostReadiness {
        self.readiness
    }
}

/// 正式宿主持续持有的协调状态与原生资源记录。
struct DesktopHostRuntimeData {
    /// 接管门槛与阶段协调器。
    coordinator: TakeoverCoordinator,
    /// 单显示器正式宿主记录。
    host: NativeDesktopHost,
    /// 接管前 Explorer 桌面状态。
    shell: ShellDesktopState,
    /// 独立恢复进程。
    guardian: Option<GuardianProcess>,
    /// 最近一次完整语义命中图。
    hit_regions: Vec<HitRegion>,
    /// 正常退出资源释放是否已经执行。
    cleanup_complete: bool,
}

/// 持有单显示器透明宿主、接管门槛和 Explorer 恢复契约。
pub struct DesktopHostRuntime {
    /// 跨命令和 watchdog 共享的正式宿主状态。
    data: Mutex<DesktopHostRuntimeData>,
}

/// 提供正式宿主启动、命中图发布、接管和恢复能力。
impl DesktopHostRuntime {
    /// 创建正式宿主、启动 guardian 并导航 React 页面。
    pub fn start(app: &AppHandle) -> Result<Arc<Self>, String> {
        // 接管前 Explorer 桌面层状态。
        let shell = discover_shell_desktop()?;
        // guardian 可执行文件路径。
        let guardian_path = std::env::current_exe()
            .map_err(|error| format!("读取主程序路径失败: {error}"))?
            .parent()
            .ok_or_else(|| "主程序路径缺少父目录".to_string())?
            .join("desktop-tools-guardian.exe");
        if !guardian_path.is_file() {
            return Err(format!(
                "缺少 guardian 可执行文件: {}",
                guardian_path.display()
            ));
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
        // 主显示器正式 Windowed WebView2 宿主。
        let host = create_primary_desktop_host(app, shell, command_handler)?;
        // 保存接管前 Explorer 可见性的门槛协调器。
        let mut coordinator = TakeoverCoordinator::new(shell.original_items_visible);
        coordinator.mark_ready(ReadinessGate::Guardian);
        // 正式宿主共享运行时。
        let runtime = Arc::new(Self {
            data: Mutex::new(DesktopHostRuntimeData {
                coordinator,
                host,
                shell,
                guardian: Some(guardian),
                hit_regions: Vec::new(),
                cleanup_complete: false,
            }),
        });
        runtime_slot
            .set(Arc::clone(&runtime))
            .map_err(|_| "正式宿主运行时重复初始化".to_string())?;
        load_primary_desktop_host()?;
        crate::log_info!("宿主：等待页面、系统图像、像素、输入和命中图门槛");
        Ok(runtime)
    }

    /// 返回当前正式宿主不可变快照。
    pub fn snapshot(&self) -> Result<DesktopHostSnapshot, DesktopHostError> {
        // 当前正式宿主共享状态。
        let data = self
            .data
            .lock()
            .map_err(|_| DesktopHostError::state_unavailable())?;
        Ok(snapshot_from(&data))
    }

    /// 校验并提交 React 双帧、系统图像和交互命中图证据。
    pub fn publish_frontend_readiness(
        &self,
        report: FrontendHostReadiness,
    ) -> Result<DesktopHostSnapshot, DesktopHostError> {
        // 当前正式宿主记录。
        let host = {
            // 等待前端证据校验的正式宿主状态。
            let data = self
                .data
                .lock()
                .map_err(|_| DesktopHostError::state_unavailable())?;
            validate_frontend_readiness(&report, &data.host)?;
            data.host.clone()
        };
        // 已应用到 HWND 的语义区域数量。
        let region_count = apply_host_hit_regions(&host, report.hit_regions.clone())
            .map_err(|error| DesktopHostError::platform_failed(&error))?;
        // PrintWindow 检测到的真实渲染像素数量。
        let mut visible_pixels = None;
        for _ in 0..10 {
            match probe_host_pixels(&host) {
                Ok(count) => {
                    visible_pixels = Some(count);
                    break;
                }
                Err(_) => thread::sleep(Duration::from_millis(100)),
            }
        }
        // 最终通过探针的可见像素数量。
        let visible_pixels = visible_pixels.ok_or_else(|| {
            DesktopHostError::platform_failed(&"正式宿主像素探针未检测到渲染内容")
        })?;

        // 日志中用于验证输入坐标的首个语义命中区域。
        let first_region = report
            .hit_regions
            .first()
            .map(|region| {
                format!(
                    "{}:[{:.0}, {:.0}, {:.0}, {:.0}]",
                    region.id, region.left, region.top, region.width, region.height
                )
            })
            .unwrap_or_else(|| "无".to_string());
        // 前端门槛更新后的正式宿主快照。
        let snapshot = {
            // 等待前端证据写入的正式宿主状态。
            let mut data = self
                .data
                .lock()
                .map_err(|_| DesktopHostError::state_unavailable())?;
            data.hit_regions = report.hit_regions;
            data.coordinator.mark_ready(ReadinessGate::Page);
            data.coordinator.mark_ready(ReadinessGate::SystemImages);
            data.coordinator.mark_ready(ReadinessGate::Pixels);
            data.coordinator.mark_ready(ReadinessGate::HitMap);
            snapshot_from(&data)
        };
        crate::log_info!(
            "宿主：前端门槛已通过，命中区域：{}，首个区域：{}，系统图像：{}/{}，真实像素：{}",
            region_count,
            first_region,
            report.loaded_system_images,
            report.total_system_images,
            visible_pixels
        );
        Ok(snapshot)
    }

    /// 原子替换页面布局变化后的完整语义命中图。
    pub fn update_hit_map(
        &self,
        hit_regions: Vec<HitRegion>,
    ) -> Result<DesktopHostSnapshot, DesktopHostError> {
        // 当前正式宿主与 CSS 视口尺寸。
        let host = {
            // 等待命中图校验的正式宿主状态。
            let data = self
                .data
                .lock()
                .map_err(|_| DesktopHostError::state_unavailable())?;
            validate_hit_regions(
                &hit_regions,
                data.host.monitor.work_width as f64 / data.host.monitor.scale_factor,
                data.host.monitor.work_height as f64 / data.host.monitor.scale_factor,
            )?;
            data.host.clone()
        };
        apply_host_hit_regions(&host, hit_regions.clone())
            .map_err(|error| DesktopHostError::platform_failed(&error))?;
        // 等待替换命中图的正式宿主状态。
        let mut data = self
            .data
            .lock()
            .map_err(|_| DesktopHostError::state_unavailable())?;
        data.hit_regions = hit_regions;
        Ok(snapshot_from(&data))
    }

    /// 在连续拖动期间暂停语义命中区域裁剪。
    pub fn begin_input_drag(&self) -> Result<DesktopHostSnapshot, DesktopHostError> {
        // 当前拖动所属的正式宿主记录。
        let host = self
            .data
            .lock()
            .map_err(|_| DesktopHostError::state_unavailable())?
            .host
            .clone();
        suspend_host_hit_regions(&host)
            .map_err(|error| DesktopHostError::platform_failed(&error))?;
        crate::log_debug!("宿主：连续拖动命中区域已暂停，窗口句柄={:#x}", host.hwnd);
        self.snapshot()
    }

    /// 结束连续拖动并恢复最近一份完整语义命中图。
    pub fn end_input_drag(&self) -> Result<DesktopHostSnapshot, DesktopHostError> {
        // 等待恢复的正式宿主和最近命中图。
        let (host, hit_regions) = {
            // 当前受锁保护的正式宿主状态。
            let data = self
                .data
                .lock()
                .map_err(|_| DesktopHostError::state_unavailable())?;
            (data.host.clone(), data.hit_regions.clone())
        };
        apply_host_hit_regions(&host, hit_regions)
            .map_err(|error| DesktopHostError::platform_failed(&error))?;
        crate::log_debug!("宿主：连续拖动命中区域已恢复，窗口句柄={:#x}", host.hwnd);
        self.snapshot()
    }

    /// 记录 React 项目点击回传并在全部门槛满足时提交接管。
    pub fn record_input_probe(
        &self,
        item_id: &str,
    ) -> Result<DesktopHostSnapshot, DesktopHostError> {
        {
            // 等待输入门槛写入的正式宿主状态。
            let mut data = self
                .data
                .lock()
                .map_err(|_| DesktopHostError::state_unavailable())?;
            if !data
                .hit_regions
                .iter()
                .any(|region| region.kind == "desktop-item" && region.id == item_id)
            {
                return Err(DesktopHostError::invalid_readiness(
                    "输入探针未命中已发布的桌面项目区域",
                ));
            }
            data.coordinator.mark_ready(ReadinessGate::Input);
        }
        crate::log_info!("宿主：最小输入探针已回传，项目引用：{item_id}");
        self.commit_if_ready()
    }

    /// 在 30 秒启动门槛超时后恢复 Explorer 状态。
    pub fn timeout(&self) -> Result<bool, DesktopHostError> {
        // 超时前的正式宿主、Shell 与原始可见性。
        let (host, shell, original_visibility, pending) = {
            // 等待超时恢复转换的正式宿主状态。
            let mut data = self.data.lock().unwrap_or_else(|error| {
                crate::log_warn!("宿主：运行状态锁已中毒，继续执行超时恢复");
                error.into_inner()
            });
            // 超时时仍未满足的接管门槛。
            let pending = data.coordinator.pending_gates();
            // 超时恢复使用的接管前 Explorer 可见状态。
            let Some(original_visibility) = data.coordinator.timeout() else {
                return Ok(false);
            };
            (data.host.clone(), data.shell, original_visibility, pending)
        };
        // 超时后撤下正式宿主的结果。
        let host_hidden = set_host_visible(host.hwnd, false);
        // 超时后恢复 Explorer 桌面项目的结果。
        let explorer_restored = set_desktop_items_visible(shell.def_view, original_visibility);
        redraw_shell_desktop(shell);
        crate::log_error!(
            "宿主：启动门槛超时，缺失门槛：{pending:?}，宿主隐藏：{}，Explorer 恢复：{}",
            u8::from(host_hidden),
            u8::from(explorer_restored)
        );
        Ok(true)
    }

    /// 正常退出时恢复 Explorer、停止 guardian 并销毁正式宿主。
    pub fn cleanup(&self) {
        // 正常恢复所需的资源快照。
        let cleanup = {
            // 等待正常恢复转换的正式宿主状态。
            let mut data = match self.data.lock() {
                Ok(data) => data,
                Err(error) => {
                    crate::log_warn!("宿主：运行状态锁已中毒，继续执行退出恢复");
                    error.into_inner()
                }
            };
            if data.cleanup_complete {
                return;
            }
            data.cleanup_complete = true;
            // 正常退出恢复使用的接管前 Explorer 可见状态。
            let original_visibility = data.coordinator.restore();
            (
                data.host.clone(),
                data.shell,
                original_visibility,
                data.guardian.take(),
            )
        };
        // 正常恢复使用的正式宿主、Shell 状态和 guardian。
        let (host, original_shell, original_visibility, guardian) = cleanup;
        // 正常退出时撤下正式宿主的结果。
        let host_hidden = set_host_visible(host.hwnd, false);
        // Explorer 重启后优先恢复当前 DefView。
        let current_shell = discover_shell_desktop().unwrap_or(original_shell);
        // 正常退出时恢复 Explorer 桌面项目的结果。
        let restored = set_desktop_items_visible(current_shell.def_view, original_visibility);
        redraw_shell_desktop(current_shell);
        crate::log_info!(
            "宿主：正常退出恢复，宿主隐藏：{}，Explorer 恢复：{}",
            u8::from(host_hidden),
            u8::from(restored)
        );
        if restored {
            if let Some(mut guardian) = guardian {
                guardian.shutdown();
            }
        } else if let Some(guardian) = guardian {
            crate::log_error!("恢复：Explorer 恢复失败，guardian 继续监视主进程退出");
            guardian.continue_recovery();
        }
        destroy_primary_desktop_host();
    }

    /// 启动正式宿主 30 秒准备 watchdog。
    pub fn start_watchdog(app: AppHandle, runtime: Arc<Self>) {
        thread::spawn(move || {
            thread::sleep(Duration::from_secs(30));
            if runtime.timeout().unwrap_or(false) {
                app.exit(9);
            }
        });
    }

    /// 全部门槛通过时切换 Explorer 与正式宿主可见性。
    fn commit_if_ready(&self) -> Result<DesktopHostSnapshot, DesktopHostError> {
        // 获得提交权后使用的资源快照。
        let commit = {
            // 等待接管提交权转换的正式宿主状态。
            let mut data = self
                .data
                .lock()
                .map_err(|_| DesktopHostError::state_unavailable())?;
            if !data.coordinator.begin_commit() {
                return Ok(snapshot_from(&data));
            }
            (data.host.clone(), data.shell, data.hit_regions.clone())
        };
        // 已获得提交权的正式宿主、Shell 状态与命中图。
        let (host, shell, hit_regions) = commit;
        // 正式宿主切换前的隐藏结果。
        let host_hidden = set_host_visible(host.hwnd, false);
        // Explorer 桌面项目隐藏结果。
        let explorer_hidden = host_hidden && set_desktop_items_visible(shell.def_view, false);
        if explorer_hidden {
            redraw_shell_desktop(shell);
        }
        // 正式宿主接管显示结果。
        let host_shown = explorer_hidden && set_host_visible(host.hwnd, true);
        // 接管显示后重新应用的完整命中图结果。
        let hit_map_applied = host_shown && apply_host_hit_regions(&host, hit_regions).is_ok();
        // 四项原生切换步骤的整体提交结果。
        let committed = host_hidden && explorer_hidden && host_shown && hit_map_applied;
        // 等待提交接管结果的正式宿主状态。
        let mut data = self
            .data
            .lock()
            .map_err(|_| DesktopHostError::state_unavailable())?;
        if committed {
            data.coordinator.mark_committed();
            crate::log_info!("宿主：桌面接管已提交，Explorer 桌面项目已隐藏");
            return Ok(snapshot_from(&data));
        }

        let _ = set_desktop_items_visible(shell.def_view, shell.original_items_visible);
        redraw_shell_desktop(shell);
        data.coordinator.mark_failed();
        crate::log_error!(
            "宿主：桌面接管提交失败，宿主隐藏：{}，Explorer 隐藏：{}，宿主显示：{}，命中图：{}",
            u8::from(host_hidden),
            u8::from(explorer_hidden),
            u8::from(host_shown),
            u8::from(hit_map_applied)
        );
        Err(DesktopHostError::platform_failed(&"桌面接管提交失败"))
    }
}

/// 从正式宿主运行状态生成不可变快照。
fn snapshot_from(data: &DesktopHostRuntimeData) -> DesktopHostSnapshot {
    DesktopHostSnapshot {
        phase: data.coordinator.phase(),
        committed: data.coordinator.is_committed(),
        monitor: data.host.monitor.clone(),
        readiness: data.coordinator.readiness(),
    }
}

/// 校验 React 准备证据与正式宿主工作区一致。
fn validate_frontend_readiness(
    report: &FrontendHostReadiness,
    host: &NativeDesktopHost,
) -> Result<(), DesktopHostError> {
    // 正式宿主对应的 CSS 视口宽度。
    let expected_width = host.monitor.work_width as f64 / host.monitor.scale_factor;
    // 正式宿主对应的 CSS 视口高度。
    let expected_height = host.monitor.work_height as f64 / host.monitor.scale_factor;
    if (report.viewport_width - expected_width).abs() > 2.0
        || (report.viewport_height - expected_height).abs() > 2.0
        || (report.device_pixel_ratio - host.monitor.scale_factor).abs() > 0.05
    {
        return Err(DesktopHostError::invalid_readiness(
            "页面视口或设备像素比例与正式宿主工作区不一致",
        ));
    }
    if report.loaded_system_images != report.total_system_images {
        return Err(DesktopHostError::invalid_readiness(format!(
            "系统图像仍在加载或包含失败项: {}/{}",
            report.loaded_system_images, report.total_system_images
        )));
    }
    validate_hit_regions(&report.hit_regions, expected_width, expected_height)
}

/// 校验前端发布的完整语义命中图。
fn validate_hit_regions(
    hit_regions: &[HitRegion],
    viewport_width: f64,
    viewport_height: f64,
) -> Result<(), DesktopHostError> {
    if hit_regions.is_empty() {
        return Err(DesktopHostError::invalid_readiness("交互命中图为空"));
    }
    for region in hit_regions {
        // 当前区域需要逐项校验的坐标与尺寸。
        let values = [region.left, region.top, region.width, region.height];
        if region.id.is_empty()
            || region.kind.is_empty()
            || values.iter().any(|value| !value.is_finite())
            || region.left < 0.0
            || region.top < 0.0
            || region.width <= 0.0
            || region.height <= 0.0
            || region.left + region.width > viewport_width + 1.0
            || region.top + region.height > viewport_height + 1.0
        {
            return Err(DesktopHostError::invalid_readiness(format!(
                "交互命中图包含越界或无效区域: {} [{:.1}, {:.1}, {:.1}, {:.1}]",
                region.id, region.left, region.top, region.width, region.height
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{ReadinessGate, TakeoverCoordinator, TakeoverPhase};

    /// 验证六项接管门槛完整通过后才允许提交。
    #[test]
    fn takeover_waits_for_every_readiness_gate() {
        // 保存接管前 Explorer 可见状态的协调器。
        let mut coordinator = TakeoverCoordinator::new(true);
        for gate in [
            ReadinessGate::Guardian,
            ReadinessGate::Page,
            ReadinessGate::SystemImages,
            ReadinessGate::Pixels,
            ReadinessGate::HitMap,
        ] {
            coordinator.mark_ready(gate);
        }

        assert_eq!(coordinator.pending_gates(), vec![ReadinessGate::Input]);
        assert!(!coordinator.begin_commit());

        coordinator.mark_ready(ReadinessGate::Input);
        assert!(coordinator.begin_commit());
        assert_eq!(coordinator.phase(), TakeoverPhase::Committing);
    }

    /// 验证启动超时保持接管前 Explorer 状态并进入恢复阶段。
    #[test]
    fn startup_timeout_restores_the_original_explorer_state() {
        // 保存接管前 Explorer 隐藏状态的协调器。
        let mut coordinator = TakeoverCoordinator::new(false);
        coordinator.mark_ready(ReadinessGate::Guardian);

        assert_eq!(coordinator.timeout(), Some(false));
        assert_eq!(coordinator.phase(), TakeoverPhase::Restored);
        assert!(!coordinator.is_committed());
    }
}
