use super::worker::{LogCommand, LogRecord, LogWorker, normalized_interval};
use super::{LogConfig, LogError, LogLevel};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// 持有进程级日志配置、命令通道和后台线程。
struct LogRuntime
{
    /// 生产者读取和更新的当前配置。
    config: LogConfig,
    /// 向后台线程发送有序命令的通道。
    sender: Sender<LogCommand>,
    /// 后台线程所有权。
    worker: JoinHandle<()>,
}

/// 保存可重复初始化的进程级日志运行时。
// ponytail: 进程级互斥锁串行化配置边界；日志吞吐成为瓶颈时再拆分生产者状态。
static LOG_RUNTIME: OnceLock<Mutex<Option<LogRuntime>>> = OnceLock::new();

/// 获取进程级日志运行时容器。
fn runtime_container() -> &'static Mutex<Option<LogRuntime>> { LOG_RUNTIME.get_or_init(|| Mutex::new(None)) }

/// 获取进程级日志运行时锁，并在锁中毒后保留可恢复状态。
fn lock_runtime() -> MutexGuard<'static, Option<LogRuntime>> { runtime_container().lock().unwrap_or_else(std::sync::PoisonError::into_inner) }

/// 初始化进程级日志系统；重复调用沿用当前实例。
pub fn init(mut config: LogConfig) -> Result<(), LogError>
{
    // 进程级日志运行时。
    let mut runtime = lock_runtime();
    if runtime.is_some()
    {
        return Ok(());
    }

    // 后台线程处理的工作状态。
    let mut worker = LogWorker::new(&config);
    if config.file_output
        && let Err(error) = worker.initialize_file_output()
    {
        if cfg!(debug_assertions)
        {
            eprintln!("日志文件输出已关闭，目录无法使用：\"{}\"：{error}", config.directory.display());
        }
        config.file_output = false;
    }

    // 后台日志命令通道。
    let (sender, receiver) = mpsc::channel();
    // 后台日志线程。
    let worker = thread::Builder::new().name("desktop-tools-log".to_string()).spawn(move || worker.run(receiver)).map_err(|error| LogError::WorkerStart(error.to_string()))?;

    *runtime = Some(LogRuntime { config, sender, worker });
    // 初始化完成后的当前配置。
    let config = runtime.as_ref().expect("日志运行时刚完成初始化").config.clone();
    drop(runtime);

    debug(format!("日志：初始化完成，文件等级：{}，控制台等级：{}，文件目录：\"{}\"，文件输出：{}", config.file_level.name(), config.console_level.name(), config.directory.display(), config.file_output), file!(), line!())?;

    Ok(())
}

/// 关闭日志系统，排空队列并刷新文件缓冲区。
pub fn shutdown() -> Result<(), LogError>
{
    // 从全局容器中取出的日志运行时。
    let runtime = lock_runtime().take().ok_or(LogError::NotInitialized)?;
    // 关闭时用于提示的文件输出状态。
    let file_output = runtime.config.file_output;
    // 关闭时用于提示的日志目录。
    let directory = runtime.config.directory.clone();
    // 等待后台线程排空队列的同步通道。
    let (reply, response) = mpsc::sync_channel(0);

    runtime.sender.send(LogCommand::Shutdown(reply)).map_err(|_| LogError::WorkerStopped)?;
    response.recv().map_err(|_| LogError::WorkerStopped)?;
    runtime.worker.join().map_err(|_| LogError::WorkerPanicked)?;

    if cfg!(debug_assertions) && file_output
    {
        // 展示给用户的绝对日志目录。
        let absolute_directory = absolute_path(&directory);
        println!("日志文件目录：\"{}\"", absolute_directory.display());
    }

    Ok(())
}

/// 同时设置文件和控制台日志等级。
pub fn set_log_level(level: LogLevel) -> Result<(), LogError>
{
    // 进程级日志运行时。
    let mut runtime = lock_runtime();
    // 已初始化的日志运行时。
    let runtime = runtime.as_mut().ok_or(LogError::NotInitialized)?;

    runtime.config.file_level = level;
    runtime.config.console_level = level;
    Ok(())
}

/// 设置文件日志等级。
pub fn set_file_level(level: LogLevel) -> Result<(), LogError>
{
    // 进程级日志运行时。
    let mut runtime = lock_runtime();
    // 已初始化的日志运行时。
    let runtime = runtime.as_mut().ok_or(LogError::NotInitialized)?;

    runtime.config.file_level = level;
    Ok(())
}

/// 设置控制台日志等级。
pub fn set_console_level(level: LogLevel) -> Result<(), LogError>
{
    // 进程级日志运行时。
    let mut runtime = lock_runtime();
    // 已初始化的日志运行时。
    let runtime = runtime.as_mut().ok_or(LogError::NotInitialized)?;

    runtime.config.console_level = level;
    Ok(())
}

/// 同步启用或关闭日志文件输出。
pub fn set_file_output(enabled: bool) -> Result<(), LogError>
{
    // 进程级日志运行时。
    let mut runtime = lock_runtime();
    // 已初始化的日志运行时。
    let runtime = runtime.as_mut().ok_or(LogError::NotInitialized)?;

    // 等待文件状态切换完成的同步通道。
    let (reply, response) = mpsc::sync_channel(0);
    runtime.sender.send(LogCommand::SetFileOutput(enabled, reply)).map_err(|_| LogError::WorkerStopped)?;
    response.recv().map_err(|_| LogError::WorkerStopped)?.map_err(LogError::FileOutput)?;

    runtime.config.file_output = enabled;
    Ok(())
}

/// 同步切换日志文件目录。
pub fn set_file_path(directory: impl Into<PathBuf>) -> Result<(), LogError>
{
    // 请求切换到的日志目录。
    let directory = directory.into();
    // 进程级日志运行时。
    let mut runtime = lock_runtime();
    // 已初始化的日志运行时。
    let runtime = runtime.as_mut().ok_or(LogError::NotInitialized)?;
    if !runtime.config.file_output
    {
        return Err(LogError::FileOutput("文件输出当前处于关闭状态".to_string()));
    }

    // 等待目录切换完成的同步通道。
    let (reply, response) = mpsc::sync_channel(0);
    runtime.sender.send(LogCommand::SetFilePath(directory.clone(), reply)).map_err(|_| LogError::WorkerStopped)?;
    response.recv().map_err(|_| LogError::WorkerStopped)?.map_err(LogError::FileOutput)?;

    runtime.config.directory = directory;
    Ok(())
}

/// 设置后台文件刷新间隔，最小值为一秒。
pub fn set_flush_interval(interval: Duration) -> Result<(), LogError>
{
    // 归一化后的刷新间隔。
    let interval = normalized_interval(interval);
    // 进程级日志运行时。
    let mut runtime = lock_runtime();
    // 已初始化的日志运行时。
    let runtime = runtime.as_mut().ok_or(LogError::NotInitialized)?;

    runtime.sender.send(LogCommand::SetFlushInterval(interval)).map_err(|_| LogError::WorkerStopped)?;
    runtime.config.flush_interval = interval;
    Ok(())
}

/// 写入带源代码位置的 Debug 日志。
pub fn debug(message: impl Into<String>, file: &'static str, line: u32) -> Result<(), LogError> { enqueue(LogLevel::Debug, "DEBUG", message.into(), Some(file), Some(line)) }

/// 写入 Info 日志。
pub fn info(message: impl Into<String>) -> Result<(), LogError> { enqueue(LogLevel::Info, "INFO", message.into(), None, None) }

/// 写入 Warn 日志。
pub fn warn(message: impl Into<String>) -> Result<(), LogError> { enqueue(LogLevel::Warn, "WARN", message.into(), None, None) }

/// 写入 Error 日志。
pub fn error(message: impl Into<String>) -> Result<(), LogError> { enqueue(LogLevel::Error, "ERROR", message.into(), None, None) }

/// 写入使用 Error 阈值的 Fatal 日志。
pub fn fatal(message: impl Into<String>) -> Result<(), LogError> { enqueue(LogLevel::Error, "FATAL", message.into(), None, None) }

/// 按当前等级和文件开关计算目标并发送日志记录。
fn enqueue(level: LogLevel, level_name: &'static str, message: String, source_file: Option<&'static str>, source_line: Option<u32>) -> Result<(), LogError>
{
    // 进程级日志运行时。
    let runtime = lock_runtime();
    // 已初始化的日志运行时。
    let runtime = runtime.as_ref().ok_or(LogError::NotInitialized)?;
    // 本条记录的文件输出决策。
    let file_output = runtime.config.file_output && runtime.config.file_level.allows(level);
    // 本条记录的控制台输出决策。
    let console_output = cfg!(debug_assertions) && runtime.config.console_level.allows(level);
    if !file_output && !console_output
    {
        return Ok(());
    }

    runtime.sender.send(LogCommand::Record(LogRecord { level_name, message, source_file, source_line, file_output, console_output })).map_err(|_| LogError::WorkerStopped)
}

/// 将日志目录解析为关闭提示使用的绝对路径。
fn absolute_path(path: &Path) -> PathBuf
{
    if path.is_absolute()
    {
        return path.to_path_buf();
    }

    std::env::current_dir().map(|directory| directory.join(path)).unwrap_or_else(|_| path.to_path_buf())
}
