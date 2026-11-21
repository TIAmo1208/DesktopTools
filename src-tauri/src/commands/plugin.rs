//! 插件通用命令：凭据写入/清除/兑换、浮岛消息投递与协议白名单打开。
//!
//! 四条命令都按实例能力授权：工作区实例与岛上合成实例走同一条合并校验，
//! 组件身份只由运行时解析，不接受前端传入；任何返回值、事件与日志都不含秘密。

use std::io::Read;
use std::path::Path;
use std::time::Duration;

use chrono::Utc;
use tauri::{AppHandle, Emitter, State};
use zeroize::Zeroize;

use crate::component_runtime::ComponentRuntimeState;
use crate::contracts::{ComponentCapability, ComponentRuntimeError, ISLAND_PLUGIN_MESSAGE_EVENT, IslandMessageKind, IslandPluginMessage, PluginCredentialExchangeResult, valid_credential_slot};
use crate::preferences::SettingsState;
use crate::workspace::WorkspaceState;

/// 凭据槽位上限：只允许 1–32 位小写字母、数字与连字符。
const SLOT_LIMIT: usize = 32;

/// 插件消息键上限（字符）；超长直接拒绝，不截断身份。
const KEY_LIMIT: usize = 128;

/// 插件消息正文上限（字符）；超长直接拒绝，不截断。
const TEXT_LIMIT: usize = 200;

/// 允许交给系统 Shell 的 URI 协议白名单。
const ALLOWED_URI_SCHEMES: &[&str] = &["dsh:", "https:"];

/// DSH 的进程可执行文件名；协议处理器同名，抬窗与测试共用这一处常量。
pub(crate) const DSH_PROCESS_NAME: &str = "DeepSeek Harness.exe";

/// 凭据目标前缀；插件只能落在自己的命名空间里。
const PLUGIN_CREDENTIAL_PREFIX: &str = "DesktopTools/plugin/";

/// 宿主兑换的请求超时；连接与总超时都受它约束。
const EXCHANGE_TIMEOUT: Duration = Duration::from_secs(5);

/// 宿主兑换请求体上限：只承载配对码一类的小载荷。
const EXCHANGE_BODY_LIMIT: usize = 8 * 1024;

/// 宿主兑换响应体上限：64 KiB，超出整次兑换失败。
const EXCHANGE_RESPONSE_LIMIT: usize = 64 * 1024;

/// 插件在自有命名空间写入一条凭据；宿主永不回读明文。
#[tauri::command]
pub fn plugin_credential_set(state: State<'_, ComponentRuntimeState>, workspace: State<'_, WorkspaceState>, settings: State<'_, SettingsState>, instance_id: String, slot: String, value: String) -> Result<(), ComponentRuntimeError> { execute_plugin_credential_set(&state, &workspace, &settings, instance_id, slot, value) }

/// Tauri 与本地 transport 共用的凭据写入执行。
pub(crate) fn execute_plugin_credential_set(state: &ComponentRuntimeState, workspace: &WorkspaceState, settings: &SettingsState, instance_id: String, slot: String, value: String) -> Result<(), ComponentRuntimeError>
{
    let component_id = authorize(state, workspace, settings, &instance_id, ComponentCapability::PluginCredential)?;
    let target = plugin_credential_target(&component_id, &slot)?;
    validate_secret(&value)?;
    let mut blob = value.into_bytes(); // 明文只留在这一层缓冲区。
    let result = crate::platform::windows::credentials::write_secret(&target, &mut blob).map_err(|_| ComponentRuntimeError::plugin_credential_failed("Windows 凭据保存失败，请稍后重试"));
    blob.zeroize();
    result?;
    state.loopback().invalidate_credential(&target); // 引用旧秘密的缓存通道必须重新注册。
    crate::log_info!("插件凭据：已写入，目标={target}"); // 只记目标名，不记内容。
    Ok(())
}

/// 插件清除自己命名空间里的一条凭据；记录不存在视为成功。
#[tauri::command]
pub fn plugin_credential_clear(state: State<'_, ComponentRuntimeState>, workspace: State<'_, WorkspaceState>, settings: State<'_, SettingsState>, instance_id: String, slot: String) -> Result<(), ComponentRuntimeError> { execute_plugin_credential_clear(&state, &workspace, &settings, instance_id, slot) }

/// Tauri 与本地 transport 共用的凭据清除执行。
pub(crate) fn execute_plugin_credential_clear(state: &ComponentRuntimeState, workspace: &WorkspaceState, settings: &SettingsState, instance_id: String, slot: String) -> Result<(), ComponentRuntimeError>
{
    let component_id = authorize(state, workspace, settings, &instance_id, ComponentCapability::PluginCredential)?;
    let target = plugin_credential_target(&component_id, &slot)?;
    crate::platform::windows::credentials::clear_secret(&target).map_err(|_| ComponentRuntimeError::plugin_credential_failed("Windows 凭据清除失败，请稍后重试"))?;
    state.loopback().invalidate_credential(&target);
    crate::log_info!("插件凭据：已清除，目标={target}");
    Ok(())
}

/// 宿主向本机回环地址兑换一次并把秘密直接落槽；成功只返回 `{ stored: true }`。
///
/// 响应正文与提取出的秘密都不进入返回值、事件或日志，失败时也不落槽。
#[tauri::command]
pub fn plugin_credential_exchange(state: State<'_, ComponentRuntimeState>, workspace: State<'_, WorkspaceState>, settings: State<'_, SettingsState>, instance_id: String, slot: String, url: String, body: Option<serde_json::Value>, extract: String) -> Result<PluginCredentialExchangeResult, ComponentRuntimeError> { execute_plugin_credential_exchange(&state, &workspace, &settings, instance_id, slot, url, body, extract) }

/// Tauri 与本地 transport 共用的兑换执行。
pub(crate) fn execute_plugin_credential_exchange(state: &ComponentRuntimeState, workspace: &WorkspaceState, settings: &SettingsState, instance_id: String, slot: String, url: String, body: Option<serde_json::Value>, extract: String) -> Result<PluginCredentialExchangeResult, ComponentRuntimeError>
{
    let component_id = authorize(state, workspace, settings, &instance_id, ComponentCapability::PluginCredentialExchange)?;
    let target = plugin_credential_target(&component_id, &slot)?;
    let endpoint = parse_exchange_endpoint(&url)?;
    let body = match body
    {
        Some(value) => Some(serde_json::to_string(&value).map_err(|_| ComponentRuntimeError::invalid_input("兑换请求体不是合法 JSON"))?),
        None => None, // 插件没给正文时不发送 body，保持与调用方意图一致。
    };
    if let Some(serialized) = body.as_deref()
    {
        validate_exchange_body(serialized)?;
    }
    let path = parse_extract_path(&extract)?;
    let client = reqwest::blocking::Client::builder().timeout(EXCHANGE_TIMEOUT).connect_timeout(EXCHANGE_TIMEOUT).no_proxy().redirect(reqwest::redirect::Policy::none()).build().map_err(|_| ComponentRuntimeError::exchange_failed("兑换请求客户端创建失败"))?;
    let mut request = client.post(&endpoint.url).header("Content-Type", "application/json");
    if let Some(body) = body
    {
        request = request.body(body);
    }
    let response = request.send().map_err(|_| ComponentRuntimeError::exchange_failed("兑换请求未送达或超时"))?;
    // 禁重定向之外，再确认响应仍来自最初的本机端点。
    if response.url().host_str() != Some("127.0.0.1") || response.url().port_or_known_default() != Some(endpoint.port)
    {
        return Err(ComponentRuntimeError::exchange_failed("兑换响应地址与请求地址不一致"));
    }
    let status = response.status().as_u16();
    if !(200..300).contains(&status)
    {
        return Err(ComponentRuntimeError::exchange_failed(format!("兑换请求返回状态 {status}")));
    }
    let mut bytes = Vec::new();
    response.take((EXCHANGE_RESPONSE_LIMIT + 1) as u64).read_to_end(&mut bytes).map_err(|_| ComponentRuntimeError::exchange_failed("读取兑换响应失败"))?;
    if bytes.len() > EXCHANGE_RESPONSE_LIMIT
    {
        return Err(ComponentRuntimeError::exchange_failed(format!("兑换响应超过 {EXCHANGE_RESPONSE_LIMIT} 字节上限")));
    }
    let value = serde_json::from_slice::<serde_json::Value>(&bytes).map_err(|_| ComponentRuntimeError::exchange_failed("兑换响应不是合法 JSON"))?;
    let secret = extract_secret(&value, &path).ok_or_else(|| ComponentRuntimeError::exchange_failed("兑换响应里没有可用的秘密字段"))?;
    validate_secret(&secret)?;
    let mut blob = secret.into_bytes();
    let result = crate::platform::windows::credentials::write_secret(&target, &mut blob).map_err(|_| ComponentRuntimeError::plugin_credential_failed("Windows 凭据保存失败，请稍后重试"));
    blob.zeroize();
    result?;
    state.loopback().invalidate_credential(&target);
    crate::log_info!("插件凭据：兑换成功并已落槽，目标={target}，状态={status}"); // 不记 URL、正文或秘密。
    Ok(PluginCredentialExchangeResult { stored: true })
}

/// 插件向浮岛消息锚点投递一条本地条目；运行时只发事件，不保存队列、不弹原生通知。
#[tauri::command]
pub fn publish_island_message(app: AppHandle, state: State<'_, ComponentRuntimeState>, workspace: State<'_, WorkspaceState>, settings: State<'_, SettingsState>, instance_id: String, key: String, kind: IslandMessageKind, text: String, uri: Option<String>) -> Result<(), ComponentRuntimeError> { execute_publish_island_message(&app, &state, &workspace, &settings, instance_id, key, kind, text, uri) }

/// Tauri 与本地 transport 共用的消息投递执行。
pub(crate) fn execute_publish_island_message(app: &AppHandle, state: &ComponentRuntimeState, workspace: &WorkspaceState, settings: &SettingsState, instance_id: String, key: String, kind: IslandMessageKind, text: String, uri: Option<String>) -> Result<(), ComponentRuntimeError>
{
    let component_id = authorize(state, workspace, settings, &instance_id, ComponentCapability::PublishIslandMessage)?;
    let key = sanitize_key(&key)?;
    let text = sanitize_text(&text)?;
    let uri = sanitize_message_uri(uri.as_deref())?; // 非法跳转地址整条拒绝，不做降级。
    let message = IslandPluginMessage { source: component_id, key, kind, text, unix_millis: Utc::now().timestamp_millis(), uri };
    app.emit(ISLAND_PLUGIN_MESSAGE_EVENT, &message).map_err(|error| ComponentRuntimeError { code: "island-message-failed".to_string(), message: format!("浮岛消息投递失败：{error}") })
}

/// 按插件声明的能力打开协议白名单内的 URI；其余协议一律拒绝。
#[tauri::command]
pub fn open_uri(state: State<'_, ComponentRuntimeState>, workspace: State<'_, WorkspaceState>, settings: State<'_, SettingsState>, instance_id: String, uri: String) -> Result<(), ComponentRuntimeError> { execute_open_uri(&state, &workspace, &settings, instance_id, uri) }

/// Tauri 与本地 transport 共用的打开执行。
pub(crate) fn execute_open_uri(state: &ComponentRuntimeState, workspace: &WorkspaceState, settings: &SettingsState, instance_id: String, uri: String) -> Result<(), ComponentRuntimeError>
{
    authorize(state, workspace, settings, &instance_id, ComponentCapability::OpenUri)?;
    open_uri_with(&uri, &open_shell_target, &activate_target)
}

/// 交系统 Shell 打开原始链接；`dsh:` 由注册的协议处理器接手。
fn open_shell_target(uri: &str) -> Result<(), String> { crate::platform::windows::settings::open_path(Path::new(uri)) }

/// 把目标进程的主窗口带到前台；`dsh:` 协议本身不会激活已存在的 DSH 窗口。
fn activate_target(process_name: &str) -> Result<(), String> { crate::platform::windows::foreground::activate_window_by_process_name(process_name) }

/// URI 是否使用 `dsh:` 协议；协议名大小写不敏感，只看冒号前的部分。
pub(crate) fn is_dsh_uri(uri: &str) -> bool { uri.split_once(':').is_some_and(|(scheme, _)| scheme.eq_ignore_ascii_case("dsh")) }

/// 打开一个已通过白名单的 URI：Shell 打开与（仅 `dsh:`）原生抬窗任一成功即算成功。
///
/// 两个端口都由调用方注入，便于测试覆盖「协议打开失败但抬窗成功」等组合；
/// 失败日志只记协议与原因，不写完整链接。
fn open_uri_with(uri: &str, open_target: &dyn Fn(&str) -> Result<(), String>, activate: &dyn Fn(&str) -> Result<(), String>) -> Result<(), ComponentRuntimeError>
{
    let uri = uri.trim();
    if !allowed_uri(uri)
    {
        return Err(ComponentRuntimeError::uri_scheme_rejected());
    }
    let protocol = uri.split(':').next().unwrap_or_default();
    let shell = open_target(uri);
    let dsh = is_dsh_uri(uri);
    let raised = if dsh { activate(DSH_PROCESS_NAME) } else { Err("不是 dsh: 协议，无需抬窗".to_string()) };
    if shell.is_ok() || (dsh && raised.is_ok())
    {
        if dsh && raised.is_err()
        {
            crate::log_debug!("插件 URI：已交给系统打开，但未能把 DSH 窗口带到前台，协议={protocol}"); // 不记链接内容。
        }
        return Ok(());
    }
    // Shell 的错误文案里可能带完整路径或查询串，这里只保留协议与「是否 dsh」，不外带链接内容。
    crate::log_warn!("插件 URI：打开失败，协议={protocol}，dsh={dsh}，Shell失败={}，抬窗失败={}", shell.is_err(), raised.is_err()); // 不记链接内容。
    Err(ComponentRuntimeError::uri_open_failed("无法打开链接"))
}

/// 解析实例的组件身份并确认已声明该能力；组件 id 只由运行时给出。
fn authorize(state: &ComponentRuntimeState, workspace: &WorkspaceState, settings: &SettingsState, instance_id: &str, capability: ComponentCapability) -> Result<String, ComponentRuntimeError>
{
    let instances = super::component::merged_instances(state, workspace, settings)?;
    let registration = state.authorize_capability_on(&instances, instance_id, capability)?;
    Ok(registration.component_id)
}

/// 组装插件凭据目标：`DesktopTools/plugin/<componentId>/<slot>`。
fn plugin_credential_target(component_id: &str, slot: &str) -> Result<String, ComponentRuntimeError>
{
    let slot = slot.trim();
    if slot.chars().count() > SLOT_LIMIT || !valid_credential_slot(slot)
    {
        return Err(ComponentRuntimeError::invalid_input("凭据槽位只能使用 1–32 位小写字母、数字与连字符"));
    }
    Ok(format!("{PLUGIN_CREDENTIAL_PREFIX}{component_id}/{slot}"))
}

/// 凭据值必须有界且不含控制字符，和通用 loopback 的密钥规则保持一致。
fn validate_secret(value: &str) -> Result<(), ComponentRuntimeError>
{
    let bytes = value.as_bytes();
    if bytes.is_empty() || bytes.len() > crate::platform::windows::credentials::MAX_SECRET_BYTES
    {
        return Err(ComponentRuntimeError::invalid_input("凭据值长度必须在 1–2560 字节之间"));
    }
    if bytes.iter().any(u8::is_ascii_control)
    {
        return Err(ComponentRuntimeError::invalid_input("凭据值不能包含控制字符"));
    }
    Ok(())
}

/// 消息跳转地址上限（字符）。
const MESSAGE_URI_LIMIT: usize = 512;

/// 可选的消息跳转地址：去空白后只接受 `dsh:` 与 `https:`，长度 ≤512，不含控制字符；缺省返回 `None`。
///
/// 地址不合法时整条消息拒绝（`uri-scheme-rejected`），不静默降级成无跳转条目。
fn sanitize_message_uri(uri: Option<&str>) -> Result<Option<String>, ComponentRuntimeError>
{
    let Some(uri) = uri.map(str::trim).filter(|value| !value.is_empty())
    else
    {
        return Ok(None);
    };
    if uri.chars().count() > MESSAGE_URI_LIMIT || uri.chars().any(char::is_control) || !allowed_uri(uri)
    {
        return Err(ComponentRuntimeError::uri_scheme_rejected());
    }
    Ok(Some(uri.to_string()))
}

/// 只接受白名单协议；前缀比较不区分大小写，与系统注册表的处理一致。
fn allowed_uri(uri: &str) -> bool
{
    if uri.is_empty() || uri.chars().any(char::is_control)
    {
        return false;
    }
    let Some((scheme, _)) = uri.split_once(':')
    else
    {
        return false;
    };
    let scheme = format!("{}:", scheme.to_ascii_lowercase());
    ALLOWED_URI_SCHEMES.contains(&scheme.as_str())
}

/// 兑换目标：只接受 `http://127.0.0.1:<port>/<path>` 字面量，不接受其它主机、协议、片段或控制字符。
#[derive(Clone, Debug, Eq, PartialEq)]
struct ExchangeEndpoint
{
    /// 归一化后的请求地址。
    url: String,
    /// 端口，用于响应地址一致性校验。
    port: u16,
}

/// 解析兑换目标；失败时只回结构化错误，不含 URL 细节以外的秘密（URL 本身不是秘密）。
fn parse_exchange_endpoint(value: &str) -> Result<ExchangeEndpoint, ComponentRuntimeError>
{
    let value = value.trim();
    if value.is_empty() || value.chars().any(char::is_control) || value.contains('#')
    {
        return Err(ComponentRuntimeError::exchange_target_invalid("兑换地址无效"));
    }
    let Some(rest) = value.get(..7).filter(|prefix| prefix.eq_ignore_ascii_case("http://")).map(|_| &value[7..])
    else
    {
        return Err(ComponentRuntimeError::exchange_target_invalid("兑换地址必须是 http://127.0.0.1:<端口>/<路径>"));
    };
    let (authority, path) = match rest.split_once('/')
    {
        Some((authority, _)) => (authority, &rest[authority.len()..]),
        None => (rest, "/"),
    };
    let Some((host, port)) = authority.split_once(':')
    else
    {
        return Err(ComponentRuntimeError::exchange_target_invalid("兑换地址缺少端口"));
    };
    if host != "127.0.0.1"
    {
        return Err(ComponentRuntimeError::exchange_target_invalid("兑换地址只允许字面量 127.0.0.1"));
    }
    let port = port.parse::<u16>().ok().filter(|port| *port > 0).ok_or_else(|| ComponentRuntimeError::exchange_target_invalid("兑换地址端口必须在 1–65535"))?;
    if path.len() > 2048 || path.contains("//") || path.contains('\\')
    {
        return Err(ComponentRuntimeError::exchange_target_invalid("兑换地址路径无效"));
    }
    Ok(ExchangeEndpoint { url: format!("http://127.0.0.1:{port}{path}"), port })
}

/// 兑换请求体必须是有限大小的 JSON 对象。
fn validate_exchange_body(body: &str) -> Result<(), ComponentRuntimeError>
{
    if body.is_empty() || body.len() > EXCHANGE_BODY_LIMIT
    {
        return Err(ComponentRuntimeError::invalid_input(format!("兑换请求体长度必须在 1–{EXCHANGE_BODY_LIMIT} 字节之间")));
    }
    if body.bytes().any(|byte| byte.is_ascii_control() && !matches!(byte, b'\t' | b'\n' | b'\r'))
    {
        return Err(ComponentRuntimeError::invalid_input("兑换请求体不能包含控制字符"));
    }
    serde_json::from_str::<serde_json::Value>(body).map_err(|_| ComponentRuntimeError::invalid_input("兑换请求体不是合法 JSON"))?;
    Ok(())
}

/// 提取路径：点分字段名，每段只允许字母、数字、下划线与连字符。
fn parse_extract_path(extract: &str) -> Result<Vec<String>, ComponentRuntimeError>
{
    let segments = extract.trim().split('.').map(str::to_string).collect::<Vec<_>>();
    if segments.is_empty() || segments.len() > 8 || segments.iter().any(|segment| segment.is_empty() || segment.len() > 64 || !segment.chars().all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-')))
    {
        return Err(ComponentRuntimeError::invalid_input("提取字段名无效"));
    }
    Ok(segments)
}

/// 按路径读取字符串秘密；缺失、非字符串或空串都按失败处理，避免落一个空槽。
fn extract_secret(value: &serde_json::Value, path: &[String]) -> Option<String>
{
    let mut current = value;
    for segment in path
    {
        current = current.get(segment.as_str())?;
    }
    current.as_str().filter(|secret| !secret.is_empty()).map(str::to_string)
}

/// 消息键：保留完整身份，超长或含控制字符直接拒绝，不截断。
fn sanitize_key(key: &str) -> Result<String, ComponentRuntimeError>
{
    let key = key.trim();
    if key.is_empty()
    {
        return Err(ComponentRuntimeError::invalid_input("插件消息必须带非空 key"));
    }
    if key.chars().count() > KEY_LIMIT || key.chars().any(char::is_control)
    {
        return Err(ComponentRuntimeError::invalid_input(format!("插件消息 key 不能超过 {KEY_LIMIT} 字符且不能包含控制字符")));
    }
    Ok(key.to_string())
}

/// 正文：去控制字符后按字符计数；空文本与超长都拒绝，避免前端与宿主对截断规则理解不一致。
fn sanitize_text(text: &str) -> Result<String, ComponentRuntimeError>
{
    let cleaned = text.chars().filter(|character| !character.is_control()).collect::<String>();
    let cleaned = cleaned.trim();
    if cleaned.is_empty()
    {
        return Err(ComponentRuntimeError::invalid_input("插件消息正文不能为空"));
    }
    if cleaned.chars().count() > TEXT_LIMIT
    {
        return Err(ComponentRuntimeError::invalid_input(format!("插件消息正文不能超过 {TEXT_LIMIT} 字符")));
    }
    Ok(cleaned.to_string())
}

#[cfg(test)]
mod tests
{
    use super::*;

    #[test]
    fn credential_targets_are_confined_to_the_plugin_namespace()
    {
        assert_eq!(plugin_credential_target("plugin.dsh", "token").expect("合法槽位"), "DesktopTools/plugin/plugin.dsh/token");
        for slot in ["", " ", "token!", "Token", "token/two", "tok en", &"a".repeat(33)]
        {
            assert_eq!(plugin_credential_target("plugin.dsh", slot).expect_err("非法槽位").code, "component-invalid-input", "{slot:?}");
        }
    }

    #[test]
    fn secrets_are_bounded_and_control_free()
    {
        assert!(validate_secret("super-secret-token-value").is_ok());
        assert!(validate_secret("").is_err());
        assert!(validate_secret("bad\ntoken").is_err());
        assert!(validate_secret(&"x".repeat(2561)).is_err());
    }

    #[test]
    fn uri_whitelist_accepts_only_dsh_and_https()
    {
        for allowed in ["dsh://open", "dsh:open?session=1", "https://example.com/a", "HTTPS://example.com"]
        {
            assert!(allowed_uri(allowed), "{allowed} 应被允许");
        }
        for rejected in ["http://example.com", "file:///c:/windows", "javascript:alert(1)", "ms-settings:", "", "dsh", "dsh:\n"]
        {
            assert!(!allowed_uri(rejected), "{rejected:?} 应被拒绝");
        }
    }

    #[test]
    fn exchange_endpoint_accepts_only_literal_loopback_http()
    {
        assert_eq!(parse_exchange_endpoint("http://127.0.0.1:47832/pair").expect("合法地址"), ExchangeEndpoint { url: "http://127.0.0.1:47832/pair".to_string(), port: 47832 });
        assert_eq!(parse_exchange_endpoint("http://127.0.0.1:47832").expect("无路径按根处理").url, "http://127.0.0.1:47832/");
        for rejected in ["https://127.0.0.1:47832/pair", "http://localhost:47832/pair", "http://10.0.0.1:47832/pair", "http://127.0.0.1/pair", "http://127.0.0.1:0/pair", "http://127.0.0.1:47832/pair#x", "http://127.0.0.1:47832//pair", "", "http://127.0.0.1:47832/pa\nth"]
        {
            assert_eq!(parse_exchange_endpoint(rejected).unwrap_err().code, "exchange-target-invalid", "{rejected:?}");
        }
    }

    #[test]
    fn exchange_body_and_extract_are_bounded()
    {
        assert!(validate_exchange_body("{\"code\":\"DEV-CODE\"}").is_ok());
        assert!(validate_exchange_body("not-json").is_err());
        assert!(validate_exchange_body("").is_err());
        assert!(parse_extract_path("token").is_ok());
        assert!(parse_extract_path("data.token").is_ok());
        for rejected in ["", "a..b", "data.token!", "a b", &"x".repeat(65)]
        {
            assert!(parse_extract_path(rejected).is_err(), "{rejected:?}");
        }
        let value = serde_json::json!({ "ok": true, "token": "secret-value", "data": { "token": "nested-secret" } });
        assert_eq!(extract_secret(&value, &["token".to_string()]), Some("secret-value".to_string()));
        assert_eq!(extract_secret(&value, &["data".to_string(), "token".to_string()]), Some("nested-secret".to_string()));
        assert_eq!(extract_secret(&value, &["missing".to_string()]), None);
        assert_eq!(extract_secret(&serde_json::json!({ "token": "" }), &["token".to_string()]), None);
    }

    #[test]
    fn message_text_is_cleaned_and_rejected_instead_of_truncated()
    {
        assert_eq!(sanitize_text("  hello\nworld  ").expect("合法正文"), "helloworld");
        assert_eq!(sanitize_text("字".repeat(TEXT_LIMIT).as_str()).expect("等长允许").chars().count(), TEXT_LIMIT);
        assert_eq!(sanitize_text(&"字".repeat(TEXT_LIMIT + 1)).unwrap_err().code, "component-invalid-input", "超长拒绝而不是截断");
        assert_eq!(sanitize_text("\n\t ").unwrap_err().code, "component-invalid-input");
        assert_eq!(sanitize_key(&"k".repeat(KEY_LIMIT)).expect("等长允许"), "k".repeat(KEY_LIMIT));
        assert_eq!(sanitize_key(&"k".repeat(KEY_LIMIT + 1)).unwrap_err().code, "component-invalid-input", "key 不截断");
        assert_eq!(sanitize_key(" ").unwrap_err().code, "component-invalid-input");
    }
}

/// 兑换结果只有 `stored`，消息载荷带可信来源组件与 camelCase 时间。
#[test]
fn exchange_result_and_message_payload_carry_only_safe_fields()
{
    assert_eq!(serde_json::to_value(PluginCredentialExchangeResult { stored: true }).unwrap(), serde_json::json!({ "stored": true }));
    let message = IslandPluginMessage { source: "plugin.dsh".to_string(), key: "pairing".to_string(), kind: IslandMessageKind::Waiting, text: "等待配对".to_string(), unix_millis: 1_700_000_000_000, uri: None };
    let value = serde_json::to_value(&message).unwrap();
    assert_eq!(value, serde_json::json!({ "source": "plugin.dsh", "key": "pairing", "kind": "waiting", "text": "等待配对", "unixMillis": 1_700_000_000_000_i64 }));
    assert!(!value.to_string().contains("token"), "投递载荷不含凭据字段");
    let with_uri = IslandPluginMessage { uri: Some("dsh://open".to_string()), ..message.clone() };
    assert_eq!(serde_json::to_value(&with_uri).unwrap()["uri"], "dsh://open", "提供跳转地址时进入载荷");
    // 未提供跳转地址时字段整体省略，旧前端按 5 个字段解析仍然成立。
    assert!(serde_json::to_value(&message).unwrap().get("uri").is_none());
}

/// 消息跳转地址：只接受 dsh/https、限长 512、禁控制字符，缺省省略。
#[test]
fn message_uri_is_limited_to_the_scheme_whitelist()
{
    assert_eq!(sanitize_message_uri(Some("dsh://open")).expect("dsh 允许"), Some("dsh://open".to_string()));
    assert_eq!(sanitize_message_uri(Some("  https://example.com/a?b=1  ")).expect("https 允许"), Some("https://example.com/a?b=1".to_string()));
    assert_eq!(sanitize_message_uri(Some("HTTPS://example.com")).expect("协议大小写不敏感"), Some("HTTPS://example.com".to_string()));
    // 去空白只修剪两端；内部出现控制字符则整条拒绝。
    assert_eq!(sanitize_message_uri(Some(" dsh://open\n")).expect("两端空白与控制字符可修剪"), Some("dsh://open".to_string()));
    for rejected in ["file:///c:/windows", "javascript:alert(1)", "http://example.com", "ms-settings:", "dsh://open\nx", "https://example.com/a\tb"]
    {
        assert_eq!(sanitize_message_uri(Some(rejected)).expect_err("非法地址整条拒绝").code, "uri-scheme-rejected", "{rejected:?}");
    }
    let longest = format!("https://example.com/{}", "a".repeat(MESSAGE_URI_LIMIT - "https://example.com/".len()));
    assert_eq!(sanitize_message_uri(Some(&longest)).expect("等长允许").map(|value| value.chars().count()), Some(MESSAGE_URI_LIMIT));
    let too_long = format!("{longest}a");
    assert_eq!(sanitize_message_uri(Some(&too_long)).expect_err("超长拒绝").code, "uri-scheme-rejected");
    for absent in [None, Some(""), Some("   ")]
    {
        assert_eq!(sanitize_message_uri(absent).expect("缺省允许"), None, "未提供时不进入载荷");
    }
}

/// dsh 协议判定：只认 `dsh:`，协议名大小写不敏感，其它协议一律 false。
#[test]
fn dsh_uri_detection_is_scheme_only()
{
    for dsh in ["dsh://open", "dsh:open", "DSH://OPEN?session=1"]
    {
        assert!(is_dsh_uri(dsh), "{dsh:?} 应识别为 dsh 协议");
    }
    for other in ["https://example.com", "http://example.com", "file:///c:/x", "javascript:alert(1)", "", "dsh", "ms-dsh:open"]
    {
        assert!(!is_dsh_uri(other), "{other:?} 不应识别为 dsh 协议");
    }
}

/// 打开结果的判定矩阵：协议与抬窗任一成功即成功；非 dsh 协议不调用抬窗；两者都失败才报错。
#[test]
fn open_uri_succeeds_when_either_shell_or_activation_works()
{
    use std::cell::Cell;
    let shell_ok = |_: &str| Ok(());
    let shell_err = |_: &str| Err("关联程序缺失".to_string());
    let activate_ok = |_: &str| Ok(());
    let activate_err = |_: &str| Err("未找到窗口".to_string());
    assert!(open_uri_with("dsh://open", &shell_ok, &activate_err).is_ok(), "Shell 成功即可");
    assert!(open_uri_with("dsh://open", &shell_err, &activate_ok).is_ok(), "抬窗成功也可");
    assert_eq!(open_uri_with("dsh://open", &shell_err, &activate_err).unwrap_err().code, "uri-open-failed", "两者都失败必须报错");
    assert_eq!(open_uri_with("file:///c:/windows", &shell_ok, &activate_ok).unwrap_err().code, "uri-scheme-rejected", "白名单外仍拒绝");
    let https_calls = Cell::new(0);
    let counting_activate = |_: &str| {
        https_calls.set(https_calls.get() + 1);
        Ok(())
    };
    assert!(open_uri_with("https://example.com/a", &shell_ok, &counting_activate).is_ok(), "https 交给系统即可");
    assert_eq!(https_calls.get(), 0, "非 dsh 协议不得触发抬窗");
    assert_eq!(open_uri_with("https://example.com/a", &shell_err, &counting_activate).unwrap_err().code, "uri-open-failed", "https 失败按 Shell 失败处理");
    let echo_shell = |value: &str| Err(format!("无法打开路径={value}"));
    let leak = open_uri_with("https://example.com/secret?token=abc", &echo_shell, &counting_activate).unwrap_err();
    assert!(!leak.message.contains("example.com") && !leak.message.contains("token"), "失败信息不得带链接内容：{}", leak.message);
    let dsh_calls = Cell::new(0);
    let counting_dsh_activate = |process: &str| {
        assert_eq!(process, DSH_PROCESS_NAME, "抬窗必须使用集中常量里的 DSH 进程名");
        dsh_calls.set(dsh_calls.get() + 1);
        Err("未找到窗口".to_string())
    };
    assert!(open_uri_with("dsh:open", &shell_ok, &counting_dsh_activate).is_ok());
    assert_eq!(dsh_calls.get(), 1, "dsh 协议在 Shell 打开后仍会尝试抬窗");
}
