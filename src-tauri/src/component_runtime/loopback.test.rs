use std::collections::{HashSet, VecDeque};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::*;
use crate::contracts::LoopbackChannelMode;

/// 绑定一个临时端口后立刻释放，供监听通道测试使用。
fn free_port() -> u16 { TcpListener::bind(("127.0.0.1", 0)).unwrap().local_addr().unwrap().port() }

/// 造一条 listen 通道的注册请求。
fn listen_request(instance_id: &str, channel_id: &str, port: u16) -> RegisterLoopbackChannel { RegisterLoopbackChannel { instance_id: instance_id.to_string(), channel_id: channel_id.to_string(), mode: LoopbackChannelMode::Listen, port: Some(port), base_url: None, credential_ref: None, header_name: None } }

/// 造一条 request 通道的注册请求。
fn request_request(instance_id: &str, channel_id: &str, port: u16) -> RegisterLoopbackChannel { RegisterLoopbackChannel { instance_id: instance_id.to_string(), channel_id: channel_id.to_string(), mode: LoopbackChannelMode::Request, port: None, base_url: Some(format!("http://127.0.0.1:{port}")), credential_ref: None, header_name: None } }

/// 发一条原始 HTTP 请求并读完整响应；服务端一律 Connection: close。
fn send_http(port: u16, request: &str) -> String
{
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    stream.write_all(request.as_bytes()).unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response
}

/// 造一条未分配序号的原始请求。
fn raw_request(path: &str) -> RawRequest { RawRequest { method: "POST".to_string(), path: path.to_string(), headers: vec![], body: String::new() } }

/// 通道标识、基址与常量时间比较只接受本机原始 HTTP 边界内的值。
#[test]
fn loopback_boundaries_accept_only_local_raw_http()
{
    assert_eq!(normalize_base_url("http://127.0.0.1:8766").unwrap(), "http://127.0.0.1:8766");
    assert_eq!(normalize_base_url("http://localhost:8766").unwrap(), "http://127.0.0.1:8766");
    for invalid in ["https://127.0.0.1:8766", "http://10.0.0.1:8766", "http://127.0.0.1:8766/path", "http://127.0.0.1", "http://127.0.0.1:0", "http://127.0.0.1:http"]
    {
        assert!(normalize_base_url(invalid).is_err(), "{invalid}");
    }
    assert!(validate_channel_id("status").is_ok());
    for invalid in ["", "Status", "状态", "a b"]
    {
        assert!(validate_channel_id(invalid).is_err(), "{invalid}");
    }
    assert!(is_valid_header_name("X-Status-Token"));
    assert!(!is_valid_header_name("X-Status-Token\r\n"));
    assert!(constant_time_eq(b"token", b"token"));
    assert!(!constant_time_eq(b"token", b"tokeN"));
    assert!(!constant_time_eq(b"token", b"tok"));
    assert!(header_matches(&[LoopbackHeader { name: "x-status-token".to_string(), value: "abc".to_string() }], "X-Status-Token", b"abc"));
    assert!(!header_matches(&[LoopbackHeader { name: "x-status-token".to_string(), value: "abd".to_string() }], "X-Status-Token", b"abc"));
    assert_eq!(event_kind("status"), "loopback-data:status");
}

/// 注册参数按方向二选一，端口被占用时返回结构化错误。
#[test]
fn registration_validates_endpoint_and_reports_occupied_ports()
{
    let registry = LoopbackRegistry::new();
    let port = free_port();
    let mut mismatched = listen_request("instance-1", "status", port);
    mismatched.base_url = Some("http://127.0.0.1:8766".to_string());
    assert_eq!(registry.register_with_credential(mismatched, "plugin.astrbot", None).unwrap_err().code, "loopback-address-invalid");
    let mut missing_port = listen_request("instance-1", "status", port);
    missing_port.port = None;
    assert_eq!(registry.register_with_credential(missing_port, "plugin.astrbot", None).unwrap_err().code, "loopback-address-invalid");
    let occupied = TcpListener::bind(("127.0.0.1", 0)).unwrap(); // 被其它进程占用的端口。
    let occupied_port = occupied.local_addr().unwrap().port();
    assert_eq!(registry.register_with_credential(listen_request("instance-1", "status", occupied_port), "plugin.astrbot", None).unwrap_err().code, "loopback-port-in-use");
    assert_eq!(registry.channel_count("instance-1"), 0);
}

/// 入站请求按原始形态入缓冲，序号递增，读到即消费，afterSequence 只返回更大序号。
#[test]
fn listen_channel_buffers_raw_requests_and_read_consumes_them()
{
    let registry = LoopbackRegistry::new();
    let port = free_port();
    registry.register_with_credential(listen_request("instance-1", "status", port), "plugin.astrbot", None).unwrap();
    let first = send_http(port, "POST /snapshot HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: 11\r\n\r\n{\"status\":1");
    assert!(first.starts_with("HTTP/1.1 200"), "{first}");
    send_http(port, "GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n");
    let items = registry.read("instance-1", "status", None).unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].sequence, 1);
    assert_eq!(items[0].method, "POST");
    assert_eq!(items[0].path, "/snapshot");
    assert_eq!(items[0].body, "{\"status\":1");
    assert!(items[0].headers.iter().any(|header| header.name == "Content-Type" && header.value == "application/json"));
    assert!(items[0].received_at > 0);
    assert_eq!(items[1].sequence, 2);
    assert!(registry.read("instance-1", "status", None).unwrap().is_empty(), "读到即消费");
    send_http(port, "GET /again HTTP/1.1\r\n\r\n");
    let after = registry.read("instance-1", "status", Some(2)).unwrap();
    assert_eq!(after.iter().map(|item| item.sequence).collect::<Vec<_>>(), [3]);
    assert_eq!(registry.read("instance-1", "missing", None).unwrap_err().code, "loopback-channel-not-found");
}

/// 凭据校验失败回 401 且不交给插件；声明头名与密钥一致时才入缓冲。
#[test]
fn listen_channel_enforces_the_declared_credential_header()
{
    let registry = LoopbackRegistry::new();
    let port = free_port();
    let credential = Arc::new(ChannelCredential { header_name: "X-Status-Token".to_string(), secret: Zeroizing::new("secret-token".to_string()) });
    registry.register_with_credential(listen_request("instance-1", "status", port), "plugin.astrbot", Some(credential)).unwrap();
    assert!(send_http(port, "POST /snapshot HTTP/1.1\r\nContent-Length: 2\r\n\r\nok").starts_with("HTTP/1.1 401"));
    assert!(send_http(port, "POST /snapshot HTTP/1.1\r\nX-Status-Token: nope\r\nContent-Length: 2\r\n\r\nok").starts_with("HTTP/1.1 401"));
    assert!(registry.read("instance-1", "status", None).unwrap().is_empty(), "被拒请求不进入缓冲");
    assert!(send_http(port, "POST /snapshot HTTP/1.1\r\nX-Status-Token: secret-token\r\nContent-Length: 2\r\n\r\nok").starts_with("HTTP/1.1 200"));
    let items = registry.read("instance-1", "status", None).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].body, "ok");
    assert!(!items[0].headers.iter().any(|header| header.name.eq_ignore_ascii_case("x-status-token")), "凭据明文不交给插件");
}

/// 超过单条 body 上限的请求整条丢弃并回 413，不当成正常数据。
#[test]
fn oversized_inbound_body_is_rejected_and_counted()
{
    let registry = LoopbackRegistry::new();
    let port = free_port();
    registry.register_with_credential(listen_request("instance-1", "status", port), "plugin.astrbot", None).unwrap();
    let response = send_http(port, &format!("POST /snapshot HTTP/1.1\r\nContent-Length: {}\r\n\r\n", BODY_LIMIT + 1));
    assert!(response.starts_with("HTTP/1.1 413"), "{response}");
    assert!(registry.read("instance-1", "status", None).unwrap().is_empty());
}

/// 格式无效的请求回 400，不进入缓冲。
#[test]
fn malformed_inbound_request_is_rejected()
{
    let registry = LoopbackRegistry::new();
    let port = free_port();
    registry.register_with_credential(listen_request("instance-1", "status", port), "plugin.astrbot", None).unwrap();
    assert!(send_http(port, "GARBAGE\r\n\r\n").starts_with("HTTP/1.1 400"));
    assert!(registry.read("instance-1", "status", None).unwrap().is_empty());
}

/// 缓冲按条数丢最旧、按 300 秒淘汰旧条目。
#[test]
fn buffer_drops_oldest_beyond_limit_and_expires_by_age()
{
    let state = ChannelState { buffer: Mutex::new(BufferState { next_sequence: 0, entries: VecDeque::new(), dropped: 0 }), subscribed: AtomicBool::new(false) };
    for index in 0..(BUFFER_LIMIT + 2)
    {
        insert_request_at(&state, None, "status", raw_request(&format!("/n/{index}")), 1_000);
    }
    {
        let buffer = state.buffer.lock().unwrap();
        assert_eq!(buffer.entries.len(), BUFFER_LIMIT);
        assert_eq!(buffer.entries.front().unwrap().sequence, 3, "超限丢最旧");
        assert_eq!(buffer.entries.back().unwrap().sequence, (BUFFER_LIMIT + 2) as u64);
        assert_eq!(buffer.dropped, 2);
    }
    insert_request_at(&state, None, "status", raw_request("/later"), 1_000 + RETENTION_SECONDS + 1);
    let buffer = state.buffer.lock().unwrap();
    assert_eq!(buffer.entries.len(), 1, "超过保留期的条目按时间清理");
    assert_eq!(buffer.entries.front().unwrap().sequence, (BUFFER_LIMIT + 3) as u64);
}

/// 首次读取前只入缓冲不发事件；读取后才按通道标识通知。
#[test]
fn live_notifications_start_only_after_the_first_read()
{
    let registry = LoopbackRegistry::new();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let recorder = Arc::clone(&seen);
    registry.set_notifier(Arc::new(move |channel_id: &str| recorder.lock().unwrap().push(channel_id.to_string())));
    let port = free_port();
    registry.register_with_credential(listen_request("instance-1", "status", port), "plugin.astrbot", None).unwrap();
    send_http(port, "POST /snapshot HTTP/1.1\r\nContent-Length: 0\r\n\r\n");
    assert!(seen.lock().unwrap().is_empty(), "没有订阅读者时不发事件");
    registry.read("instance-1", "status", None).unwrap();
    send_http(port, "POST /snapshot HTTP/1.1\r\nContent-Length: 0\r\n\r\n");
    assert_eq!(seen.lock().unwrap().as_slice(), ["status"]);
}

/// 每实例最多两条通道；同名通道替换旧端口与缓冲，释放后端口立即可用。
#[test]
fn channel_limit_replacement_and_release_hold_the_hard_limits()
{
    let registry = LoopbackRegistry::new();
    let first_port = free_port();
    registry.register_with_credential(listen_request("instance-1", "one", first_port), "plugin.astrbot", None).unwrap();
    registry.register_with_credential(listen_request("instance-1", "two", free_port()), "plugin.astrbot", None).unwrap();
    assert_eq!(registry.channel_count("instance-1"), 2);
    let error = registry.register_with_credential(listen_request("instance-1", "three", free_port()), "plugin.astrbot", None).unwrap_err();
    assert_eq!(error.code, "loopback-channel-limit");
    registry.register_with_credential(listen_request("instance-1", "one", first_port), "plugin.astrbot", None).unwrap(); // 同名通道替换，旧端口先释放。
    registry.release_component("plugin.astrbot");
    assert_eq!(registry.channel_count("instance-1"), 0);
    drop(TcpListener::bind(("127.0.0.1", first_port)).unwrap()); // 端口已随实例释放。
}

/// 实例从工作区消失后清理通道并释放端口。
#[test]
fn prune_releases_channels_of_removed_instances()
{
    let registry = LoopbackRegistry::new();
    let port = free_port();
    registry.register_with_credential(listen_request("instance-1", "status", port), "plugin.astrbot", None).unwrap();
    registry.prune(&HashSet::from(["instance-2".to_string()]));
    assert_eq!(registry.channel_count("instance-1"), 0);
    drop(TcpListener::bind(("127.0.0.1", port)).unwrap());
}

/// request 通道原样转发响应，并按声明注入凭据头。
#[test]
fn request_channel_forwards_raw_response_and_injects_credential()
{
    let target = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = target.local_addr().unwrap().port();
    let received = Arc::new(Mutex::new(String::new()));
    let recorder = Arc::clone(&received);
    std::thread::spawn(move || {
        let (mut stream, _) = target.accept().unwrap();
        let mut buffer = [0_u8; 2048];
        let read = stream.read(&mut buffer).unwrap();
        *recorder.lock().unwrap() = String::from_utf8_lossy(&buffer[..read]).into_owned();
        let _ = stream.write_all(b"HTTP/1.1 201 Created\r\nX-Reply: yes\r\nContent-Length: 5\r\n\r\nhello");
    });
    let registry = LoopbackRegistry::new();
    let credential = Arc::new(ChannelCredential { header_name: "X-Status-Token".to_string(), secret: Zeroizing::new("secret-token".to_string()) });
    registry.register_with_credential(request_request("instance-1", "control", port), "plugin.astrbot", Some(credential)).unwrap();
    let response = registry.request("instance-1", "control", "POST", "/api/platform/start", Some("{}".to_string())).unwrap();
    assert_eq!(response.status, 201);
    assert_eq!(response.body, "hello");
    assert!(response.headers.iter().any(|header| header.name.eq_ignore_ascii_case("x-reply") && header.value == "yes"));
    let raw = received.lock().unwrap().clone();
    assert!(raw.starts_with("POST /api/platform/start HTTP/1.1"), "{raw}");
    assert!(raw.to_ascii_lowercase().contains("x-status-token: secret-token"), "{raw}");
}

/// listen 通道不能当 request 用，非法方法或路径在发请求前拒绝。
#[test]
fn request_direction_and_arguments_are_validated()
{
    let registry = LoopbackRegistry::new();
    let port = free_port();
    registry.register_with_credential(listen_request("instance-1", "status", port), "plugin.astrbot", None).unwrap();
    assert_eq!(registry.request("instance-1", "status", "GET", "/", None).unwrap_err().code, "loopback-address-invalid");
    assert!(validate_method("GET / HTTP/1.1").is_err());
    assert!(validate_request_path("//evil").is_err());
    assert!(validate_request_path("/snapshot?x=1").is_ok());
}

/// 未声明的凭据记录给出结构化错误，不泄露记录内容以外的信息。
#[test]
fn missing_credential_record_reports_unavailable()
{
    let error = read_credential("DesktopTools/loopback-test-missing").unwrap_err();
    assert_eq!(error.code, "loopback-credential-unavailable");
    let mut missing_header = request_request("instance-1", "control", 8766);
    missing_header.credential_ref = Some("DesktopTools/loopback-test-missing".to_string());
    assert_eq!(resolve_credential(&missing_header).err().unwrap().code, "loopback-credential-unavailable");
    missing_header.header_name = Some("X-Status-Token".to_string());
    assert_eq!(resolve_credential(&missing_header).err().unwrap().code, "loopback-credential-unavailable");
}

/// 宿主序列化的字段名与前端契约逐字段一致，通知种类前缀可被前端识别。
#[test]
fn serialized_contract_fields_match_the_frontend_bridge()
{
    let event = crate::contracts::ComponentHostEvent { sequence: 7, kind: event_kind("astrbot-push"), unix_millis: 1_700_000_000_000 };
    assert_eq!(serde_json::to_value(&event).unwrap(), serde_json::json!({ "sequence": 7, "kind": "loopback-data:astrbot-push", "unixMillis": 1_700_000_000_000_i64 }));
    assert!(is_event_kind("loopback-data:astrbot-push"));
    assert!(!is_event_kind("time-tick"));

    let handle = LoopbackChannelHandle { channel_id: "astrbot-push".to_string(), mode: LoopbackChannelMode::Listen, port: Some(8765), base_url: None, registered_at: 1_700_000_000 };
    assert_eq!(serde_json::to_value(&handle).unwrap(), serde_json::json!({ "channelId": "astrbot-push", "mode": "listen", "port": 8765, "baseUrl": null, "registeredAt": 1_700_000_000 }));

    let request = LoopbackRequest { sequence: 1, received_at: 1_700_000_000, method: "POST".to_string(), path: "/v1/snapshot".to_string(), headers: vec![LoopbackHeader { name: "X-Status-Token".to_string(), value: "secret".to_string() }], body: "{}".to_string() };
    assert_eq!(serde_json::to_value(&request).unwrap(), serde_json::json!({ "sequence": 1, "receivedAt": 1_700_000_000, "method": "POST", "path": "/v1/snapshot", "headers": [{ "name": "X-Status-Token", "value": "secret" }], "body": "{}" }));

    let response = LoopbackResponse { status: 200, headers: vec![], body: "ok".to_string() };
    assert_eq!(serde_json::to_value(&response).unwrap(), serde_json::json!({ "status": 200, "headers": [], "body": "ok" }));
}

/// 第二个假数据源换端口与 JSON 后，宿主零改动即可注册、缓冲并分别通知。
#[test]
fn a_second_data_source_reuses_the_same_host_path()
{
    let registry = LoopbackRegistry::new();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let recorder = Arc::clone(&seen);
    registry.set_notifier(Arc::new(move |channel_id: &str| recorder.lock().unwrap().push(channel_id.to_string())));
    let astrbot_port = free_port();
    let demo_port = free_port();
    registry.register_with_credential(listen_request("instance-1", "astrbot-push", astrbot_port), "plugin.astrbot", None).unwrap();
    registry.register_with_credential(listen_request("instance-1", "demo-push", demo_port), "plugin.demo", None).unwrap();
    registry.read("instance-1", "astrbot-push", None).unwrap();
    registry.read("instance-1", "demo-push", None).unwrap();

    let astrbot_body = r#"{"schemaVersion":2,"service":{"probe":{"level":"ok"}}}"#; // AstrBot 快照。
    let demo_body = r#"{"kind":"demo","value":42}"#; // 另一个数据源的完全不同结构。
    for (port, body) in [(astrbot_port, astrbot_body), (demo_port, demo_body)]
    {
        let request = format!("POST /v1/snapshot HTTP/1.1\r\nContent-Length: {}\r\n\r\n{body}", body.len()); // 原始推送请求。
        assert!(send_http(port, &request).starts_with("HTTP/1.1 200"));
    }

    assert_eq!(registry.read("instance-1", "astrbot-push", None).unwrap()[0].body, astrbot_body, "宿主不解析也不改写正文");
    assert_eq!(registry.read("instance-1", "demo-push", None).unwrap()[0].body, demo_body);
    assert_eq!(seen.lock().unwrap().as_slice(), ["astrbot-push", "demo-push"], "两条通道各自通知，互不串数据");
    registry.release_component("plugin.demo"); // 只释放另一个数据源的插件。
    assert_eq!(registry.channel_count("instance-1"), 1, "两条通道互相独立");
}
