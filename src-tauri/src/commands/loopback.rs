use std::sync::Arc;

use tauri::{AppHandle, Emitter, Manager, State};
use zeroize::Zeroize;

use crate::component_runtime::ComponentRuntimeState;
use crate::component_runtime::loopback::{LoopbackNotifier, event_kind};
use crate::contracts::{COMPONENT_HOST_EVENT, ComponentCapability, ComponentRuntimeError, LoopbackChannelHandle, LoopbackChannelMode, LoopbackRequest, LoopbackResponse, RegisterLoopbackChannel};
use crate::workspace::WorkspaceState;

/// 插件在实例内注册一条本机 loopback 通道。
#[tauri::command]
pub fn register_loopback_channel(app: AppHandle, state: State<'_, ComponentRuntimeState>, workspace: State<'_, WorkspaceState>, instance_id: String, channel_id: String, mode: LoopbackChannelMode, port: Option<u16>, base_url: Option<String>, credential_ref: Option<String>, header_name: Option<String>) -> Result<LoopbackChannelHandle, ComponentRuntimeError> { execute_register_loopback_channel(&app, &state, &workspace, RegisterLoopbackChannel { instance_id, channel_id, mode, port, base_url, credential_ref, header_name }) }

/// Tauri 与本地 transport 共用的通道注册执行。
pub(crate) fn execute_register_loopback_channel(app: &AppHandle, state: &ComponentRuntimeState, workspace: &WorkspaceState, request: RegisterLoopbackChannel) -> Result<LoopbackChannelHandle, ComponentRuntimeError>
{
    let workspace = workspace.snapshot().map_err(|error| ComponentRuntimeError::invalid_instance(error.message))?; // 统一工作区的实例来源。
    let registration = state.authorize_capability(&workspace.components, &request.instance_id, ComponentCapability::LoopbackChannel)?;
    state.loopback().set_notifier(loopback_notifier(app.clone())); // 缓冲更新后经宿主事件通知插件。
    state.loopback().register(request, &registration.component_id)
}

/// 读走通道缓冲里尚未消费的原始请求。
#[tauri::command]
pub fn read_loopback_channel(state: State<'_, ComponentRuntimeState>, workspace: State<'_, WorkspaceState>, instance_id: String, channel_id: String, after_sequence: Option<u64>) -> Result<Vec<LoopbackRequest>, ComponentRuntimeError> { execute_read_loopback_channel(&state, &workspace, instance_id, channel_id, after_sequence) }

/// Tauri 与本地 transport 共用的通道读取执行。
pub(crate) fn execute_read_loopback_channel(state: &ComponentRuntimeState, workspace: &WorkspaceState, instance_id: String, channel_id: String, after_sequence: Option<u64>) -> Result<Vec<LoopbackRequest>, ComponentRuntimeError>
{
    let workspace = workspace.snapshot().map_err(|error| ComponentRuntimeError::invalid_instance(error.message))?;
    state.authorize_capability(&workspace.components, &instance_id, ComponentCapability::LoopbackChannel)?;
    state.loopback().read(&instance_id, &channel_id, after_sequence)
}

/// 经 request 通道向对端发一次原始请求。
#[tauri::command]
pub fn request_loopback(state: State<'_, ComponentRuntimeState>, workspace: State<'_, WorkspaceState>, instance_id: String, channel_id: String, method: String, path: String, body: Option<String>) -> Result<LoopbackResponse, ComponentRuntimeError> { execute_request_loopback(&state, &workspace, instance_id, channel_id, method, path, body) }

/// Tauri 与本地 transport 共用的出站请求执行。
pub(crate) fn execute_request_loopback(state: &ComponentRuntimeState, workspace: &WorkspaceState, instance_id: String, channel_id: String, method: String, path: String, body: Option<String>) -> Result<LoopbackResponse, ComponentRuntimeError>
{
    let workspace = workspace.snapshot().map_err(|error| ComponentRuntimeError::invalid_instance(error.message))?;
    state.authorize_capability(&workspace.components, &instance_id, ComponentCapability::LoopbackChannel)?;
    state.loopback().request(&instance_id, &channel_id, &method, &path, body)
}

/// 缓冲更新时用既有宿主事件通知，事件本身不携带请求内容。
fn loopback_notifier(app: AppHandle) -> LoopbackNotifier
{
    Arc::new(move |channel_id: &str| {
        let Some(state) = app.try_state::<ComponentRuntimeState>()
        else
        {
            return;
        };
        let event = state.publish_host_event(&event_kind(channel_id)); // 与时间事件共用序号与最近事件槽。
        if let Err(error) = app.emit(COMPONENT_HOST_EVENT, event)
        {
            crate::log_warn!("loopback：通道数据事件发送失败，通道={channel_id}，错误={error}"); // 只记通道标识，不记请求内容。
        }
    })
}

/// 宿主命令允许读写的凭据目标前缀；其它程序的目标名一律拒绝。
pub(crate) const CREDENTIAL_TARGET_PREFIX: &str = "DesktopTools/";

/// 目标名总长度上限，避免异常超长目标。
const CREDENTIAL_TARGET_LIMIT: usize = 256;

/// 把用户粘贴的数据源共享密钥写入 Windows Credential Manager。
#[tauri::command]
pub fn set_loopback_credential(target: String, secret: String) -> Result<(), ComponentRuntimeError> { execute_set_loopback_credential(target, secret) }

/// Tauri 与本地 transport 共用的凭据写入执行；明文用完立即清零，不返回、不记录。
pub(crate) fn execute_set_loopback_credential(target: String, secret: String) -> Result<(), ComponentRuntimeError>
{
    let target = validate_credential_target(&target)?;
    validate_credential_secret(&secret)?;
    let mut blob = secret.into_bytes(); // 明文只留在这一层缓冲区。
    let result = crate::platform::windows::credentials::write_secret(&target, &mut blob).map_err(|_| ComponentRuntimeError::loopback_credential_store_failed("Windows 凭据保存失败，请稍后重试"));
    blob.zeroize();
    result?;
    crate::log_info!("loopback：共享密钥已写入凭据存储，目标={target}");
    Ok(())
}

/// 清除指定目标的共享密钥；记录本就不存在时视为成功。
#[tauri::command]
pub fn clear_loopback_credential(target: String) -> Result<(), ComponentRuntimeError> { execute_clear_loopback_credential(target) }

/// Tauri 与本地 transport 共用的凭据清除执行。
pub(crate) fn execute_clear_loopback_credential(target: String) -> Result<(), ComponentRuntimeError>
{
    let target = validate_credential_target(&target)?;
    crate::platform::windows::credentials::clear_secret(&target).map_err(|_| ComponentRuntimeError::loopback_credential_store_failed("Windows 凭据清除失败，请稍后重试"))?;
    crate::log_info!("loopback：共享密钥已清除，目标={target}");
    Ok(())
}

/// 凭据目标只接受受控前缀，拒绝任意其它程序的目标名。
fn validate_credential_target(target: &str) -> Result<String, ComponentRuntimeError>
{
    let target = target.trim();
    if !target.starts_with(CREDENTIAL_TARGET_PREFIX) || target.len() > CREDENTIAL_TARGET_LIMIT || target[CREDENTIAL_TARGET_PREFIX.len()..].is_empty() || target.chars().any(char::is_control)
    {
        return Err(ComponentRuntimeError::loopback_credential_invalid("凭据目标只能使用 DesktopTools/ 开头的受控名称"));
    }
    Ok(target.to_string())
}

/// 共享密钥必须有界且不含控制字符，避免写进凭据后污染 HTTP 头。
fn validate_credential_secret(secret: &str) -> Result<(), ComponentRuntimeError>
{
    let bytes = secret.as_bytes();
    if bytes.is_empty() || bytes.len() > crate::platform::windows::credentials::MAX_SECRET_BYTES
    {
        return Err(ComponentRuntimeError::loopback_credential_invalid("共享密钥长度必须在 1–2560 字节之间"));
    }
    if bytes.iter().any(u8::is_ascii_control)
    {
        return Err(ComponentRuntimeError::loopback_credential_invalid("共享密钥不能包含控制字符"));
    }
    Ok(())
}

#[cfg(test)]
#[path = "loopback.test.rs"]
mod tests;
