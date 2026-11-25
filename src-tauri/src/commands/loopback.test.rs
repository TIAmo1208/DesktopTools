use super::{execute_clear_loopback_credential, execute_set_loopback_credential, validate_credential_secret, validate_legacy_credential_target};
use crate::component_runtime::ComponentRuntimeState;
use crate::component_runtime::loopback::LoopbackRegistry;
use crate::contracts::{CredentialTarget, LoopbackChannelHandle, LoopbackChannelMode, RegisterLoopbackChannel, classify_credential_target};

/// 每个用例独占的临时插件目录，避免读取仓库插件目录。
fn runtime() -> ComponentRuntimeState
{
    let directory = std::env::temp_dir().join(format!("desktop-tools-loopback-command-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|value| value.as_nanos()).unwrap_or(0)));
    ComponentRuntimeState::new(&directory)
}

/// 按插件声明的 target + headerName 注册一条 request 通道，触发宿主解析凭据。
fn register_request_channel(target: &str, component_id: &str) -> Result<LoopbackChannelHandle, crate::contracts::ComponentRuntimeError>
{
    let request = RegisterLoopbackChannel { instance_id: "instance-1".to_string(), channel_id: "status".to_string(), mode: LoopbackChannelMode::Request, port: None, base_url: Some("http://127.0.0.1:8766".to_string()), credential_ref: Some(target.to_string()), header_name: Some("X-Status-Token".to_string()), credential_prefix: None };
    LoopbackRegistry::new().register(request, component_id)
}

/// 旧写入/清除只保留 AstrBot 的兼容目标；插件命名空间（含大小写变体）在访问凭据存储前就被拒。
#[test]
fn legacy_commands_only_accept_the_astrbot_target()
{
    assert_eq!(validate_legacy_credential_target(" DesktopTools/astrbot-status ").unwrap(), "DesktopTools/astrbot-status");
    assert_eq!(validate_legacy_credential_target("desktoptools/AstrBot-Status").unwrap(), "DesktopTools/astrbot-status", "Windows 凭据目标名不区分大小写");
    for plugin_target in ["DesktopTools/plugin/plugin.alpha/token", "DesktopTools/PLUGIN/plugin.alpha/token", "desktoptools/Plugin/plugin.alpha/token"]
    {
        assert_eq!(validate_legacy_credential_target(plugin_target).unwrap_err().code, "credential-target-denied", "{plugin_target}");
    }
    for reserved in ["", "Other/x", "DesktopTools/other", "DesktopTools/Test/LoopbackCredential/x", "DesktopTools/plugin/", "DesktopTools/plugin/plugin.alpha"]
    {
        assert_eq!(validate_legacy_credential_target(reserved).unwrap_err().code, "loopback-credential-invalid", "{reserved:?}");
    }
    let state = runtime();
    for plugin_target in ["DesktopTools/plugin/plugin.alpha/token", "desktoptools/Plugin/plugin.alpha/token"]
    {
        assert_eq!(execute_set_loopback_credential(&state, plugin_target.to_string(), "token-123".to_string()).unwrap_err().code, "credential-target-denied", "旧写入命令必须拒绝插件命名空间");
        assert_eq!(execute_clear_loopback_credential(&state, plugin_target.to_string()).unwrap_err().code, "credential-target-denied", "旧清除命令必须拒绝插件命名空间");
    }
}

/// 密钥边界只做纯校验，测试不写入真实凭据。
#[test]
fn credential_secret_is_bounded_and_control_free()
{
    assert!(validate_credential_secret("super-secret-token-value").is_ok());
    assert!(validate_credential_secret("").is_err());
    assert!(validate_credential_secret("bad\ntoken").is_err());
    assert!(validate_credential_secret(&"x".repeat(2561)).is_err());
}

/// 目标分类是唯一判断点：插件命名空间按所有者区分，旧目标只剩 AstrBot 一条，其余一律保留。
#[test]
fn credential_targets_are_classified_by_one_case_insensitive_rule()
{
    assert_eq!(classify_credential_target("DesktopTools/astrbot-status"), CredentialTarget::Legacy);
    assert_eq!(classify_credential_target("DESKTOOPTOOLS/astrbot-status"), CredentialTarget::Reserved);
    assert!(matches!(classify_credential_target("desktoptools/PLUGIN/plugin.alpha/token"), CredentialTarget::Plugin { plugin_id: "plugin.alpha", slot: "token" }));
    assert!(matches!(classify_credential_target("DesktopTools/plugin/plugin.alpha/token"), CredentialTarget::Plugin { .. }));
    assert_eq!(classify_credential_target("DesktopTools/plugin/plugin.beta/token"), CredentialTarget::Plugin { plugin_id: "plugin.beta", slot: "token" });
    assert_eq!(classify_credential_target("DesktopTools/plugin/plugin.alpha"), CredentialTarget::Reserved);
    assert_eq!(classify_credential_target("DesktopTools/plugin/plugin.alpha/a/b"), CredentialTarget::Reserved);
    assert_eq!(classify_credential_target("DesktopTools/other"), CredentialTarget::Reserved);
}

/// 通道注册只接受本插件自己的命名空间或旧白名单；跨插件与保留目标在读取凭据前被拒。
#[test]
fn channel_credential_refs_reject_cross_plugin_and_reserved_targets()
{
    assert_eq!(register_request_channel("DesktopTools/plugin/plugin.beta/token", "plugin.alpha").unwrap_err().code, "credential-target-denied", "不能引用其它插件命名空间");
    assert_eq!(register_request_channel("DesktopTools/PLUGIN/plugin.beta/token", "plugin.alpha").unwrap_err().code, "credential-target-denied", "大小写变体同样拒绝");
    assert_eq!(register_request_channel("DesktopTools/other", "plugin.alpha").unwrap_err().code, "credential-target-denied", "未许可的旧目标同样拒绝");
    // 自己的命名空间会走到凭据读取；测试目录里没有该记录，因此报不可用而不是越权。
    assert_eq!(register_request_channel("DesktopTools/plugin/plugin.alpha/token", "plugin.alpha").unwrap_err().code, "loopback-credential-unavailable");
}
