//! Windows 输出设备枚举与系统音频频谱采集。

use std::str::FromStr;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc::{Receiver, Sender, channel},
};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{DeviceId, FromSample, Sample, SampleFormat, SizedSample, Stream, StreamConfig};
use realfft::{RealFftPlanner, RealToComplex};
use serde::Serialize;
use tauri::Emitter;
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::Media::Audio::{DEVICE_STATE, EDataFlow, ERole, IMMDeviceEnumerator, IMMNotificationClient, IMMNotificationClient_Impl, MMDeviceEnumerator, eConsole, eRender};
use windows::Win32::System::Com::{CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx};

/// 每帧频谱数据的事件名。
pub const SPECTRUM_EVENT: &str = "audio-spectrum-changed";
/// 实际采集设备变化的事件名。
pub const SPECTRUM_DEVICE_EVENT: &str = "audio-spectrum-device-changed";
/// 系统输出设备列表变化的事件名，载荷为最新设备列表。
pub const DEVICES_EVENT: &str = "audio-devices-changed";

const FFT_SIZE: usize = 2048; // 48kHz 下约 23Hz 的频率分辨率。
const WINDOW_CAPACITY: usize = FFT_SIZE * 2; // 只保留最近一窗采样，覆盖写入不需要搬运。
const DEVICE_DEBOUNCE: Duration = Duration::from_millis(300); // 合并一次插拔产生的多条端点通知。
const DEVICE_FALLBACK_POLL: Duration = Duration::from_secs(10); // 通知漏发时的兜底检查。
const DEVICE_RETRY: Duration = Duration::from_millis(500); // 重建失败后的首次重试间隔。
const DEVICE_RETRY_MAX: Duration = Duration::from_secs(2); // 连续失败时逐步拉长到这个上限。
const MIN_FREQ: f32 = 40.0; // 最低频段中心。
const MAX_FREQ: f32 = 16000.0; // 最高频段中心，超过奈奎斯特频率时自动收敛。
const FLOOR_DB: f32 = 60.0; // 显示动态范围，低于该值视为静音。

/// 设置页可选择的输出设备。
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputDevice
{
    pub id: String,       // cpal 稳定设备标识，跨重启保持不变。
    pub name: String,     // 面向用户的设备名称。
    pub is_default: bool, // 是否为当前系统默认输出设备。
}

/// 一次采集连接的实际状态。
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpectrumStatus
{
    pub running: bool,       // 采集线程是否在运行。
    pub device_id: String,   // 实际采集的设备标识。
    pub device_name: String, // 实际采集的设备名称。
    pub fallback: bool,      // 偏好设备不可用时回退到系统默认。
}

/// 一帧频谱数据。
#[derive(Clone, Debug, Serialize)]
pub struct SpectrumFrame
{
    pub bins: Vec<u8>, // 0–255 的对数幅度，索引从低频到高频。
}

/// 采样定长环形窗口：回调只覆盖写入，分析时一次性取出最近一窗。
struct SampleWindow
{
    values: Box<[f32; WINDOW_CAPACITY]>, // 回绕保存的最近采样。
    next: usize,                         // 下一次写入位置。
}

/// 提供无分支写入与连续窗口拷贝。
impl SampleWindow
{
    /// 创建全零窗口，启动瞬间即可分析静音帧。
    fn new() -> Self { Self { values: Box::new([0.0; WINDOW_CAPACITY]), next: 0 } }

    /// 按到达顺序覆盖写入一个单声道采样。
    fn push(&mut self, value: f32)
    {
        self.values[self.next] = value;
        self.next = (self.next + 1) % WINDOW_CAPACITY;
    }

    /// 把最近 FFT_SIZE 个采样按时间顺序复制到分析缓冲。
    fn copy_latest(&self, target: &mut [f32])
    {
        let start = (self.next + WINDOW_CAPACITY - FFT_SIZE) % WINDOW_CAPACITY; // 窗口起点。
        let head = (WINDOW_CAPACITY - start).min(FFT_SIZE); // 回绕点之前的一段。
        target[..head].copy_from_slice(&self.values[start..start + head]);
        if head < FFT_SIZE
        {
            target[head..].copy_from_slice(&self.values[..FFT_SIZE - head]);
        }
    }
}

/// 固定窗口的实数 FFT 分析器，复用缓冲区避免每帧分配。
struct Analyzer
{
    plan: Arc<dyn RealToComplex<f32>>,               // 可复用的正向实数变换。
    input: Vec<f32>,                                 // 加窗后的时域窗口。
    output: Vec<realfft::num_complex::Complex<f32>>, // 单边频域结果。
    window: Vec<f32>,                                // 汉宁窗，降低截断泄漏。
    bins: Vec<u8>,                                   // 当前帧的分段幅度。
    sample_rate: f32,                                // 设备实际采样率决定频段边界。
}

impl Analyzer
{
    /// 按采样率与段数创建一次变换规划。
    fn new(sample_rate: u32, bars: usize) -> Self
    {
        let mut planner = RealFftPlanner::<f32>::new(); // 仅首次启动时规划。
        let plan = planner.plan_fft_forward(FFT_SIZE);
        let window = (0..FFT_SIZE).map(|index| 0.5 - 0.5 * (std::f32::consts::TAU * index as f32 / (FFT_SIZE as f32 - 1.0)).cos()).collect(); // 周期化汉宁窗。
        Self { input: plan.make_input_vec(), output: plan.make_output_vec(), window, bins: vec![0; bars], plan, sample_rate: sample_rate as f32 }
    }

    /// 把最近窗口转换为从低频到高频的分段对数幅度。
    fn analyze(&mut self, samples: &SampleWindow) -> &[u8]
    {
        samples.copy_latest(&mut self.input); // 未播放时段窗口保持全零，得到全静音频谱。
        for (value, weight) in self.input.iter_mut().zip(&self.window)
        {
            *value *= weight;
        }
        if self.plan.process(&mut self.input, &mut self.output).is_err()
        {
            self.bins.fill(0);
            return &self.bins;
        }
        let scale = FFT_SIZE as f32 * 0.25; // 实数 FFT 单边幅度归一化系数。
        let bin_hz = self.sample_rate / FFT_SIZE as f32; // 每个频点的频率宽度。
        let top = MAX_FREQ.min(self.sample_rate * 0.5); // 超过奈奎斯特频率的频段没有意义。
        let bars = self.bins.len();
        for (index, value) in self.bins.iter_mut().enumerate()
        {
            let low = MIN_FREQ * (top / MIN_FREQ).powf(index as f32 / bars as f32); // 对数刻度让低频占用更多段。
            let high = MIN_FREQ * (top / MIN_FREQ).powf((index + 1) as f32 / bars as f32);
            let start = ((low / bin_hz) as usize).clamp(1, FFT_SIZE / 2 - 1); // 跳过直流分量。
            let end = (((high / bin_hz) as usize).max(start + 1)).min(FFT_SIZE / 2);
            let peak = self.output[start..end].iter().fold(0.0f32, |peak, bin| peak.max(bin.norm())); // 每段取峰值，保留瞬态。
            let db = 20.0 * (peak / scale).max(1e-7).log10();
            *value = (((db + FLOOR_DB) / FLOOR_DB).clamp(0.0, 1.0) * 255.0).round() as u8;
        }
        &self.bins
    }
}

/// 一次成功建立的采集连接，drop 即释放回环流。
struct Capture
{
    id: String,                        // 当前实际设备标识。
    name: String,                      // 当前实际设备名称。
    fallback: bool,                    // 偏好设备是否不可用。
    samples: Arc<Mutex<SampleWindow>>, // 采集回调写入的单声道采样。
    errors: Arc<AtomicBool>,           // 采集流是否报告过失效，供重建流程消费。
    analyzer: Analyzer,                // 与设备采样率匹配的分析器。
    _stream: Stream,                   // 持有期间持续接收数据。
}

/// 采集线程与停止信号。
struct Runner
{
    stop: Arc<AtomicBool>,               // 通知线程退出。
    thread: std::thread::JoinHandle<()>, // 用于停止时等待资源释放。
}

/// 设备通知线程句柄：停止时先唤醒阻塞中的接收再等待退出。
struct WatchHandle
{
    stop: Arc<AtomicBool>,               // 通知监听线程退出。
    wake: Sender<()>,                    // 唤醒阻塞在接收上的监听线程。
    thread: std::thread::JoinHandle<()>, // 等待监听线程注销回调后退出。
}

static RUNNER: Mutex<Option<Runner>> = Mutex::new(None); // 同一进程只允许一组采集线程。
static DEVICE_EPOCH: AtomicU64 = AtomicU64::new(0); // 每批设备通知递增一次。
static WATCH_SETTINGS: AtomicBool = AtomicBool::new(false); // 设置面板是否在监听设备变化。
static WATCH_CAPTURE: AtomicBool = AtomicBool::new(false); // 采集线程是否在监听设备变化。
static WATCH: Mutex<Option<WatchHandle>> = Mutex::new(None); // 进程内唯一的设备通知线程。

/// 设备展示名称，优先使用结构化描述。
fn device_name(device: &cpal::Device) -> String { device.description().map(|description| description.name().to_string()).unwrap_or_else(|_| device.to_string()) }

/// 列出全部输出设备，供设置页选择。
pub fn output_devices() -> Vec<OutputDevice>
{
    let host = cpal::default_host();
    let default_id = host.default_output_device().and_then(|device| device.id().ok()).map(|id| id.to_string()); // 用于标记跟随目标。
    let Ok(devices) = host.output_devices()
    else
    {
        return Vec::new(); // 枚举失败时返回空列表，设置页保留跟随系统选项。
    };
    let mut result = Vec::new();
    for device in devices
    {
        let Ok(id) = device.id()
        else
        {
            continue; // 已拔出的设备会在这里失败。
        };
        let id = id.to_string();
        result.push(OutputDevice { is_default: default_id.as_deref() == Some(id.as_str()), name: device_name(&device), id });
    }
    result
}

/// Core Audio 端点通知回调：只在系统线程上转发唤醒信号，不做枚举等重活。
#[windows::core::implement(IMMNotificationClient)]
struct DeviceNotificationClient
{
    wake: Sender<()>, // 交给通知监听线程去抖后统一处理。
}

impl DeviceNotificationClient
{
    /// 一次通知只表达“设备集合可能变了”，具体差异由监听线程重新枚举得到。
    fn notify(&self) { let _ = self.wake.send(()); }
}

impl IMMNotificationClient_Impl for DeviceNotificationClient_Impl
{
    /// 设备状态变化（拔出、禁用、缺失）都可能改变可用列表。
    fn OnDeviceStateChanged(&self, _device: &windows::core::PCWSTR, _state: DEVICE_STATE) -> windows::core::Result<()>
    {
        self.notify();
        Ok(())
    }

    /// 新设备接入。
    fn OnDeviceAdded(&self, _device: &windows::core::PCWSTR) -> windows::core::Result<()>
    {
        self.notify();
        Ok(())
    }

    /// 设备移除。
    fn OnDeviceRemoved(&self, _device: &windows::core::PCWSTR) -> windows::core::Result<()>
    {
        self.notify();
        Ok(())
    }

    /// 默认输出设备切换会改变列表里的系统默认标记。
    fn OnDefaultDeviceChanged(&self, flow: EDataFlow, role: ERole, _device: &windows::core::PCWSTR) -> windows::core::Result<()>
    {
        if flow == eRender && role == eConsole
        {
            self.notify();
        }
        Ok(())
    }

    /// 属性变化过于频繁且不影响设备列表，忽略。
    fn OnPropertyValueChanged(&self, _device: &windows::core::PCWSTR, _key: &PROPERTYKEY) -> windows::core::Result<()> { Ok(()) }
}

/// 监听线程主体：注册端点通知，去抖后重新枚举，仅在列表真正变化时投递事件。
fn watch_thread(app: tauri::AppHandle, receiver: Receiver<()>, wake: Sender<()>, stop: Arc<AtomicBool>)
{
    // 端点通知依赖 COM；本线程只负责去抖与枚举，COM 回调在系统线程上只发信号。
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    let enumerator: IMMDeviceEnumerator = match unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
    {
        Ok(enumerator) => enumerator,
        Err(error) =>
        {
            crate::log_warn!("音频：设备通知初始化失败，错误={error}");
            return;
        }
    };
    let client: IMMNotificationClient = DeviceNotificationClient { wake }.into();
    if let Err(error) = unsafe { enumerator.RegisterEndpointNotificationCallback(&client) }
    {
        crate::log_warn!("音频：设备通知注册失败，错误={error}");
        return;
    }
    let mut last = output_devices(); // 以监听启动时的列表为基线。
    crate::log_info!("音频：设备通知监听已启动");
    while !stop.load(Ordering::Relaxed)
    {
        if receiver.recv().is_err() || stop.load(Ordering::Relaxed)
        {
            break;
        }
        // 去抖：合并同一物理动作产生的多条通知。
        while !stop.load(Ordering::Relaxed) && receiver.recv_timeout(DEVICE_DEBOUNCE).is_ok()
        {}
        let devices = output_devices();
        if devices != last
        {
            crate::log_debug!("音频：设备列表变化，数量={}", devices.len());
            last = devices.clone();
            let _ = app.emit(DEVICES_EVENT, devices);
        }
        DEVICE_EPOCH.fetch_add(1, Ordering::Relaxed); // 枚举完成后再唤醒采集线程，避免它读到切换中的默认设备。
    }
    unsafe {
        let _ = enumerator.UnregisterEndpointNotificationCallback(&client);
    }
    crate::log_info!("音频：设备通知监听已停止");
}

/// 创建通知线程；注册结果由线程自己记录日志，失败时设置页仍可读取打开面板时的快照。
fn spawn_watch(app: tauri::AppHandle) -> Option<WatchHandle>
{
    let stop = Arc::new(AtomicBool::new(false));
    let (wake, receiver) = channel();
    let flag = stop.clone();
    let client_wake = wake.clone(); // 回调侧唤醒入口。
    let thread = std::thread::Builder::new().name("audio-device-watch".to_string()).spawn(move || watch_thread(app, receiver, client_wake, flag)).map_err(|error| crate::log_warn!("音频：设备通知线程创建失败，错误={error}")).ok()?;
    Some(WatchHandle { stop, wake, thread })
}

/// 按需维护通知线程；设置面板与采集线程各持有一个开关，全部关闭后释放线程。
fn sync_watch(app: Option<&tauri::AppHandle>)
{
    let release = {
        let mut slot = WATCH.lock().unwrap_or_else(|error| error.into_inner());
        if WATCH_SETTINGS.load(Ordering::Relaxed) || WATCH_CAPTURE.load(Ordering::Relaxed)
        {
            if slot.is_none()
                && let Some(app) = app
            {
                *slot = spawn_watch(app.clone());
            }
            None
        }
        else
        {
            slot.take()
        }
    }; // 锁内只改状态，等待线程退出放到锁外。
    if let Some(handle) = release
    {
        handle.stop.store(true, Ordering::Relaxed);
        let _ = handle.wake.send(());
        let _ = handle.thread.join();
    }
}

/// 设置面板开关设备通知监听。
pub fn watch_settings(app: &tauri::AppHandle, active: bool)
{
    WATCH_SETTINGS.store(active, Ordering::Relaxed);
    sync_watch(Some(app));
}

/// 优先使用保存的设备，不可用时回退系统默认。
fn resolve_device(host: &cpal::Host, preferred: &str) -> Result<(cpal::Device, bool), String>
{
    if !preferred.is_empty()
        && let Ok(id) = DeviceId::from_str(preferred)
        && let Some(device) = host.device_by_id(&id)
    {
        return Ok((device, false)); // 保存的设备仍然存在时始终使用它。
    }
    host.default_output_device().map(|device| (device, !preferred.is_empty())).ok_or_else(|| "没有可用的音频输出设备".to_string())
}

/// 按帧混合为单声道并写入环形窗口。
fn push_frames<T>(data: &[T], channels: usize, sink: &Mutex<SampleWindow>)
where
    T: SizedSample + Copy,
    f32: FromSample<T>,
{
    let channels = channels.max(1);
    let mut window = sink.lock().unwrap_or_else(|error| error.into_inner()); // 与发布线程共享的窗口。
    for frame in data.chunks_exact(channels)
    {
        let mono = frame.iter().map(|sample| f32::from_sample(*sample)).sum::<f32>() / channels as f32; // 多声道取平均。
        window.push(mono);
    }
}

/// 以设备默认格式建立回环输入流；输出设备作为输入时 cpal 自动启用 loopback。
fn build_stream<T>(device: &cpal::Device, config: StreamConfig, samples: Arc<Mutex<SampleWindow>>, errors: Arc<AtomicBool>) -> Result<Stream, cpal::Error>
where
    T: SizedSample + Copy,
    f32: FromSample<T>,
{
    let channels = config.channels as usize; // 回调按交错帧给出样本。
    // 回环流在启停与无人播放时会上报缓冲欠载，属预期行为；只把设备失效当作重建信号。
    device.build_input_stream::<T, _, _>(
        config,
        move |data: &[T], _| push_frames(data, channels, &samples),
        move |error| {
            if matches!(error.kind(), cpal::ErrorKind::DeviceNotAvailable | cpal::ErrorKind::StreamInvalidated)
            {
                errors.store(true, Ordering::Relaxed);
            }
            crate::log_debug!("音频：采集流回调，错误={error}");
        },
        None,
    )
}

/// 建立一次采集连接，失败时返回可直接展示的原因。
fn attach(host: &cpal::Host, preferred: &str, bars: usize) -> Result<Capture, String>
{
    let (device, fallback) = resolve_device(host, preferred)?;
    let id = device.id().map(|id| id.to_string()).map_err(|error| format!("读取音频设备标识失败：{error}"))?;
    let name = device_name(&device);
    let supported = device.default_output_config().map_err(|error| format!("读取音频格式失败：{error}"))?;
    let format = supported.sample_format();
    let config = supported.config();
    let sample_rate = config.sample_rate;
    let samples = Arc::new(Mutex::new(SampleWindow::new()));
    let sink = samples.clone();
    let errors = Arc::new(AtomicBool::new(false));
    let stream_errors = errors.clone();
    let stream = match format
    {
        SampleFormat::F32 => build_stream::<f32>(&device, config, sink, stream_errors),
        SampleFormat::F64 => build_stream::<f64>(&device, config, sink, stream_errors),
        SampleFormat::I16 => build_stream::<i16>(&device, config, sink, stream_errors),
        SampleFormat::U16 => build_stream::<u16>(&device, config, sink, stream_errors),
        SampleFormat::I32 => build_stream::<i32>(&device, config, sink, stream_errors),
        SampleFormat::U32 => build_stream::<u32>(&device, config, sink, stream_errors),
        SampleFormat::I8 => build_stream::<i8>(&device, config, sink, stream_errors),
        SampleFormat::U8 => build_stream::<u8>(&device, config, sink, stream_errors),
        other => return Err(format!("不支持的音频采样格式：{other}")),
    }
    .map_err(|error| format!("启动音频采集失败：{error}"))?;
    stream.play().map_err(|error| format!("启动音频采集失败：{error}"))?;
    Ok(Capture { id, name, fallback, samples, errors, analyzer: Analyzer::new(sample_rate, bars), _stream: stream })
}

/// 按帧率发布频谱，并在设备通知或兜底检查点按规则重建设备。
fn run(app: tauri::AppHandle, host: cpal::Host, preferred: String, bars: usize, interval: Duration, capture: Capture, stop: Arc<AtomicBool>)
{
    let mut capture = Some(capture); // 重建期间可以暂时没有可用连接。
    let mut seen_epoch = DEVICE_EPOCH.load(Ordering::Relaxed); // 已处理到哪一批设备通知。
    let mut next_check = Instant::now(); // 启动后立即核对一次设备规则。
    let mut retry = DEVICE_RETRY; // 连续失败时逐步拉长重试间隔。
    while !stop.load(Ordering::Relaxed)
    {
        let started = Instant::now();
        let epoch = DEVICE_EPOCH.load(Ordering::Relaxed); // 通知线程每批通知递增一次。
        if epoch != seen_epoch || started >= next_check
        {
            seen_epoch = epoch;
            next_check = started + DEVICE_FALLBACK_POLL; // 兜底检查防止通知漏发。
            let current_id = capture.as_ref().map(|current| current.id.clone()); // 当前实际设备标识。
            let failed = capture.as_ref().is_some_and(|current| current.errors.swap(false, Ordering::Relaxed)); // 采集流已失效时即使设备标识没变也要重建。
            let current_valid = current_id.as_deref().is_some_and(|id| DeviceId::from_str(id).ok().and_then(|id| host.device_by_id(&id)).is_some()); // 当前设备是否仍然可用。
            let desired = resolve_device(&host, &preferred).ok().and_then(|(device, _)| device.id().ok().map(|id| id.to_string())); // 规则要求的设备标识。
            let needs_rebuild = failed || !current_valid || desired.as_deref() != current_id.as_deref(); // 只有设备消失、流失效或规则目标变化才切换，新设备接入不触发。
            if needs_rebuild
            {
                // 旧设备已失效或目标就是同一设备时先释放旧流；切换到另一台可用设备时先建新流，失败不影响现有出图。
                if failed || !current_valid || desired.as_deref() == current_id.as_deref()
                {
                    drop(capture.take());
                }
                match attach(&host, &preferred, bars)
                {
                    Ok(next) =>
                    {
                        let status = SpectrumStatus { running: true, device_id: next.id.clone(), device_name: next.name.clone(), fallback: next.fallback };
                        crate::log_info!("音频：采集设备切换为 {}", next.name);
                        capture = Some(next); // 释放上一路流，新流已就绪。
                        retry = DEVICE_RETRY;
                        let _ = app.emit(SPECTRUM_DEVICE_EVENT, status);
                    }
                    Err(error) =>
                    {
                        crate::log_warn!("音频：重建采集失败，错误={error}");
                        next_check = started + retry; // 设备切换瞬间可能尚未就绪，短间隔重试。
                        retry = (retry * 2).min(DEVICE_RETRY_MAX);
                    }
                }
            }
        }
        // 跟随桌面刷新规则：锁屏或全部暂停时不发布，前端保持最后一帧。
        if !crate::platform::windows::refresh_policy::is_refresh_paused()
            && let Some(current) = capture.as_mut()
        {
            let samples = current.samples.lock().unwrap_or_else(|error| error.into_inner()); // 只读引用，避免每帧额外拷贝。
            let bins = current.analyzer.analyze(&samples).to_vec(); // 分析器复用内部缓冲。
            drop(samples);
            let _ = app.emit(SPECTRUM_EVENT, SpectrumFrame { bins });
        }
        let elapsed = started.elapsed();
        if elapsed < interval
        {
            std::thread::sleep(interval - elapsed);
        }
    }
}

/// 采集线程入口：COM 环境、设备枚举与音频流都在本线程建立，重建时才不会跨线程拿不到音频客户端。
fn capture_thread(app: tauri::AppHandle, preferred: String, bars: usize, interval: Duration, stop: Arc<AtomicBool>, ready: Sender<Result<SpectrumStatus, String>>)
{
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    } // WASAPI 客户端要求调用线程不处于单线程套间。
    let host = cpal::default_host();
    let capture = match attach(&host, &preferred, bars)
    {
        Ok(capture) => capture,
        Err(error) =>
        {
            let _ = ready.send(Err(error));
            return;
        }
    };
    let status = SpectrumStatus { running: true, device_id: capture.id.clone(), device_name: capture.name.clone(), fallback: capture.fallback };
    let _ = ready.send(Ok(status));
    run(app, host, preferred, bars, interval, capture, stop);
}

/// 启动采集；重复调用会先停止旧线程。
pub fn start(app: tauri::AppHandle, preferred: String, bars: u8, fps: u8) -> Result<SpectrumStatus, String>
{
    crate::log_info!("音频：收到启动请求，设备={preferred}，段数={bars}，帧率={fps}");
    stop();
    let bars = bars.clamp(8, 64) as usize; // 段数上限保证单帧事件足够小。
    let interval = Duration::from_millis((1000 / fps.clamp(10, 60) as u64).max(1)); // 帧率收敛到可用的整数毫秒间隔。
    WATCH_CAPTURE.store(true, Ordering::Relaxed); // 采集靠设备通知即时重建，不再自己轮询。
    sync_watch(Some(&app));
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let (ready, readiness) = channel();
    let thread_app = app.clone();
    let thread = std::thread::spawn(move || capture_thread(thread_app, preferred, bars, interval, flag, ready));
    let status = match readiness.recv()
    {
        Ok(Ok(status)) => status,
        Ok(Err(error)) =>
        {
            disable_capture_watch();
            let _ = thread.join();
            return Err(error);
        }
        Err(_) =>
        {
            disable_capture_watch();
            let _ = thread.join();
            return Err("音频采集线程启动失败".to_string());
        }
    }; // 首次建流在线程内完成，启动错误仍同步返回给前端。
    *RUNNER.lock().unwrap_or_else(|error| error.into_inner()) = Some(Runner { stop, thread });
    crate::log_info!("音频：频谱采集已启动，设备={}，段数={bars}，帧率={fps}", status.device_name);
    Ok(status)
}

/// 关闭采集线程对设备通知的需求。
fn disable_capture_watch()
{
    WATCH_CAPTURE.store(false, Ordering::Relaxed);
    sync_watch(None);
}

/// 停止采集线程，重复调用无副作用。
pub fn stop()
{
    let runner = RUNNER.lock().unwrap_or_else(|error| error.into_inner()).take(); // 取出唯一运行实例。
    if let Some(runner) = runner
    {
        runner.stop.store(true, Ordering::Relaxed);
        let _ = runner.thread.join();
        crate::log_info!("音频：频谱采集已停止");
    }
    disable_capture_watch();
}

/// 当前是否有采集线程在运行。
pub fn running() -> bool { RUNNER.lock().unwrap_or_else(|error| error.into_inner()).is_some() }

#[cfg(test)]
mod tests
{
    use super::{Analyzer, FFT_SIZE, SampleWindow};

    /// 静音输入必须得到全零分段，避免把底噪画成频谱。
    #[test]
    fn silent_window_produces_zero_bins()
    {
        let mut analyzer = Analyzer::new(48000, 32);
        let window = SampleWindow::new(); // 未写入过采样的空窗口。
        let bins = analyzer.analyze(&window).to_vec();
        assert_eq!(bins.len(), 32);
        assert!(bins.iter().all(|value| *value == 0));
    }

    /// 单频正弦应在对应频段产生明显能量，且远高于无关频段。
    #[test]
    fn sine_wave_peaks_in_matching_bar()
    {
        let mut analyzer = Analyzer::new(48000, 32);
        let frequency = 1000.0f32; // 1kHz 测试音。
        let mut window = SampleWindow::new(); // 连续写入一整个分析窗口的采样。
        for index in 0..FFT_SIZE
        {
            window.push((std::f32::consts::TAU * frequency * index as f32 / 48000.0).sin());
        }
        let bins = analyzer.analyze(&window).to_vec();
        let peak = bins.iter().copied().max().unwrap_or(0); // 最强频段。
        assert!(peak > 120, "峰值过低：{peak}");
        assert!(bins.iter().filter(|value| **value > 60).count() <= 6, "能量泄漏过宽：{bins:?}");
    }

    /// 环形窗口回绕后仍按时间顺序取出最近一窗，跨回绕点不能错位。
    #[test]
    fn window_keeps_order_across_wrap()
    {
        let mut window = SampleWindow::new();
        for index in 0..(super::WINDOW_CAPACITY + FFT_SIZE / 2)
        {
            window.push(index as f32); // 连续递增序列便于检查顺序。
        }
        let mut target = vec![0.0f32; FFT_SIZE];
        window.copy_latest(&mut target);
        let first = (super::WINDOW_CAPACITY + FFT_SIZE / 2 - FFT_SIZE) as f32; // 窗口起点对应的采样值。
        assert_eq!(target[0], first);
        assert_eq!(target[FFT_SIZE - 1], first + FFT_SIZE as f32 - 1.0);
        assert!(target.windows(2).all(|pair| pair[1] - pair[0] == 1.0));
    }

    /// 真实回环采集依赖本机音频设备，默认忽略，手动执行验证设备链路可用。
    #[test]
    #[ignore]
    fn loopback_device_opens()
    {
        let host = cpal::default_host();
        let capture = super::attach(&host, "", 32).expect("建立系统音频回环采集");
        assert!(!capture.id.is_empty());
        assert!(!capture.name.is_empty());
    }
}
