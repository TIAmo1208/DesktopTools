use super::{execute_clear_loopback_credential, execute_set_loopback_credential, validate_credential_secret, validate_credential_target};
use crate::component_runtime::loopback::LoopbackRegistry;
use crate::contracts::{ComponentRuntimeError, LoopbackChannelHandle, LoopbackChannelMode, RegisterLoopbackChannel};

/// 每个用例独占的目标名，避免与真实凭据或其它测试互相影响。
fn test_target(suffix: &str) -> String { format!("DesktopTools/Test/LoopbackCredential/{}-{suffix}", std::process::id()) }

/// 按插件声明的 target + headerName 注册一条 request 通道，触发宿主读取凭据。
fn register_request_channel(target: &str) -> Result<LoopbackChannelHandle, ComponentRuntimeError>
{
    let request = RegisterLoopbackChannel { instance_id: "instance-1".to_string(), channel_id: "status".to_string(), mode: LoopbackChannelMode::Request, port: None, base_url: Some("http://127.0.0.1:8766".to_string()), credential_ref: Some(target.to_string()), header_name: Some("X-Status-Token".to_string()) };
    LoopbackRegistry::new().register(request, "plugin.astrbot")
}

/// 目标只接受受控前缀，密钥有界且不含控制字符。
#[test]
fn credential_inputs_only_accept_the_controlled_prefix()
{
    assert_eq!(validate_credential_target(" DesktopTools/astrbot-status ").unwrap(), "DesktopTools/astrbot-status");
    for rejected in ["", "Other/x", "desktopTools/x", "DesktopTools", "DesktopTools/", "DesktopTools/a\nb"]
    {
        assert_eq!(validate_credential_target(rejected).unwrap_err().code, "loopback-credential-invalid", "{rejected:?}");
    }
    assert!(validate_credential_target(&format!("DesktopTools/{}", "x".repeat(300))).is_err());
    assert!(validate_credential_secret("token-123").is_ok());
    assert!(validate_credential_secret("").is_err());
    assert!(validate_credential_secret("bad\ntoken").is_err());
    assert!(validate_credential_secret(&"x".repeat(2561)).is_err());
}

/// 出错时只回结构化码与中文说明，密钥明文不出现在消息或调试输出里。
#[test]
fn credential_errors_never_echo_the_secret()
{
    let secret = "super-secret-token-value";
    let target_error = execute_set_loopback_credential("Other/target".to_string(), secret.to_string()).unwrap_err();
    assert_eq!(target_error.code, "loopback-credential-invalid");
    assert!(!target_error.message.contains(secret));
    assert!(!format!("{target_error:?}").contains(secret));
    let too_long = "y".repeat(3000);
    let secret_error = execute_set_loopback_credential(test_target("too-long").to_string(), too_long.clone()).unwrap_err();
    assert_eq!(secret_error.code, "loopback-credential-invalid");
    assert!(!secret_error.message.contains(&too_long));
    assert!(!format!("{secret_error:?}").contains(&too_long));
    let empty_error = execute_set_loopback_credential(test_target("empty").to_string(), String::new()).unwrap_err();
    assert_eq!(empty_error.code, "loopback-credential-invalid");
}

/// 写入后 loopback 通道能读到同一 target 的凭据；清除后注册回 loopback-credential-unavailable。
#[test]
fn write_read_clear_round_trip_feeds_the_loopback_channel()
{
    let target = test_target("roundtrip");
    let secret = "round-trip-secret";
    let _ = execute_clear_loopback_credential(target.clone()); // 先清掉上次失败留下的记录。
    assert_eq!(register_request_channel(&target).unwrap_err().code, "loopback-credential-unavailable", "写入前没有凭据");
    execute_set_loopback_credential(target.clone(), secret.to_string()).unwrap();
    assert_eq!(crate::platform::windows::credentials::read_secret(&target).unwrap(), Some(secret.as_bytes().to_vec()));
    let handle = register_request_channel(&target).expect("写入后通道注册应读到凭据"); // 注册成功即证明宿主按同一 target + headerName 读到了密钥。
    assert_eq!(handle.channel_id, "status");
    assert_eq!(handle.mode, LoopbackChannelMode::Request);
    execute_clear_loopback_credential(target.clone()).unwrap();
    assert_eq!(crate::platform::windows::credentials::read_secret(&target).unwrap(), None);
    assert_eq!(register_request_channel(&target).unwrap_err().code, "loopback-credential-unavailable", "清除后通道注册应报不可用");
    execute_clear_loopback_credential(target).unwrap(); // 再次清除不存在的记录视为成功。
}
