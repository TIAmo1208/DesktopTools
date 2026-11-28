//! 桌面项目与围栏成员的位图取用。
//!
//! 取图**不重造**：直接复用 Shell 侧的图像 STA（[`crate::shell::ImageRuntime`]，自带代次失效、
//! 有界 LRU 与「无缩略图也缓存」的负缓存）。本模块只做四件事：
//! 1. 载入 Shell 的系统图标与显示名（`enumerate_desktop_items` 的既有结果，解码成原生位图）；
//! 2. 绘制线程**只读缓存**：命中就返回，未命中返回 `None`（调用方先画占位），不阻塞绘制；
//! 3. 目录与快捷方式本来没有缩略图，直接用系统图标；文件提取失败也回退系统图标；
//! 4. 每个引用只排队一次，结果落定后置位唤醒事件，由宿主重绘——**不新开轮询**。
//!
//! 尺寸口径（`docs/design/视觉规范.md`「未归类桌面项目和围栏平铺使用 48×48px 图像外框，
//! 围栏列表使用 36×36px」）：统一按 48 物理像素提取/取图标，列表 36 外框由 painter 等比缩小。
//!
//! ponytail: 诊断入口自带一份图像 STA；默认路径切换时应改为共享 `DesktopItems.images`。

use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread::JoinHandle,
};

use base64::{Engine, engine::general_purpose::STANDARD};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Threading::{CreateEventW, SetEvent};

use crate::platform::windows::shell_image::ShellImage;
use crate::shell::ImageRuntime;

/// 一次提取的结果通道（`None` 表示没有缩略图，属正常回退）。
type ThumbReply = mpsc::Receiver<Option<Arc<ShellImage>>>;

/// 缩略图提取边长（规范：48×48 图像外框，长边保持比例）。
pub const THUMB_PIXELS: u32 = 48;

/// 一个桌面项目在原生侧的显示事实（来自既有 Shell 枚举，只读）。
#[derive(Clone)]
pub struct ItemFact
{
    /// Shell 给出的系统显示名。
    pub name: String,
    /// 系统图标（解码后的预乘 BGRA）；取不到时只显示占位。
    pub icon: Option<Arc<ShellImage>>,
}

/// 一次取图的读数（诊断与记录用）。
#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct ThumbStats
{
    /// 拿到可用图像（缩略图或系统图标）的次数。
    pub hits: usize,
    /// 首次排队（真正触发提取）的引用数。
    pub requested: usize,
    /// 没有图像、先画占位的次数。
    pub misses: usize,
    /// 已落定（成功或失败）的结果数。
    pub settled: u64,
}

/// 位图来源：绘制线程只读缓存，提取在既有 Shell 图像 STA 上完成。
pub struct Thumbs
{
    runtime: ImageRuntime,                                 // 既有 Shell 图像 STA（不重写提取）。
    facts: HashMap<String, ItemFact>,                      // 系统显示名与图标（载入后只读）。
    event: isize,                                          // 自动复位唤醒事件（结果落定时置位）。
    inbox: Option<mpsc::Sender<(String, ThumbReply)>>,     // 交给收集线程等待的结果通道。
    collector: Option<JoinHandle<()>>,                     // 串行等待结果并累加代次。
    requested: Mutex<HashSet<String>>,                     // 已排队的引用，避免同一项重复提取。
    failed: Arc<Mutex<HashSet<String>>>,                   // 提取失败（含无缩略图）的引用：回退系统图标。
    settled: Arc<AtomicU64>,                               // 落定结果数（代次）。
    stats: Mutex<ThumbStats>,                              // 只做诊断计数。
}

impl Thumbs
{
    /// 载入系统显示名与图标：直接读既有 Shell 枚举结果，不新开目录扫描。
    ///
    /// 调用线程需要 COM 公寓（Shell 图像列表要求 STA）；失败时返回错误，由调用方决定是否只画占位。
    pub fn load_facts() -> Result<HashMap<String, ItemFact>, String>
    {
        let items = crate::platform::windows::enumerate_desktop_items()?;
        Ok(items
            .into_iter()
            .map(|item| {
                let icon = item.system_image.as_deref().and_then(decode_icon);
                (item.stable_reference, ItemFact { name: item.display_name, icon })
            })
            .collect())
    }

    /// 启动取图：内部自建自动复位事件，结果落定时置位；绘制线程用它等待重绘（不轮询）。
    pub fn start(facts: HashMap<String, ItemFact>) -> Result<Self, String>
    {
        let event = unsafe { CreateEventW(None, false, false, None) }.map_err(|error| error.to_string())?; // 自动复位，等待者放行后自动清位。
        let handle = event.0 as isize; // HANDLE 不跨线程传递，只传整数值。
        let signal = move || unsafe {
            let _ = SetEvent(HANDLE(handle as *mut _));
        };
        let runtime = match ImageRuntime::start()
        {
            Ok(runtime) => runtime,
            Err(error) =>
            {
                let _ = unsafe { CloseHandle(event) };
                return Err(error);
            }
        }; // 复用既有 STA，失败按环境问题上报。
        let settled = Arc::new(AtomicU64::new(0)); // 与收集线程共享的代次。
        let failed = Arc::new(Mutex::new(HashSet::new())); // 失败引用由收集线程登记。
        let counter = Arc::clone(&settled);
        let failures = Arc::clone(&failed);
        let (inbox, jobs) = mpsc::channel::<(String, ThumbReply)>();
        let collector = std::thread::Builder::new().name("desktop-tools-thumb-wake".into()).spawn(move || {
            // 图像 STA 本来就串行提取，这里按 FIFO 等下一条结果即可，不需要并发。
            for (reference, job) in jobs
            {
                match job.recv()
                {
                    Ok(Some(_)) => {} // 成功结果已在既有缓存里，绘制时按缓存命中取。
                    Ok(None) =>
                    {
                        if let Ok(mut failed) = failures.lock()
                        {
                            failed.insert(reference);
                        }
                    }
                    // 发送端在 STA 退出时一并释放：没有结果可报，直接结束。
                    Err(_) => break,
                }
                counter.fetch_add(1, Ordering::AcqRel);
                signal();
            }
        });
        let collector = match collector
        {
            Ok(collector) => collector,
            Err(error) =>
            {
                let _ = unsafe { CloseHandle(event) };
                return Err(error.to_string());
            }
        };
        Ok(Self { runtime, facts, event: handle, inbox: Some(inbox), collector: Some(collector), requested: Mutex::new(HashSet::new()), failed, settled, stats: Mutex::new(ThumbStats::default()) })
    }

    /// 唤醒事件的原始句柄值：绘制线程用它和消息一起等，**不新开轮询**。
    pub fn handle(&self) -> isize { self.event }

    /// 系统显示名（Shell 枚举结果）；未枚举到该项时返回 `None`，由调用方回退到引用路径。
    pub fn label(&self, reference: &str) -> Option<&str> { self.facts.get(reference).map(|fact| fact.name.as_str()) }

    /// 绘制用查询：命中缩略图或系统图标就返回；文件未就绪时返回 `None`（先画占位），目录与快捷方式直接给图标。
    pub fn image(&self, reference: &str) -> Option<Arc<ShellImage>>
    {
        let image = if let Some(thumbnail) = self.runtime.cached(reference)
        {
            Some(thumbnail)
        }
        else if self.is_failed(reference)
        {
            self.icon(reference)
        }
        else if wants_thumbnail(reference)
        {
            self.request(reference);
            None
        }
        else
        {
            // 目录、快捷方式与虚拟对象本来没有缩略图：规范要求显示系统图标，不再白跑一次提取。
            self.icon(reference)
        };
        match (&image, self.stats.lock())
        {
            (Some(_), Ok(mut stats)) => stats.hits += 1,
            (None, Ok(mut stats)) => stats.misses += 1,
            _ => {}
        }
        image
    }

    /// 系统图标（未枚举到该项时为 `None`，调用方画通用占位）。
    fn icon(&self, reference: &str) -> Option<Arc<ShellImage>> { self.facts.get(reference).and_then(|fact| fact.icon.clone()) }

    /// 是否已确认提取失败（失败引用只登记一次，之后每帧回退系统图标）。
    fn is_failed(&self, reference: &str) -> bool { self.failed.lock().map(|failed| failed.contains(reference)).unwrap_or(false) }

    /// 已落定结果数：绘制方比较前后值就知道是否需要重绘。
    pub fn settled(&self) -> u64 { self.settled.load(Ordering::Acquire) }

    /// 诊断读数。
    pub fn stats(&self) -> ThumbStats
    {
        let mut stats = self.stats.lock().map(|stats| *stats).unwrap_or_default();
        stats.settled = self.settled();
        stats
    }

    /// Shell 内容变化后整体失效：旧结果与排队一起丢弃（沿用既有代次语义）。
    pub fn invalidate(&self)
    {
        self.runtime.invalidate();
        if let Ok(mut requested) = self.requested.lock()
        {
            requested.clear();
        }
        if let Ok(mut failed) = self.failed.lock()
        {
            failed.clear();
        }
    }

    /// 只排队一次；排队或通道失败时回滚集合，下次绘制还会再试。
    fn request(&self, reference: &str)
    {
        let Ok(mut requested) = self.requested.lock()
        else
        {
            return;
        };
        if !requested.insert(reference.to_string())
        {
            return;
        }
        let queued = self.runtime.request(reference.to_string(), THUMB_PIXELS).ok().and_then(|receiver| self.inbox.as_ref().map(|inbox| inbox.send((reference.to_string(), receiver)).is_ok()));
        if queued == Some(true)
        {
            if let Ok(mut stats) = self.stats.lock()
            {
                stats.requested += 1;
            }
            return;
        }
        requested.remove(reference);
    }
}

impl Drop for Thumbs
{
    /// 先停 STA（等待者拿到 `None`），再关通道收掉收集线程。
    fn drop(&mut self)
    {
        self.runtime.shutdown();
        let inbox = self.inbox.take();
        drop(inbox);
        if let Some(collector) = self.collector.take()
        {
            let _ = collector.join();
        }
        let _ = unsafe { CloseHandle(HANDLE(self.event as *mut _)) };
    }
}

/// 只有「绝对路径的真实文件且不是快捷方式」才请求缩略图；其余按系统图标处理（与 Shell 提取的守卫一致）。
fn wants_thumbnail(reference: &str) -> bool
{
    let path = Path::new(reference);
    path.is_absolute() && !path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("lnk")) && path.is_file()
}

/// 把 Shell 的系统图标 data URL 解成预乘 BGRA 位图；格式不符返回 `None`（该引用只显示占位）。
fn decode_icon(data_url: &str) -> Option<Arc<ShellImage>>
{
    let encoded = data_url.strip_prefix("data:image/png;base64,")?;
    let bytes = STANDARD.decode(encoded).ok()?;
    let mut reader = png::Decoder::new(std::io::Cursor::new(bytes)).read_info().ok()?;
    let mut pixels = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut pixels).ok()?;
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight
    {
        return None;
    }
    let (width, height) = (info.width as i32, info.height as i32);
    let length = width.max(0) as usize * height.max(0) as usize * 4;
    if width <= 0 || height <= 0 || length > pixels.len()
    {
        return None;
    }
    pixels.truncate(length);
    for pixel in pixels.chunks_exact_mut(4)
    {
        // RGBA → 预乘 BGRA：与 `shell_image` 的像素约定一致，painter 直接按预乘 Alpha 绘制。
        let alpha = u32::from(pixel[3]);
        let (red, green, blue) = (u32::from(pixel[0]), u32::from(pixel[1]), u32::from(pixel[2]));
        pixel[0] = ((blue * alpha + 127) / 255) as u8;
        pixel[1] = ((green * alpha + 127) / 255) as u8;
        pixel[2] = ((red * alpha + 127) / 255) as u8;
    }
    Some(Arc::new(ShellImage { width, height, bgra: pixels, data_url: String::new() }))
}

#[cfg(test)]
mod tests
{
    use super::*;
    use std::time::{Duration, Instant};
    use windows::Win32::Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT};
    use windows::Win32::System::Threading::WaitForSingleObject;

    /// 读一次唤醒事件（自动复位事件在本次检查后清位）。
    fn signaled(thumbs: &Thumbs) -> u32
    {
        unsafe { WaitForSingleObject(HANDLE(thumbs.handle() as *mut _), 0).0 }
    }

    /// 等结果落定（测试内有限等待，不引入产品轮询）。
    fn wait_settled(thumbs: &Thumbs, count: u64)
    {
        let deadline = Instant::now() + Duration::from_secs(5);
        while thumbs.settled() < count && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// 用隔离图片跑真实提取：首帧未命中返回占位，结果落定后置位唤醒事件并命中缓存，重复请求只提取一次。
    #[test]
    fn thumbnail_arrives_off_thread_and_signals_once()
    {
        let path = std::env::temp_dir().join(format!("desktop-tools-thumbs-{}.png", std::process::id()));
        {
            let file = std::fs::File::create(&path).expect("创建样本"); // 独占样本，不碰用户文件。
            let mut encoder = png::Encoder::new(file, 96, 40); // 宽图可暴露方形拉伸。
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.write_header().expect("写头").write_image_data(&[10, 120, 200, 255].repeat(96 * 40)).expect("写像素");
        }
        let reference = path.to_str().expect("路径可转 UTF-8").to_string();
        let thumbs = Thumbs::start(HashMap::new()).expect("图像 STA 应可启动");
        // 首帧：未命中，返回占位，不阻塞。
        assert!(thumbs.image(&reference).is_none(), "首帧应返回占位");
        // 重复绘制不重复排队。
        for _ in 0..5
        {
            assert!(thumbs.image(&reference).is_none());
        }
        assert_eq!(thumbs.stats().requested, 1, "同一引用只排队一次");
        wait_settled(&thumbs, 1);
        assert_eq!(thumbs.settled(), 1, "只应有一次结果落定");
        assert_eq!(signaled(&thumbs), WAIT_OBJECT_0.0, "落定应置位唤醒事件");
        assert_eq!(signaled(&thumbs), WAIT_TIMEOUT.0, "事件自动复位，只能唤醒一次");
        let image = thumbs.image(&reference).expect("落定后应命中缓存");
        assert_eq!((image.width, image.height), (48, 20), "长边 48，比例不变");
        assert!(thumbs.stats().requested == 1, "命中缓存不再排队");
        drop(thumbs);
        std::fs::remove_file(&path).ok();
    }

    /// 快捷方式与目录不请求缩略图：直接用 Shell 枚举带来的系统图标（规范：失败显示系统图标）。
    #[test]
    fn shortcuts_and_directories_use_system_icon()
    {
        let icon = Arc::new(ShellImage { width: 2, height: 2, bgra: vec![255; 16], data_url: String::new() });
        let mut facts = HashMap::new();
        facts.insert("C:\\Users\\Public\\Desktop\\sample.lnk".to_string(), ItemFact { name: "示例快捷方式".to_string(), icon: Some(Arc::clone(&icon)) });
        let thumbs = Thumbs::start(facts).expect("图像 STA 应可启动");
        let shortcut = "C:\\Users\\Public\\Desktop\\sample.lnk";
        let image = thumbs.image(shortcut).expect("快捷方式应直接给系统图标");
        assert!(Arc::ptr_eq(&image, &icon));
        assert_eq!(thumbs.stats().requested, 0, "快捷方式不应排队提取缩略图");
        assert_eq!(thumbs.label(shortcut), Some("示例快捷方式"), "显示名取 Shell 枚举结果");
        assert!(thumbs.image("C:\\Windows").is_none(), "没有枚举到的目录不画假图标");
    }

    /// 文件提取失败后回退系统图标：失败只登记一次，不每帧重排。
    #[test]
    fn failed_file_falls_back_to_system_icon()
    {
        let icon = Arc::new(ShellImage { width: 2, height: 2, bgra: vec![255; 16], data_url: String::new() });
        let sample = std::env::temp_dir().join(format!("desktop-tools-thumbs-{}.dat", std::process::id()));
        std::fs::write(&sample, b"desktop-tools no preview handler").expect("写样本"); // 没有缩略图处理器的真实文件。
        let reference = sample.to_str().expect("路径可转 UTF-8").to_string();
        let mut facts = HashMap::new();
        facts.insert(reference.clone(), ItemFact { name: "缺失.png".to_string(), icon: Some(Arc::clone(&icon)) });
        let thumbs = Thumbs::start(facts).expect("图像 STA 应可启动");
        assert!(thumbs.image(&reference).is_none(), "首帧先画占位");
        wait_settled(&thumbs, 1);
        assert_eq!(thumbs.settled(), 1, "失败也要落定，避免无限重排");
        let image = thumbs.image(&reference).expect("失败后回退系统图标");
        assert!(Arc::ptr_eq(&image, &icon));
        assert_eq!(thumbs.stats().requested, 1, "失败只排队一次");
        drop(thumbs);
        std::fs::remove_file(&sample).ok();
    }
}
