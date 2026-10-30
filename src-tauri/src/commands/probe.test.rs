use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::Duration;

use super::{judge, observe, parse_probe_service, probe, Observation, ProbeLevel, ProbeService};

/// 起一个只应答一次的本地服务，返回其地址与应答线程。
fn serve_once(status: &str, body: &str, delay: Duration) -> (String, thread::JoinHandle<()>)
{
    let listener = TcpListener::bind("127.0.0.1:0").expect("测试服务应能监听本机端口"); // 测试专用的临时端口。
    let address = listener.local_addr().expect("测试服务应有本地地址");
    let response = format!("HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len()); // 固定应答内容。
    let handle = thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept()
        {
            let mut request = [0_u8; 1024]; // 读掉请求头，避免客户端等待。
            let _ = stream.read(&mut request);
            thread::sleep(delay);
            let _ = stream.write_all(response.as_bytes());
        }
    });
    (format!("http://{address}/api/stat/start-time"), handle)
}

/// 只有 astrbot 接受，其它值都是参数错误。
#[test]
fn service_name_only_accepts_astrbot()
{
    assert_eq!(parse_probe_service("astrbot"), Ok(ProbeService::AstrBot));
    assert!(parse_probe_service("napcat").is_err());
    assert!(parse_probe_service("").is_err());
    assert!(parse_probe_service("http://127.0.0.1:6185").is_err());
}

/// 四档判定映射：超时归 unknown 而不是 off，401 与结构异常归 warn。
#[test]
fn observations_map_to_four_levels()
{
    assert_eq!(judge(Observation::Running(1_793_365_017)), (ProbeLevel::Ok, Some(1_793_365_017)));
    assert_eq!(judge(Observation::Refused), (ProbeLevel::Off, None));
    assert_eq!(judge(Observation::Unauthorized), (ProbeLevel::Warn, None));
    assert_eq!(judge(Observation::Malformed), (ProbeLevel::Warn, None));
    assert_eq!(judge(Observation::StatusFailed), (ProbeLevel::Warn, None));
    assert_eq!(judge(Observation::TimedOut), (ProbeLevel::Unknown, None));
    assert_eq!(judge(Observation::TransportFailed), (ProbeLevel::Unknown, None));
}

/// 缺服务名或非法服务名归入 unknown，且不带启动时间。
#[test]
fn invalid_service_reports_unknown_without_start_time()
{
    for payload in [serde_json::json!({}), serde_json::json!({ "service": "napcat" }), serde_json::json!({ "service": 1 })]
    {
        let result = probe(&payload); // 非法入参的探活结果。
        assert_eq!(result["level"], "unknown");
        assert!(result["startTime"].is_null());
        assert!(result["checkedAt"].as_i64().unwrap() > 0);
    }
}

/// 真实 HTTP 应答：正常结构给出 ok 与启动时间。
#[test]
fn real_response_reports_ok()
{
    let (url, handle) = serve_once("200 OK", r#"{"status":"ok","message":null,"data":{"start_time":1793365017}}"#, Duration::ZERO);
    assert_eq!(observe(&url, Duration::from_secs(2)), Observation::Running(1_793_365_017));
    handle.join().unwrap();
}

/// 浮点启动时间同样按运行中处理，取整到秒。
#[test]
fn real_response_accepts_float_start_time()
{
    let (url, handle) = serve_once("200 OK", r#"{"status":"ok","message":null,"data":{"start_time":1793365017.123}}"#, Duration::ZERO);
    assert_eq!(observe(&url, Duration::from_secs(2)), Observation::Running(1_793_365_017));
    handle.join().unwrap();
}

/// 字符串启动时间属于协议异常，归 warn 而不是离线。
#[test]
fn real_response_with_string_start_time_is_malformed()
{
    let (url, handle) = serve_once("200 OK", r#"{"status":"ok","data":{"start_time":"1793365017"}}"#, Duration::ZERO);
    let observation = observe(&url, Duration::from_secs(2)); // 协议异常的观察结果。
    assert_eq!(observation, Observation::Malformed);
    assert_eq!(judge(observation).0, ProbeLevel::Warn);
    handle.join().unwrap();
}

/// 401 归 warn，表明探测被拒。
#[test]
fn real_response_401_is_unauthorized()
{
    let (url, handle) = serve_once("401 Unauthorized", r#"{"status":"error","message":"未授权"}"#, Duration::ZERO);
    let observation = observe(&url, Duration::from_secs(2)); // 被拒的观察结果。
    assert_eq!(observation, Observation::Unauthorized);
    assert_eq!(judge(observation).0, ProbeLevel::Warn);
    handle.join().unwrap();
}

/// 500 归 warn。
#[test]
fn real_response_500_is_status_failed()
{
    let (url, handle) = serve_once("500 Internal Server Error", "{}", Duration::ZERO);
    assert_eq!(observe(&url, Duration::from_secs(2)), Observation::StatusFailed);
    handle.join().unwrap();
}

/// 无人监听时连接被拒，判定为离线。
#[test]
fn closed_port_reports_off()
{
    let listener = TcpListener::bind("127.0.0.1:0").expect("测试应能占用临时端口"); // 先占位拿到一个空闲端口。
    let address = listener.local_addr().expect("测试端口应有地址");
    drop(listener); // 立刻释放，端口上不再有进程监听。
    let observation = observe(&format!("http://{address}/api/stat/start-time"), Duration::from_secs(2)); // 无人监听时的观察结果。
    assert_eq!(observation, Observation::Refused);
    assert_eq!(judge(observation).0, ProbeLevel::Off);
}

/// 连接被拒的错误要能从多层包装里找出来，不能只看第一层。
#[test]
fn refused_error_is_found_through_wrapped_sources()
{
    #[derive(Debug)]
    struct Wrapper(Box<dyn std::error::Error + Send + Sync>); // 模拟 hyper 的多层错误包装。
    impl std::fmt::Display for Wrapper
    {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { formatter.write_str("包装错误") }
    }
    impl std::error::Error for Wrapper
    {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> { Some(&*self.0) }
    }
    let inner = std::io::Error::from(std::io::ErrorKind::ConnectionRefused); // 最内层的连接被拒。
    let wrapped = Wrapper(Box::new(inner)); // 第二层包装。
    assert!(super::has_connection_refused(&wrapped));
    let other = Wrapper(Box::new(std::io::Error::from(std::io::ErrorKind::TimedOut))); // 不是被拒的另一种错误。
    assert!(!super::has_connection_refused(&other));
}

/// 超时重试后仍超时归 unknown，不能显示成离线。
#[test]
fn timeout_reports_unknown_not_off()
{
    let (url, handle) = serve_once("200 OK", r#"{"status":"ok","data":{"start_time":1793365017}}"#, Duration::from_millis(600));
    let observation = observe(&url, Duration::from_millis(100)); // 短超时下的观察结果。
    assert_eq!(observation, Observation::TimedOut);
    assert_eq!(judge(observation).0, ProbeLevel::Unknown);
    handle.join().unwrap();
}
