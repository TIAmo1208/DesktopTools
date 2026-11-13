//! 网易云调试端口：客户端以 `--remote-debugging-port=9222` 启动时读取真实播放头。
//! 端口未开、目标未就绪或页面结构变化时返回 `None`，由调用方回退本地锚点外推。
use serde_json::Value;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

/// 调试端口固定在回环地址；未开启时连接立即失败，不阻塞媒体线程。
const ENDPOINT: &str = "http://127.0.0.1:9222";
/// 单次读取的等待上限；正常响应在毫秒级。
const TIMEOUT: Duration = Duration::from_millis(1200);
/// 播放条滑块是网易云页面唯一稳定的播放头来源；`max` 同时给出总时长（秒）。
const SLIDER_EXPRESSION: &str =
    "(()=>{const e=document.querySelector('[class*=StyledSliderContainer] input');if(!e)return null;const p=Number(e.value),d=Number(e.max);return Number.isFinite(p)&&p>=0?{p:p*1000,d:Number.isFinite(d)&&d>0?d*1000:0}:null})()";

/// 播放源身份是否网易云（`SourceAppUserModelId` 为 `cloudmusic.exe`）。
pub fn is_netease_source(source: &str) -> bool { source.to_ascii_lowercase().contains("cloudmusic") }

/// 一次调试读取的结果：播放位置与总时长（毫秒）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Position
{
    pub position_ms: f64,
    pub duration_ms: f64,
}

/// 读取当前播放位置与总时长；调试端口未开、页面未就绪或结构变化时返回 `None`。
pub fn read_position() -> Option<Position> { read_position_from(ENDPOINT) }

/// 按给定调试端点读取；端点参数只用于测试替身。
fn read_position_from(endpoint: &str) -> Option<Position>
{
    let target = target_websocket_url(endpoint)?;
    let (host, port, path) = parse_websocket_url(&target)?;
    let address: SocketAddr = format!("{host}:{port}").parse().ok()?;
    let mut stream = TcpStream::connect_timeout(&address, TIMEOUT).ok()?;
    stream.set_read_timeout(Some(TIMEOUT)).ok()?;
    stream.set_write_timeout(Some(TIMEOUT)).ok()?;
    handshake(&mut stream, &host, port, &path).ok()?;
    let request = serde_json::json!({
        "id": 1,
        "method": "Runtime.evaluate",
        "params": { "expression": SLIDER_EXPRESSION, "returnByValue": true }
    })
    .to_string();
    stream.write_all(&build_text_frame(request.as_bytes())).ok()?;
    let reply = read_reply(&mut stream)?;
    let value = reply.get("result")?.get("result")?.get("value")?;
    let position = value.get("p")?.as_f64()?;
    let duration = value.get("d").and_then(Value::as_f64).unwrap_or(0.0);
    (position >= 0.0).then_some(Position { position_ms: position, duration_ms: duration.max(0.0) })
}

/// 从调试端点的目标列表里选网易云播放页；页面尚未加载时返回 `None`。
fn target_websocket_url(endpoint: &str) -> Option<String>
{
    let client = reqwest::blocking::Client::builder().timeout(TIMEOUT).build().ok()?;
    let body = client.get(format!("{endpoint}/json")).send().ok()?.text().ok()?;
    select_target(&body)
}

/// 目标列表里优先 orpheus 播放页，其次任一 page；只接受带 ws 地址的目标。
fn select_target(body: &str) -> Option<String>
{
    let targets: Vec<Value> = serde_json::from_str(body).ok()?;
    let pages: Vec<&Value> = targets.iter().filter(|target| target.get("type").and_then(Value::as_str) == Some("page")).collect();
    let chosen = pages.iter().find(|target| target.get("url").and_then(Value::as_str).is_some_and(|url| url.contains("orpheus"))).or_else(|| pages.first())?;
    chosen.get("webSocketDebuggerUrl").and_then(Value::as_str).map(str::to_string)
}

/// 解析 `ws://host:port/path`；调试端点的地址固定来自目标列表。
fn parse_websocket_url(url: &str) -> Option<(String, u16, String)>
{
    let rest = url.strip_prefix("ws://")?;
    let (authority, path) = rest.split_once('/')?;
    let (host, port) = authority.rsplit_once(':')?;
    Some((host.to_string(), port.parse().ok()?, format!("/{path}")))
}

/// WebSocket 握手；只接受 101 Switching Protocols。
fn handshake(stream: &mut TcpStream, host: &str, port: u16, path: &str) -> std::io::Result<()>
{
    let key = base64_encode(&[0x44; 16]); // 回环调试不需要随机性，密钥只要满足 16 字节格式。
    let request = format!("GET {path} HTTP/1.1\r\nHost: {host}:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n");
    stream.write_all(request.as_bytes())?;
    let response = read_http_head(stream)?;
    if !response.starts_with("HTTP/1.1 101")
    {
        return Err(std::io::Error::other("调试端口握手失败"));
    }
    Ok(())
}

/// 逐字节读到响应头结束，避免把后续帧读进缓冲区。
fn read_http_head(stream: &mut TcpStream) -> std::io::Result<String>
{
    let mut head = Vec::with_capacity(256);
    let mut byte = [0u8; 1];
    while head.len() < 4096
    {
        stream.read_exact(&mut byte)?;
        head.push(byte[0]);
        if head.ends_with(b"\r\n\r\n")
        {
            return Ok(String::from_utf8_lossy(&head).into_owned());
        }
    }
    Err(std::io::Error::other("调试端口响应头过长"))
}

/// 客户端帧必须带掩码；支持 125 以内、16 位与 64 位三种长度。
fn build_text_frame(payload: &[u8]) -> Vec<u8>
{
    let mask = [0x37u8, 0x11, 0x5a, 0x0f];
    let mut frame = Vec::with_capacity(payload.len() + 14);
    frame.push(0x81);
    let length = payload.len();
    if length < 126
    {
        frame.push(0x80 | length as u8);
    }
    else if length <= u16::MAX as usize
    {
        frame.push(0x80 | 126);
        frame.extend_from_slice(&(length as u16).to_be_bytes());
    }
    else
    {
        frame.push(0x80 | 127);
        frame.extend_from_slice(&(length as u64).to_be_bytes());
    }
    frame.extend_from_slice(&mask);
    frame.extend(payload.iter().enumerate().map(|(index, byte)| byte ^ mask[index % 4]));
    frame
}

/// 读取一帧；服务端帧通常无掩码，仍按标志位兼容掩码与超长帧。
fn read_frame(stream: &mut impl Read) -> std::io::Result<(u8, Vec<u8>)>
{
    let mut head = [0u8; 2];
    stream.read_exact(&mut head)?;
    let opcode = head[0] & 0x0f;
    let masked = head[1] & 0x80 != 0;
    let mut length = (head[1] & 0x7f) as u64;
    if length == 126
    {
        let mut buffer = [0u8; 2];
        stream.read_exact(&mut buffer)?;
        length = u16::from_be_bytes(buffer) as u64;
    }
    else if length == 127
    {
        let mut buffer = [0u8; 8];
        stream.read_exact(&mut buffer)?;
        length = u64::from_be_bytes(buffer);
    }
    if length > 4 * 1024 * 1024
    {
        return Err(std::io::Error::other("调试帧过大"));
    }
    let mask = if masked
    {
        let mut key = [0u8; 4];
        stream.read_exact(&mut key)?;
        Some(key)
    }
    else
    {
        None
    };
    let mut payload = vec![0u8; length as usize];
    stream.read_exact(&mut payload)?;
    if let Some(mask) = mask
    {
        for (index, byte) in payload.iter_mut().enumerate()
        {
            *byte ^= mask[index % 4];
        }
    }
    Ok((opcode, payload))
}

/// 读取到与请求 id 匹配的文本响应；跳过无关事件帧。
fn read_reply(stream: &mut TcpStream) -> Option<Value>
{
    for _ in 0..5
    {
        let (opcode, payload) = read_frame(stream).ok()?;
        if opcode != 1
        {
            continue;
        }
        let value: Value = serde_json::from_slice(&payload).ok()?;
        if value.get("id").and_then(Value::as_i64) == Some(1)
        {
            return Some(value);
        }
    }
    None
}

/// 标准字母表的 base64；只用于握手密钥与测试，不引入新依赖。
fn base64_encode(bytes: &[u8]) -> String
{
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3)
    {
        let first = chunk[0] as u32;
        let second = *chunk.get(1).unwrap_or(&0) as u32;
        let third = *chunk.get(2).unwrap_or(&0) as u32;
        let triple = (first << 16) | (second << 8) | third;
        encoded.push(TABLE[(triple >> 18) as usize & 63] as char);
        encoded.push(TABLE[(triple >> 12) as usize & 63] as char);
        encoded.push(if chunk.len() > 1 { TABLE[(triple >> 6) as usize & 63] as char } else { '=' });
        encoded.push(if chunk.len() > 2 { TABLE[triple as usize & 63] as char } else { '=' });
    }
    encoded
}

#[cfg(test)]
mod tests
{
    use super::*;
    use std::io::Cursor;

    /// 目标选择优先播放页，缺少 orpheus 时退到任一页面。
    #[test]
    fn target_selection_prefers_the_player_page()
    {
        let body = r#"[
            {"type":"page","url":"devtools://devtools/bundled/inspector.html","webSocketDebuggerUrl":"ws://127.0.0.1:9222/devtools/page/1"},
            {"type":"page","url":"orpheus://orpheus/pub/app.html","webSocketDebuggerUrl":"ws://127.0.0.1:9222/devtools/page/2"}
        ]"#;
        assert_eq!(select_target(body).as_deref(), Some("ws://127.0.0.1:9222/devtools/page/2"));
        assert_eq!(select_target(r#"[{"type":"page","url":"about:blank","webSocketDebuggerUrl":"ws://127.0.0.1:9222/devtools/page/3"}]"#).as_deref(), Some("ws://127.0.0.1:9222/devtools/page/3"));
        assert_eq!(select_target("[]"), None);
        assert_eq!(select_target("not json"), None);
    }

    /// ws 地址解析出主机、端口与路径。
    #[test]
    fn websocket_url_parsing_keeps_host_port_and_path()
    {
        assert_eq!(parse_websocket_url("ws://127.0.0.1:9222/devtools/page/2"), Some(("127.0.0.1".into(), 9222, "/devtools/page/2".into())));
        assert_eq!(parse_websocket_url("wss://127.0.0.1:9222/devtools/page/2"), None);
        assert_eq!(parse_websocket_url("ws://127.0.0.1/devtools/page/2"), None);
    }

    /// 客户端帧带掩码；按同一掩码解码可还原，覆盖三种长度编码。
    #[test]
    fn client_frames_mask_and_round_trip()
    {
        for length in [1usize, 125, 126, 300, 70_000]
        {
            let payload = vec![b'a'; length];
            let frame = build_text_frame(&payload);
            let mut cursor = Cursor::new(frame);
            let (opcode, decoded) = read_frame(&mut cursor).expect("客户端帧应可解码");
            assert_eq!(opcode, 1);
            assert_eq!(decoded, payload);
        }
        assert_eq!(base64_encode(b"0123456789abcdef").len(), 24);
        assert!(base64_encode(b"0123456789abcdef").ends_with("="));
    }

    /// 服务端无掩码文本帧按原样读出，二进制帧保留操作码。
    #[test]
    fn server_frames_decode_without_mask()
    {
        let mut frame = vec![0x81, 2, b'o', b'k'];
        let (opcode, payload) = read_frame(&mut Cursor::new(&frame)).expect("服务端帧应可解码");
        assert_eq!((opcode, payload.as_slice()), (1, b"ok".as_slice()));
        frame = vec![0x82, 1, 0xff];
        assert_eq!(read_frame(&mut Cursor::new(&frame)).expect("二进制帧应可解码").0, 2);
        // 超长帧直接拒绝，避免不可信响应占内存。
        let oversized = vec![0x81, 127, 0, 0, 0, 0, 0, 0x80, 0, 0, 0];
        assert!(read_frame(&mut Cursor::new(oversized)).is_err());
    }

    /// 端到端替身：目标列表、握手、求值请求与结果回包走完整链路。
    #[test]
    fn position_read_round_trips_over_http_and_websocket()
    {
        use std::io::{BufRead, BufReader, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("绑定本地替身");
        let port = listener.local_addr().expect("读取端口").port();
        let server = std::thread::spawn(move || {
            for _ in 0..2
            {
                let (mut stream, _) = listener.accept().expect("接受连接");
                let mut reader = BufReader::new(stream.try_clone().expect("复制连接"));
                let mut request = String::new();
                loop
                {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0
                    {
                        break;
                    }
                    request.push_str(&line);
                    if line == "\r\n"
                    {
                        break;
                    }
                }
                if request.starts_with("GET /json")
                {
                    let body = format!(r#"[{{"type":"page","url":"orpheus://orpheus/pub/app.html","webSocketDebuggerUrl":"ws://127.0.0.1:{port}/devtools/page/1"}}]"#);
                    let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}", body.len(), body);
                    stream.write_all(response.as_bytes()).expect("写目标列表");
                }
                else
                {
                    stream.write_all(b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: test\r\n\r\n").expect("写握手响应");
                    assert!(read_client_text(&mut stream).is_some_and(|text| text.contains("Runtime.evaluate")), "客户端应发送求值请求");
                    let reply = serde_json::json!({"id": 1, "result": {"result": {"value": {"p": 12_345.0, "d": 236_818.0}}}}).to_string();
                    stream.write_all(&server_text_frame(reply.as_bytes())).expect("写求值结果");
                }
            }
        });
        let position = read_position_from(&format!("http://127.0.0.1:{port}")).expect("应读到位置");
        assert_eq!(position, Position { position_ms: 12_345.0, duration_ms: 236_818.0 });
        server.join().expect("替身线程");
    }

    /// 读出客户端带掩码文本帧的正文。
    fn read_client_text(stream: &mut std::net::TcpStream) -> Option<String>
    {
        let mut head = [0u8; 2];
        stream.read_exact(&mut head).ok()?;
        let masked = head[1] & 0x80 != 0;
        let mut length = (head[1] & 0x7f) as usize;
        if length == 126
        {
            let mut extended = [0u8; 2];
            stream.read_exact(&mut extended).ok()?;
            length = u16::from_be_bytes(extended) as usize;
        }
        else if length == 127
        {
            let mut extended = [0u8; 8];
            stream.read_exact(&mut extended).ok()?;
            length = u64::from_be_bytes(extended) as usize;
        }
        let mut mask = [0u8; 4];
        if masked
        {
            stream.read_exact(&mut mask).ok()?;
        }
        let mut payload = vec![0u8; length];
        stream.read_exact(&mut payload).ok()?;
        if masked
        {
            for (index, byte) in payload.iter_mut().enumerate()
            {
                *byte ^= mask[index % 4];
            }
        }
        String::from_utf8(payload).ok()
    }

    /// 服务端文本帧不带掩码。
    fn server_text_frame(payload: &[u8]) -> Vec<u8>
    {
        let mut frame = vec![0x81];
        if payload.len() < 126
        {
            frame.push(payload.len() as u8);
        }
        else
        {
            frame.push(126);
            frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
        }
        frame.extend_from_slice(payload);
        frame
    }
}
