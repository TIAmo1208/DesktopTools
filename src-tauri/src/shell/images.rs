use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::JoinHandle;

use crate::platform::windows::shell_image::{ShellImage, extract_thumbnail};
use crate::platform::windows::{initialize_shell_apartment, uninitialize_shell_apartment};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Threading::{CreateEventW, SetEvent};
use windows::Win32::UI::WindowsAndMessaging::{DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, TranslateMessage, WM_QUIT};

type ImageKey = (String, u32, u64); // Shell 身份、物理像素和失效代次共同标识一次读取。
type ImageResult = Option<Arc<ShellImage>>; // 无缩略图也缓存，避免重复调用处理器。

/// 同步保护有界 LRU 和待处理请求，慢提取期间不持锁。
#[derive(Default)]
struct ImageQueue
{
    generation: u64,                                            // 任意 Shell 内容变化后使旧结果失效。
    stopped: bool,                                              // 关闭后拒绝入队。
    order: VecDeque<ImageKey>,                                  // 尚未执行的去重请求。
    waiting: HashMap<ImageKey, Vec<mpsc::Sender<ImageResult>>>, // 同一请求的结果接收者。
    cache: VecDeque<(ImageKey, ImageResult)>,                   // 从最久未用到最近使用排列。
}

impl ImageQueue
{
    /// 插入结果并限制像素与编码后的总缓存量。
    fn remember(&mut self, key: ImageKey, image: ImageResult)
    {
        self.cache.push_back((key, image));
        while self.cache.len() > 256 || self.cache.iter().filter_map(|(_, image)| image.as_ref()).map(|image| image.bgra.len() + image.data_url.len()).sum::<usize>() > 16 * 1024 * 1024
        {
            self.cache.pop_front();
        }
    }

    /// 变化或关闭时拒绝旧队列和在途任务的结果。
    fn invalidate(&mut self)
    {
        self.generation = self.generation.wrapping_add(1);
        self.cache.clear();
        self.order.clear();
        for (_, replies) in self.waiting.drain()
        {
            for reply in replies
            {
                let _ = reply.send(None);
            }
        }
    }
}

/// 一个可停止的图像 STA，持有请求唤醒事件与进程内缓存。
pub(crate) struct ImageRuntime
{
    queue: Arc<Mutex<ImageQueue>>,         // 只共享复制后的图像和请求，不共享 COM。
    event: isize,                          // 线程结束后统一释放的原生唤醒事件。
    worker: Mutex<Option<JoinHandle<()>>>, // 正常退出时只回收一次。
}

impl ImageRuntime
{
    /// 初始化独立 COM apartment，确认成功后接受请求。
    pub fn start() -> Result<Self, String>
    {
        let queue = Arc::new(Mutex::new(ImageQueue::default())); // 所有消费者共用有界队列。
        let thread_queue = Arc::clone(&queue); // 工作线程共享状态。
        let event = unsafe { CreateEventW(None, false, false, None) }.map_err(|error| error.to_string())?; // 自动复位事件。
        let event_raw = event.0 as isize; // HANDLE 不直接跨线程传递。
        let (ready, initialized) = mpsc::channel(); // COM 初始化反馈。
        let worker = std::thread::Builder::new().name("desktop-tools-image-sta".into()).spawn(move || {
            if let Err(error) = initialize_shell_apartment()
            {
                let _ = ready.send(Err(error));
                return;
            }
            let _ = ready.send(Ok(()));
            crate::log_info!("系统图像：独立 STA 已启动");
            'work: loop
            {
                let mut message = MSG::default(); // 空闲期间处理 Windows 与 COM 消息。
                while unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) }.as_bool()
                {
                    if message.message == WM_QUIT
                    {
                        break 'work;
                    }
                    unsafe {
                        let _ = TranslateMessage(&message);
                        DispatchMessageW(&message);
                    }
                }
                let next = {
                    let mut state = thread_queue.lock().unwrap(); // 锁外执行可能缓慢的提取。
                    if state.stopped
                    {
                        break;
                    }
                    state.order.pop_front()
                };
                let Some(key) = next
                else
                {
                    if let Err(error) = super::wait_for_shell_activity(HANDLE(event_raw as *mut _))
                    {
                        crate::log_error!("系统图像：{error}");
                        break;
                    }
                    continue;
                };
                let image = match extract_thumbnail(&key.0, key.1)
                {
                    Ok(image) => image.map(Arc::new),
                    Err(error) =>
                    {
                        crate::log_debug!("系统图像：缩略图读取失败，保留图标，项目：{}，错误：{error}", key.0);
                        None
                    }
                }; // 提取失败不改变桌面项目可用性。
                let mut state = thread_queue.lock().unwrap(); // 提交前再次校验代次。
                if state.stopped || state.generation != key.2
                {
                    continue;
                }
                if let Some(replies) = state.waiting.remove(&key)
                {
                    state.remember(key, image.clone());
                    for reply in replies
                    {
                        let _ = reply.send(image.clone());
                    }
                }
            }
            let mut state = thread_queue.lock().unwrap(); // 所有等待者在退出时得到明确结束。
            state.stopped = true;
            state.invalidate();
            drop(state);
            uninitialize_shell_apartment();
            crate::log_info!("系统图像：独立 STA 已退出");
        });
        let worker = match worker
        {
            Ok(worker) => worker,
            Err(error) =>
            {
                let _ = unsafe { CloseHandle(event) };
                return Err(error.to_string());
            }
        }; // 启动失败也释放事件。
        if let Err(error) = initialized.recv().map_err(|error| error.to_string()).and_then(|result| result)
        {
            let _ = worker.join();
            let _ = unsafe { CloseHandle(event) };
            return Err(error);
        }
        Ok(Self { queue, event: event_raw, worker: Mutex::new(Some(worker)) })
    }

    /// 返回共享请求的接收端；调用方可以在 transport 工作线程等待。
    pub fn request(&self, reference: String, pixels: u32) -> Result<mpsc::Receiver<ImageResult>, String>
    {
        let (reply, response) = mpsc::channel(); // 非阻塞结果发布。
        let mut state = self.queue.lock().map_err(|_| "图像队列不可用")?; // 入队与失效串行。
        if state.stopped
        {
            return Err("图像线程已停止".into());
        }
        let key = (reference, pixels, state.generation); // 请求代次在锁内固定。
        if let Some(index) = state.cache.iter().position(|(cached, _)| cached == &key)
        {
            let entry = state.cache.remove(index).unwrap(); // 命中项移动到最近使用位置。
            let _ = reply.send(entry.1.clone());
            state.cache.push_back(entry);
        }
        else if let Some(replies) = state.waiting.get_mut(&key)
        {
            replies.push(reply);
        }
        else
        {
            if state.waiting.len() >= 256
            {
                let _ = reply.send(None);
                return Ok(response);
            }
            state.order.push_back(key.clone());
            state.waiting.insert(key, vec![reply]);
            unsafe { SetEvent(HANDLE(self.event as *mut _)) }.map_err(|error| error.to_string())?;
        }
        Ok(response)
    }

    /// 拖动只读取已完成图像，绝不等待缩略图处理器。
    pub fn cached(&self, reference: &str) -> ImageResult
    {
        let mut state = self.queue.lock().ok()?; // 原生准备阶段仅短暂访问缓存。
        let index = state.cache.iter().rposition(|(key, image)| key.0 == reference && image.is_some())?; // 优先最近显示的尺寸。
        let entry = state.cache.remove(index)?; // 拖动访问同样更新 LRU。
        let image = entry.1.clone(); // Arc 只包含复制的像素。
        state.cache.push_back(entry);
        image
    }

    /// Shell 通知当前仅区分重命名，因此保守失效整份有界图像缓存。
    pub fn invalidate(&self)
    {
        if let Ok(mut state) = self.queue.lock()
        {
            state.invalidate();
        }
    }

    /// 正常退出停止入队，释放等待者并等待所属线程自行清理 COM。
    pub fn shutdown(&self)
    {
        if let Ok(mut state) = self.queue.lock()
        {
            state.stopped = true;
            state.invalidate();
        }
        let _ = unsafe { SetEvent(HANDLE(self.event as *mut _)) };
        if let Some(worker) = self.worker.lock().unwrap().take()
        {
            let _ = worker.join();
        }
    }
}

impl Drop for ImageRuntime
{
    /// 工作线程退出后释放唯一事件句柄。
    fn drop(&mut self)
    {
        self.shutdown();
        let _ = unsafe { CloseHandle(HANDLE(self.event as *mut _)) };
    }
}

#[cfg(test)]
mod tests
{
    use super::*;

    /// 失效必须释放等待者、清除缓存并拒绝旧代次，负缓存也必须有界。
    #[test]
    fn invalidation_and_negative_cache_are_bounded()
    {
        let mut queue = ImageQueue::default(); // 不依赖用户桌面文件的队列测试。
        let (reply, response) = mpsc::channel(); // 模拟正在等待的可见项目。
        let key = ("sample.png".into(), 48, queue.generation); // 失效前的身份。
        queue.order.push_back(key.clone());
        queue.waiting.insert(key.clone(), vec![reply]);
        queue.remember(key.clone(), None);
        queue.invalidate();
        assert!(response.recv().unwrap().is_none());
        assert_ne!(queue.generation, key.2);
        assert!(queue.waiting.is_empty() && queue.order.is_empty() && queue.cache.is_empty());
        for index in 0..300
        {
            queue.remember((index.to_string(), 48, queue.generation), None);
        }
        assert_eq!(queue.cache.len(), 256);
        assert_eq!(queue.cache.front().unwrap().0.0, "44");
    }
}
