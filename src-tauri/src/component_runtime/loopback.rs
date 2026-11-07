use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use zeroize::Zeroizing;

use crate::contracts::{ComponentRuntimeError, LoopbackChannelHandle, LoopbackChannelMode, LoopbackHeader, LoopbackRequest, LoopbackResponse, RegisterLoopbackChannel};

/// 每个实例最多注册的通道数。
pub(crate) const CHANNEL_LIMIT: usize = 2;
/// 每条通道入站缓冲最多保留的请求条数，超出丢最旧。
pub(crate) const BUFFER_LIMIT: usize = 256;
/// 单条入站请求体上限，超出整条丢弃并计入丢弃计数。
pub(crate) const BODY_LIMIT: usize = 1024 * 1024;
/// 入站缓冲整体保留的秒数，超时条目按时间清理。
pub(crate) const RETENTION_SECONDS: i64 = 300;
/// 入站请求头的有界读取上限。
const HEADER_LIMIT: usize = 16 * 1024;
/// 监听线程检查停止标志的间隔。
const ACCEPT_POLL: Duration = Duration::from_millis(20);
/// 单条入站连接与出站请求的超时。
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(5);
/// 出站响应的内存上限，避免本机异常服务撑爆宿主。
const RESPONSE_LIMIT: usize = 4 * 1024 * 1024;
/// 通道数据的宿主事件类型前缀，后接 `:<channelId>`。
pub(crate) const EVENT_KIND_PREFIX: &str = "loopback-data";

/// 通道缓冲更新后通知宿主事件层；参数是通道标识。
pub(crate) type LoopbackNotifier = Arc<dyn Fn(&str) + Send + Sync>;

/// 通道注册时从 Windows Credential Manager 读到的凭据，明文只在宿主内部使用。
pub(crate) struct ChannelCredential
{
    /// 注入与校验使用的头名，由插件声明。
    header_name: String,
    /// 密钥明文，Drop 时清零。
    secret: Zeroizing<String>,
}

/// 监听通道的后台线程与停止标志。
struct ListenServer
{
    /// 停止标志，Drop 时置 false。
    running: Arc<AtomicBool>,
    /// 监听线程句柄。
    thread: Option<std::thread::JoinHandle<()>>,
}

/// 释放端口并等待监听线程退出，保证重新注册同一端口时不与旧线程竞争。
impl Drop for ListenServer
{
    fn drop(&mut self)
    {
        self.running.store(false, Ordering::SeqCst);
        if let Some(thread) = self.thread.take()
        {
            let _ = thread.join();
        }
    }
}

/// 一条通道的入站缓冲与订阅状态；监听线程只持有这里的共享状态。
struct ChannelState
{
    /// 受锁保护的原始请求缓冲。
    buffer: Mutex<BufferState>,
    /// 插件是否读过该通道；没有订阅读者时只入缓冲、不发事件。
    subscribed: AtomicBool,
}

/// 递增序号与最旧丢弃的原始请求缓冲。
struct BufferState
{
    /// 下一条入站请求的序号，从 1 起。
    next_sequence: u64,
    /// 尚未被读走的条目，按接收顺序排列。
    entries: VecDeque<LoopbackRequest>,
    /// 因超限或超时而丢弃的条数，只用于日志。
    dropped: u64,
}

impl BufferState
{
    /// 按时间清理超出保留期的条目，只按时间不判断业务是否新鲜。
    fn purge_expired(&mut self, now: i64)
    {
        while let Some(front) = self.entries.front()
        {
            if now.saturating_sub(front.received_at) > RETENTION_SECONDS
            {
                self.entries.pop_front();
                self.dropped += 1;
            }
            else
            {
                break;
            }
        }
    }
}

/// 一条已注册通道的运行时记录；通道属于实例，不落盘。
struct ChannelEntry
{
    /// 注册通道的组件身份，用于插件卸载时整组释放。
    component_id: String,
    /// 返回给插件的通道句柄。
    handle: LoopbackChannelHandle,
    /// 凭据声明；未声明时不注入也不校验。
    credential: Option<Arc<ChannelCredential>>,
    /// 与监听线程共享的缓冲状态。
    state: Arc<ChannelState>,
    /// 监听通道的后台线程；request 通道为空。持有它只为 Drop 时释放端口。
    _server: Option<ListenServer>,
}

/// 宿主侧 loopback 通道注册表：注册、缓冲、读取、出站转发与实例卸载清理。
pub(crate) struct LoopbackRegistry
{
    /// 实例标识到该实例通道的映射。
    channels: Mutex<HashMap<String, HashMap<String, ChannelEntry>>>,
    /// 缓冲更新后的宿主事件通知。
    notifier: Mutex<Option<LoopbackNotifier>>,
}

impl LoopbackRegistry
{
    /// 创建空注册表。
    pub(crate) fn new() -> Self { Self { channels: Mutex::new(HashMap::new()), notifier: Mutex::new(None) } }

    /// 由命令层注入宿主事件通知；重复设置以最后一次为准。
    pub(crate) fn set_notifier(&self, notifier: LoopbackNotifier) { *self.notifier.lock().unwrap_or_else(|error| error.into_inner()) = Some(notifier); }

    /// 按插件声明注册通道，凭据由 Windows Credential Manager 解析。
    pub(crate) fn register(&self, request: RegisterLoopbackChannel, component_id: &str) -> Result<LoopbackChannelHandle, ComponentRuntimeError>
    {
        let credential = resolve_credential(&request)?; // 先校验凭据声明，避免建好监听后才失败。
        self.register_with_credential(request, component_id, credential)
    }

    /// 实际注册实现；测试用它注入凭据，绕开真实凭据存储。
    fn register_with_credential(&self, request: RegisterLoopbackChannel, component_id: &str, credential: Option<Arc<ChannelCredential>>) -> Result<LoopbackChannelHandle, ComponentRuntimeError>
    {
        validate_channel_id(&request.channel_id)?;
        let (port, base_url) = resolve_endpoint(&request)?;
        {
            let mut channels = self.channels.lock().unwrap_or_else(|error| error.into_inner());
            let instance = channels.entry(request.instance_id.clone()).or_default();
            let existing = instance.keys().filter(|key| *key != &request.channel_id).count(); // 替换同名通道不占额度。
            if existing >= CHANNEL_LIMIT
            {
                return Err(ComponentRuntimeError::loopback_channel_limit(CHANNEL_LIMIT));
            }
            instance.remove(&request.channel_id); // 同名通道以最后一次为准，旧端口与缓冲一并释放。
        }
        let listener = match request.mode
        {
            LoopbackChannelMode::Listen => Some(TcpListener::bind((Ipv4Addr::LOCALHOST, port.unwrap_or(0))).map_err(|error| {
                crate::log_warn!("loopback：监听绑定失败，实例={}，通道={}，端口={}，错误={error}", request.instance_id, request.channel_id, port.unwrap_or(0));
                ComponentRuntimeError::loopback_port_in_use(port.unwrap_or(0))
            })?),
            LoopbackChannelMode::Request => None,
        };
        let registered_at = Utc::now().timestamp();
        let handle = LoopbackChannelHandle { channel_id: request.channel_id.clone(), mode: request.mode, port, base_url, registered_at };
        let state = Arc::new(ChannelState { buffer: Mutex::new(BufferState { next_sequence: 0, entries: VecDeque::new(), dropped: 0 }), subscribed: AtomicBool::new(false) });
        let notifier = self.notifier.lock().unwrap_or_else(|error| error.into_inner()).clone();
        let server = listener.map(|listener| start_listen_server(listener, Arc::clone(&state), notifier, request.channel_id.clone(), credential.clone()));
        let entry = ChannelEntry { component_id: component_id.to_string(), handle: handle.clone(), credential, state, _server: server };
        self.channels.lock().unwrap_or_else(|error| error.into_inner()).entry(request.instance_id.clone()).or_default().insert(request.channel_id.clone(), entry);
        crate::log_info!("loopback：通道已注册，实例={}，通道={}，方向={:?}，端口={:?}", request.instance_id, handle.channel_id, handle.mode, handle.port);
        Ok(handle)
    }

    /// 读走缓冲区里尚未消费的条目，`afterSequence` 只返回序号更大的部分。
    pub(crate) fn read(&self, instance_id: &str, channel_id: &str, after_sequence: Option<u64>) -> Result<Vec<LoopbackRequest>, ComponentRuntimeError>
    {
        let channels = self.channels.lock().unwrap_or_else(|error| error.into_inner());
        let entry = channels.get(instance_id).and_then(|instance| instance.get(channel_id)).ok_or_else(ComponentRuntimeError::loopback_channel_not_found)?;
        entry.state.subscribed.store(true, Ordering::Relaxed); // 首次读取即视为插件开始订阅，之后缓冲更新才通知。
        let mut buffer = entry.state.buffer.lock().unwrap_or_else(|error| error.into_inner());
        buffer.purge_expired(Utc::now().timestamp());
        let drained = buffer.entries.drain(..).collect::<Vec<_>>(); // 返回即消费，同一序号不再出现。
        Ok(drained.into_iter().filter(|item| after_sequence.is_none_or(|sequence| item.sequence > sequence)).collect())
    }

    /// 向 request 通道的对端发一次原始 HTTP 请求，非 2xx 也照常返回。
    pub(crate) fn request(&self, instance_id: &str, channel_id: &str, method: &str, path: &str, body: Option<String>) -> Result<LoopbackResponse, ComponentRuntimeError>
    {
        let (base_url, credential) = {
            let channels = self.channels.lock().unwrap_or_else(|error| error.into_inner());
            let entry = channels.get(instance_id).and_then(|instance| instance.get(channel_id)).ok_or_else(ComponentRuntimeError::loopback_channel_not_found)?;
            if entry.handle.mode != LoopbackChannelMode::Request
            {
                return Err(ComponentRuntimeError::loopback_address_invalid("该通道不是 request 方向，不能发起请求"));
            }
            (entry.handle.base_url.clone().unwrap_or_default(), entry.credential.clone())
        };
        validate_method(method)?;
        validate_request_path(path)?;
        let client = reqwest::blocking::Client::builder().timeout(CONNECTION_TIMEOUT).build().map_err(|_| ComponentRuntimeError::loopback_request_failed("本机请求客户端创建失败"))?;
        let method = reqwest::Method::from_bytes(method.as_bytes()).map_err(|_| ComponentRuntimeError::loopback_address_invalid("请求方法无效"))?;
        let mut builder = client.request(method, format!("{base_url}{path}"));
        if let Some(credential) = credential
        {
            builder = builder.header(&credential.header_name, credential.secret.as_str()); // 明文只在这个请求头里出现，不写日志。
        }
        if let Some(body) = body
        {
            builder = builder.body(body);
        }
        let response = builder.send().map_err(|_| ComponentRuntimeError::loopback_request_failed("对端未响应或连接失败"))?;
        let status = response.status().as_u16();
        let headers = response.headers().iter().map(|(name, value)| LoopbackHeader { name: name.as_str().to_string(), value: String::from_utf8_lossy(value.as_bytes()).into_owned() }).collect::<Vec<_>>();
        let bytes = response.bytes().map_err(|_| ComponentRuntimeError::loopback_request_failed("读取对端响应失败"))?;
        if bytes.len() > RESPONSE_LIMIT
        {
            return Err(ComponentRuntimeError::loopback_request_failed("对端响应超过 4 MiB 上限"));
        }
        crate::log_debug!("loopback：出站请求完成，实例={instance_id}，通道={channel_id}，状态={status}");
        Ok(LoopbackResponse { status, headers, body: String::from_utf8_lossy(&bytes).into_owned() })
    }

    /// 实例集合变化后释放已消失实例的全部通道；仍在的实例不受影响。
    pub(crate) fn prune(&self, instances: &HashSet<String>)
    {
        let mut channels = self.channels.lock().unwrap_or_else(|error| error.into_inner());
        channels.retain(|instance, registered| {
            if instances.contains(instance)
            {
                return true;
            }
            crate::log_info!("loopback：实例已移除，释放通道，实例={instance}，通道数={}", registered.len());
            registered.clear(); // Drop 关闭监听端口并丢弃缓冲。
            false
        });
    }

    /// 插件卸载时按组件身份释放它名下全部实例的通道。
    pub(crate) fn release_component(&self, component_id: &str)
    {
        let mut channels = self.channels.lock().unwrap_or_else(|error| error.into_inner());
        channels.retain(|instance, registered| {
            registered.retain(|_, entry| entry.component_id != component_id);
            if registered.is_empty()
            {
                crate::log_info!("loopback：插件已卸载，释放通道，组件={component_id}，实例={instance}");
                return false;
            }
            true
        });
    }

    /// 测试用：返回实例当前注册的通道数。
    #[cfg(test)]
    fn channel_count(&self, instance_id: &str) -> usize { self.channels.lock().unwrap_or_else(|error| error.into_inner()).get(instance_id).map(HashMap::len).unwrap_or(0) }
}

/// 与宿主事件层拼出 `component-host-event` 的 kind。
pub(crate) fn event_kind(channel_id: &str) -> String { format!("{EVENT_KIND_PREFIX}:{channel_id}") }

/// 判断宿主事件种类是否是 loopback 数据通知；通道标识不能为空。
pub(crate) fn is_event_kind(kind: &str) -> bool { kind.strip_prefix(EVENT_KIND_PREFIX).is_some_and(|rest| rest.len() > 1 && rest.starts_with(':')) }

/// 校验通道标识：与组件标识同一字符集，实例内唯一。
fn validate_channel_id(channel_id: &str) -> Result<(), ComponentRuntimeError>
{
    if channel_id.is_empty() || channel_id.len() > 128 || !channel_id.chars().all(|character| character.is_ascii_lowercase() || character.is_ascii_digit() || matches!(character, '.' | '-'))
    {
        return Err(ComponentRuntimeError::loopback_address_invalid("通道标识无效，只允许小写字母、数字、点与连字符"));
    }
    Ok(())
}

/// 按方向校验端口或基址，并返回宿主实际使用的端点。
fn resolve_endpoint(request: &RegisterLoopbackChannel) -> Result<(Option<u16>, Option<String>), ComponentRuntimeError>
{
    let has_base_url = request.base_url.as_deref().is_some_and(|value| !value.trim().is_empty());
    match request.mode
    {
        LoopbackChannelMode::Listen =>
        {
            if has_base_url
            {
                return Err(ComponentRuntimeError::loopback_address_invalid("listen 通道不接受 baseUrl"));
            }
            let port = request.port.filter(|port| *port > 0).ok_or_else(|| ComponentRuntimeError::loopback_address_invalid("listen 通道必须提供 1–65535 的端口"))?;
            Ok((Some(port), None))
        }
        LoopbackChannelMode::Request =>
        {
            if request.port.is_some()
            {
                return Err(ComponentRuntimeError::loopback_address_invalid("request 通道不接受端口"));
            }
            let base_url = request.base_url.as_deref().map(str::trim).filter(|value| !value.is_empty()).ok_or_else(|| ComponentRuntimeError::loopback_address_invalid("request 通道必须提供 baseUrl"))?;
            Ok((None, Some(normalize_base_url(base_url)?)))
        }
    }
}

/// 只接受 `http://127.0.0.1:<port>` 或 `http://localhost:<port>`，统一归一为本机 IPv4 地址。
fn normalize_base_url(value: &str) -> Result<String, ComponentRuntimeError>
{
    let Some(rest) = value.get(..7).filter(|prefix| prefix.eq_ignore_ascii_case("http://")).map(|_| &value[7..])
    else
    {
        return Err(ComponentRuntimeError::loopback_address_invalid("基址必须是 http://127.0.0.1:<端口> 或 http://localhost:<端口>"));
    };
    let Some((host, port)) = rest.split_once(':')
    else
    {
        return Err(ComponentRuntimeError::loopback_address_invalid("基址缺少端口"));
    };
    if !matches!(host, "127.0.0.1" | "localhost")
    {
        return Err(ComponentRuntimeError::loopback_address_invalid("基址只允许本机 127.0.0.1 或 localhost"));
    }
    if port.is_empty() || !port.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(ComponentRuntimeError::loopback_address_invalid("基址端口无效"));
    }
    let port = port.parse::<u16>().ok().filter(|port| *port > 0).ok_or_else(|| ComponentRuntimeError::loopback_address_invalid("基址端口必须在 1–65535"))?;
    Ok(format!("http://127.0.0.1:{port}"))
}

/// 解析插件声明的凭据：没有记录名时不做注入与校验。
fn resolve_credential(request: &RegisterLoopbackChannel) -> Result<Option<Arc<ChannelCredential>>, ComponentRuntimeError>
{
    let Some(target) = request.credential_ref.as_deref().map(str::trim).filter(|value| !value.is_empty())
    else
    {
        return Ok(None);
    };
    let header_name = request.header_name.as_deref().map(str::trim).filter(|value| !value.is_empty()).ok_or_else(|| ComponentRuntimeError::loopback_credential_unavailable("声明了凭据但没有声明头名"))?;
    if !is_valid_header_name(header_name)
    {
        return Err(ComponentRuntimeError::loopback_credential_unavailable("凭据头名无效"));
    }
    Ok(Some(Arc::new(ChannelCredential { header_name: header_name.to_string(), secret: Zeroizing::new(read_credential(target)?) })))
}

/// 从 Windows Credential Manager 读取明文；缺少记录或内容为空都按不可用处理。
fn read_credential(target: &str) -> Result<String, ComponentRuntimeError>
{
    let Some(bytes) = crate::platform::windows::credentials::read_secret(target).map_err(|_| ComponentRuntimeError::loopback_credential_unavailable("Windows 凭据存储不可用"))?
    else
    {
        return Err(ComponentRuntimeError::loopback_credential_unavailable(format!("未找到凭据记录 {target}")));
    };
    if bytes.is_empty()
    {
        return Err(ComponentRuntimeError::loopback_credential_unavailable(format!("凭据记录内容为空 {target}")));
    }
    String::from_utf8(bytes).map_err(|_| ComponentRuntimeError::loopback_credential_unavailable("凭据内容不是有效文本"))
}

/// HTTP 头名只接受 token 字符，挡住 CR/LF 注入。
fn is_valid_header_name(name: &str) -> bool { !name.is_empty() && name.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'!' | b'#' | b'$' | b'%' | b'&' | b'\'' | b'*' | b'+' | b'-' | b'.' | b'^' | b'_' | b'`' | b'|' | b'~')) }

/// 常量时间比较，避免按字节提前返回泄露凭据前缀。
pub(crate) fn constant_time_eq(left: &[u8], right: &[u8]) -> bool
{
    if left.len() != right.len()
    {
        return false;
    }
    let mut difference = 0_u8;
    for (first, second) in left.iter().zip(right.iter())
    {
        difference |= first ^ second;
    }
    difference == 0
}

/// 请求头里是否存在与声明头名同名且值等于密钥的一项。
fn header_matches(headers: &[LoopbackHeader], header_name: &str, secret: &[u8]) -> bool { headers.iter().any(|header| header.name.eq_ignore_ascii_case(header_name) && constant_time_eq(header.value.as_bytes(), secret)) }

/// 移走已校验的凭据头，避免密钥明文随缓冲交给插件。
fn strip_credential_header(headers: &mut Vec<LoopbackHeader>, header_name: &str) { headers.retain(|header| !header.name.eq_ignore_ascii_case(header_name)); }

/// 请求方法与路径的边界校验；路径只允许本机相对路径。
fn validate_method(method: &str) -> Result<(), ComponentRuntimeError>
{
    if method.is_empty() || method.len() > 16 || !method.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(ComponentRuntimeError::loopback_address_invalid("请求方法无效"));
    }
    Ok(())
}

/// 路径必须以 `/` 开头且不含控制字符，防止拼出别的地址。
fn validate_request_path(path: &str) -> Result<(), ComponentRuntimeError>
{
    if path.is_empty() || path.len() > 2048 || !path.starts_with('/') || path.contains(['\r', '\n', '#']) || path.starts_with("//")
    {
        return Err(ComponentRuntimeError::loopback_address_invalid("请求路径无效"));
    }
    Ok(())
}

/// 启动监听线程；线程只持有缓冲状态与通知回调，注册表可随时替换或释放通道。
fn start_listen_server(listener: TcpListener, state: Arc<ChannelState>, notifier: Option<LoopbackNotifier>, channel_id: String, credential: Option<Arc<ChannelCredential>>) -> ListenServer
{
    let running = Arc::new(AtomicBool::new(true));
    let thread_running = Arc::clone(&running);
    let thread = std::thread::spawn(move || serve(listener, state, notifier, channel_id, credential, thread_running));
    ListenServer { running, thread: Some(thread) }
}

/// 监听循环：非阻塞接受连接，停止标志置 false 后最多一个轮询间隔退出。
fn serve(listener: TcpListener, state: Arc<ChannelState>, notifier: Option<LoopbackNotifier>, channel_id: String, credential: Option<Arc<ChannelCredential>>, running: Arc<AtomicBool>)
{
    if listener.set_nonblocking(true).is_err()
    {
        crate::log_warn!("loopback：监听器无法设为非阻塞，通道不可用，通道={channel_id}");
        return;
    }
    while running.load(Ordering::Relaxed)
    {
        match listener.accept()
        {
            Ok((stream, _)) =>
            {
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_read_timeout(Some(CONNECTION_TIMEOUT));
                let _ = stream.set_write_timeout(Some(CONNECTION_TIMEOUT));
                handle_connection(stream, &state, notifier.as_ref(), &channel_id, credential.as_deref());
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(ACCEPT_POLL),
            Err(error) =>
            {
                crate::log_warn!("loopback：接受连接失败，通道={channel_id}，错误={error}");
                std::thread::sleep(ACCEPT_POLL);
            }
        }
    }
}

/// 处理一条入站连接：解析原始请求、校验凭据、入缓冲并回响应。
fn handle_connection(mut stream: TcpStream, state: &ChannelState, notifier: Option<&LoopbackNotifier>, channel_id: &str, credential: Option<&ChannelCredential>)
{
    match read_inbound(&mut stream)
    {
        Ok(request) =>
        {
            if let Some(credential) = credential
                && !header_matches(&request.headers, &credential.header_name, credential.secret.as_bytes())
            {
                respond(&mut stream, "401 Unauthorized", "unauthorized"); // 校验失败不交给插件。
                return;
            }
            let mut request = request;
            if let Some(credential) = credential
            {
                strip_credential_header(&mut request.headers, &credential.header_name); // 明文不出宿主，插件拿不到密钥。
            }
            insert_request(state, notifier, channel_id, request);
            respond(&mut stream, "200 OK", "ok");
        }
        Err(failure) =>
        {
            let (status, body) = match failure
            {
                InboundFailure::BodyTooLarge | InboundFailure::HeaderTooLarge => ("413 Payload Too Large", "too large"),
                InboundFailure::Malformed => ("400 Bad Request", "bad request"),
                InboundFailure::Io => return,
            };
            count_dropped(state);
            respond(&mut stream, status, body);
        }
    }
}

/// 入站解析失败的原因分类。
enum InboundFailure
{
    /// 请求头超过上限。
    HeaderTooLarge,
    /// 请求体超过上限。
    BodyTooLarge,
    /// 请求行或头格式无效。
    Malformed,
    /// 连接读写失败或对端提前关闭。
    Io,
}

/// 解析完的一条原始请求，尚未分配序号。
struct RawRequest
{
    method: String,
    path: String,
    headers: Vec<LoopbackHeader>,
    body: String,
}

/// 有界解析 HTTP/1.1 请求；只支持单条请求与 Content-Length，chunked 视为无效。
fn read_inbound(stream: &mut TcpStream) -> Result<RawRequest, InboundFailure>
{
    let mut buffer = Vec::new(); // 已读取的原始字节。
    let mut chunk = [0_u8; 4096];
    let header_end = loop
    {
        if let Some(index) = find_header_end(&buffer)
        {
            break index;
        }
        if buffer.len() > HEADER_LIMIT
        {
            return Err(InboundFailure::HeaderTooLarge);
        }
        let read = stream.read(&mut chunk).map_err(|_| InboundFailure::Io)?;
        if read == 0
        {
            return Err(InboundFailure::Malformed);
        }
        buffer.extend_from_slice(&chunk[..read]);
    };
    let header_text = String::from_utf8_lossy(&buffer[..header_end]);
    let mut lines = header_text.split("\r\n");
    let request_line = lines.next().ok_or(InboundFailure::Malformed)?;
    let mut parts = request_line.split(' ');
    let method = parts.next().filter(|value| !value.is_empty()).ok_or(InboundFailure::Malformed)?.to_string();
    let path = parts.next().ok_or(InboundFailure::Malformed)?.to_string();
    let version = parts.next().ok_or(InboundFailure::Malformed)?;
    if !version.starts_with("HTTP/") || parts.next().is_some()
    {
        return Err(InboundFailure::Malformed);
    }
    let mut headers = Vec::new();
    let mut content_length = 0_usize;
    for line in lines
    {
        if line.is_empty()
        {
            continue;
        }
        let (name, value) = line.split_once(':').ok_or(InboundFailure::Malformed)?;
        let name = name.trim().to_string();
        let value = value.trim().to_string();
        if name.is_empty()
        {
            return Err(InboundFailure::Malformed);
        }
        if name.eq_ignore_ascii_case("transfer-encoding") && value.to_ascii_lowercase().contains("chunked")
        {
            return Err(InboundFailure::Malformed);
        }
        if name.eq_ignore_ascii_case("content-length")
        {
            content_length = value.parse().map_err(|_| InboundFailure::Malformed)?;
        }
        headers.push(LoopbackHeader { name, value });
    }
    if content_length > BODY_LIMIT
    {
        return Err(InboundFailure::BodyTooLarge);
    }
    let mut body = buffer[header_end..].to_vec();
    while body.len() < content_length
    {
        let read = stream.read(&mut chunk).map_err(|_| InboundFailure::Io)?;
        if read == 0
        {
            return Err(InboundFailure::Malformed);
        }
        body.extend_from_slice(&chunk[..read]);
        if body.len() > BODY_LIMIT
        {
            return Err(InboundFailure::BodyTooLarge);
        }
    }
    body.truncate(content_length);
    Ok(RawRequest { method, path, headers, body: String::from_utf8_lossy(&body).into_owned() })
}

/// 找到请求头结束的 `\r\n\r\n`，返回正文起点。
fn find_header_end(buffer: &[u8]) -> Option<usize> { buffer.windows(4).position(|window| window == b"\r\n\r\n").map(|index| index + 4) }

/// 回一条固定文本响应并关闭连接。
fn respond(stream: &mut TcpStream, status: &str, body: &str)
{
    let response = format!("HTTP/1.1 {status}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

/// 把解析好的原始请求放进缓冲；超过条数上限时丢最旧并计数。
fn insert_request(state: &ChannelState, notifier: Option<&LoopbackNotifier>, channel_id: &str, request: RawRequest) { insert_request_at(state, notifier, channel_id, request, Utc::now().timestamp()); }

/// 可注入时刻的入缓冲实现，测试用它覆盖 300 秒淘汰与条数上限。
fn insert_request_at(state: &ChannelState, notifier: Option<&LoopbackNotifier>, channel_id: &str, request: RawRequest, now: i64)
{
    let mut buffer = state.buffer.lock().unwrap_or_else(|error| error.into_inner());
    buffer.purge_expired(now);
    if buffer.entries.len() >= BUFFER_LIMIT
    {
        buffer.entries.pop_front();
        buffer.dropped += 1;
    }
    buffer.next_sequence += 1;
    let sequence = buffer.next_sequence;
    buffer.entries.push_back(LoopbackRequest { sequence, received_at: now, method: request.method, path: request.path, headers: request.headers, body: request.body });
    drop(buffer);
    if state.subscribed.load(Ordering::Relaxed)
        && let Some(notifier) = notifier
    {
        notifier(channel_id); // 事件不携带请求内容，插件收到后回来读取。
    }
}

/// 记录一次被丢弃的入站请求，不静默当成正常数据。
fn count_dropped(state: &ChannelState) { state.buffer.lock().unwrap_or_else(|error| error.into_inner()).dropped += 1; }

#[cfg(test)]
#[path = "loopback.test.rs"]
mod tests;
