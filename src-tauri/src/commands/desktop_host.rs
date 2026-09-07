use std::sync::Arc;

use tauri::{AppHandle, Emitter, State};

use crate::contracts::{
    DESKTOP_HOST_SNAPSHOT_CHANGED_EVENT, DesktopHostError, DesktopHostSnapshot,
    FrontendHostReadiness, HitRegion,
};
use crate::host::DesktopHostRuntime;

/// 返回正式单显示器宿主当前权威快照。
#[tauri::command]
pub fn get_desktop_host_snapshot(
    state: State<'_, Arc<DesktopHostRuntime>>,
) -> Result<DesktopHostSnapshot, DesktopHostError> {
    read_desktop_host_snapshot(&state)
}

/// 响应右上角测试面板的显式退出操作。
#[tauri::command]
pub fn exit_desktop_tools(app: AppHandle) {
    request_exit(&app);
}

/// 通过 transport 共享的应用句柄请求正常退出。
pub(crate) fn request_exit(app: &AppHandle) {
    crate::log_info!("应用：测试面板请求正常退出");
    app.exit(0);
}

/// 校验并提交 React 页面、系统图像和命中图准备证据。
#[tauri::command]
pub fn publish_desktop_host_readiness(
    app: AppHandle,
    state: State<'_, Arc<DesktopHostRuntime>>,
    report: FrontendHostReadiness,
) -> Result<DesktopHostSnapshot, DesktopHostError> {
    // 已提交前端准备证据的正式宿主快照。
    let snapshot = publish_readiness(&state, report)?;
    emit_snapshot(&app, &snapshot);
    Ok(snapshot)
}

/// 返回 transport 共享的正式宿主当前权威快照。
pub(crate) fn read_desktop_host_snapshot(
    runtime: &DesktopHostRuntime,
) -> Result<DesktopHostSnapshot, DesktopHostError> {
    runtime.snapshot()
}

/// 记录逐屏 Renderer 心跳，供自动轻量重载 watchdog 使用。
#[tauri::command]
pub fn record_desktop_host_renderer_heartbeat(
    state: State<'_, Arc<DesktopHostRuntime>>,
    monitor_id: String,
) -> Result<(), DesktopHostError> {
    record_renderer_heartbeat(&state, &monitor_id)
}

/// 记录 transport 共享的逐屏 Renderer 心跳。
pub(crate) fn record_renderer_heartbeat(
    runtime: &DesktopHostRuntime,
    monitor_id: &str,
) -> Result<(), DesktopHostError> {
    runtime.record_renderer_heartbeat(monitor_id)
}

/// 校验前端准备证据并发布 transport 共享的正式宿主快照。
pub(crate) fn publish_readiness(
    runtime: &DesktopHostRuntime,
    report: FrontendHostReadiness,
) -> Result<DesktopHostSnapshot, DesktopHostError> {
    runtime.publish_frontend_readiness(report)
}

/// 原子替换页面布局变化后的完整语义命中图。
#[tauri::command]
pub fn update_desktop_host_hit_map(
    app: AppHandle,
    state: State<'_, Arc<DesktopHostRuntime>>,
    monitor_id: String,
    hit_regions: Vec<HitRegion>,
) -> Result<DesktopHostSnapshot, DesktopHostError> {
    // 已应用命中图的正式宿主快照。
    let snapshot = replace_hit_map(&state, &monitor_id, hit_regions)?;
    emit_snapshot(&app, &snapshot);
    Ok(snapshot)
}

/// 暂停正式宿主命中裁剪以承接连续指针拖动。
#[tauri::command]
pub fn begin_desktop_host_input_drag(
    app: AppHandle,
    state: State<'_, Arc<DesktopHostRuntime>>,
    monitor_id: String,
) -> Result<DesktopHostSnapshot, DesktopHostError> {
    // 已暂停命中裁剪的正式宿主快照。
    let snapshot = begin_input_drag(&state, &monitor_id)?;
    emit_snapshot(&app, &snapshot);
    Ok(snapshot)
}

/// 恢复正式宿主最近一份完整语义命中图。
#[tauri::command]
pub fn end_desktop_host_input_drag(
    app: AppHandle,
    state: State<'_, Arc<DesktopHostRuntime>>,
    monitor_id: String,
) -> Result<DesktopHostSnapshot, DesktopHostError> {
    // 已恢复命中图的正式宿主快照。
    let snapshot = end_input_drag(&state, &monitor_id)?;
    emit_snapshot(&app, &snapshot);
    Ok(snapshot)
}

/// 替换命中图并发布 transport 共享的正式宿主快照。
pub(crate) fn replace_hit_map(
    runtime: &DesktopHostRuntime,
    monitor_id: &str,
    hit_regions: Vec<HitRegion>,
) -> Result<DesktopHostSnapshot, DesktopHostError> {
    runtime.update_hit_map(monitor_id, hit_regions)
}

/// 暂停 transport 共享的正式宿主命中裁剪。
pub(crate) fn begin_input_drag(
    runtime: &DesktopHostRuntime,
    monitor_id: &str,
) -> Result<DesktopHostSnapshot, DesktopHostError> {
    runtime.begin_input_drag(monitor_id)
}

/// 恢复 transport 共享的正式宿主命中图。
pub(crate) fn end_input_drag(
    runtime: &DesktopHostRuntime,
    monitor_id: &str,
) -> Result<DesktopHostSnapshot, DesktopHostError> {
    runtime.end_input_drag(monitor_id)
}

/// 记录最小桌面项目输入回传并尝试提交接管。
#[tauri::command]
pub fn record_desktop_host_input_probe(
    app: AppHandle,
    state: State<'_, Arc<DesktopHostRuntime>>,
    monitor_id: String,
    item_id: String,
) -> Result<DesktopHostSnapshot, DesktopHostError> {
    // 已记录输入诊断状态的正式宿主快照。
    let snapshot = record_input_probe(&state, &monitor_id, &item_id)?;
    emit_snapshot(&app, &snapshot);
    Ok(snapshot)
}

/// 记录输入证据并发布 transport 共享的正式宿主快照。
pub(crate) fn record_input_probe(
    runtime: &DesktopHostRuntime,
    monitor_id: &str,
    item_id: &str,
) -> Result<DesktopHostSnapshot, DesktopHostError> {
    runtime.record_input_probe(monitor_id, item_id)
}

/// 发布正式宿主快照并记录事件发布失败。
fn emit_snapshot(app: &AppHandle, snapshot: &DesktopHostSnapshot) {
    if let Err(error) = app.emit(DESKTOP_HOST_SNAPSHOT_CHANGED_EVENT, snapshot) {
        crate::log_error!("宿主：快照事件发布失败，错误：{error}");
    }
}
