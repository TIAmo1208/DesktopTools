mod runtime;
mod worker;

#[cfg(test)]
mod tests;

use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

pub use runtime::{
    debug, error, fatal, info, init, set_console_level, set_file_level, set_file_output,
    set_file_path, set_flush_interval, set_log_level, shutdown, warn,
};

/// 控制日志记录的详细程度。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum LogLevel {
    /// 关闭对应输出目标。
    None = 0,
    /// 输出错误和致命错误。
    Error = 1,
    /// 输出警告及更严重记录。
    Warn = 2,
    /// 输出信息及更严重记录。
    Info = 3,
    /// 输出全部记录。
    Debug = 4,
}

impl LogLevel {
    /// 判断当前阈值是否接受指定记录等级。
    fn allows(self, record_level: Self) -> bool {
        self != Self::None && self as u8 >= record_level as u8
    }

    /// 返回日志状态消息使用的稳定等级名称。
    fn name(self) -> &'static str {
        match self {
            Self::None => "NONE",
            Self::Error => "ERROR",
            Self::Warn => "WARN",
            Self::Info => "INFO",
            Self::Debug => "DEBUG",
        }
    }
}

/// 定义日志系统的初始输出行为。
#[derive(Clone, Debug)]
pub struct LogConfig {
    /// 文件日志等级。
    pub file_level: LogLevel,
    /// 控制台日志等级。
    pub console_level: LogLevel,
    /// 文件输出开关。
    pub file_output: bool,
    /// 每日日志文件所在目录。
    pub directory: PathBuf,
    /// 后台线程刷新文件缓冲区的间隔。
    pub flush_interval: Duration,
}

impl Default for LogConfig {
    /// 创建与原日志工具一致的默认配置。
    fn default() -> Self {
        Self {
            file_level: LogLevel::Info,
            console_level: LogLevel::Info,
            file_output: true,
            directory: PathBuf::from("Log"),
            flush_interval: Duration::from_secs(10),
        }
    }
}

/// 描述日志初始化、配置和关闭阶段的错误。
#[derive(Debug)]
pub enum LogError {
    /// 日志系统尚未初始化。
    NotInitialized,
    /// 后台日志线程已经停止。
    WorkerStopped,
    /// 后台日志线程启动失败。
    WorkerStart(String),
    /// 文件输出配置失败。
    FileOutput(String),
    /// 后台日志线程异常退出。
    WorkerPanicked,
}

impl fmt::Display for LogError {
    /// 将日志错误格式化为用户可见的中文说明。
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotInitialized => formatter.write_str("日志系统尚未初始化"),
            Self::WorkerStopped => formatter.write_str("日志后台线程已经停止"),
            Self::WorkerStart(reason) => write!(formatter, "日志后台线程启动失败：{reason}"),
            Self::FileOutput(reason) => write!(formatter, "日志文件输出失败：{reason}"),
            Self::WorkerPanicked => formatter.write_str("日志后台线程异常退出"),
        }
    }
}

impl std::error::Error for LogError {}

/// 使用 Rust 格式参数写入 Debug 日志。
#[macro_export]
macro_rules! log_debug {
    ($($argument:tt)*) => {{
        let _ = $crate::tools::log::debug(format!($($argument)*), file!(), line!());
    }};
}

/// 使用 Rust 格式参数写入 Info 日志。
#[macro_export]
macro_rules! log_info {
    ($($argument:tt)*) => {{
        let _ = $crate::tools::log::info(format!($($argument)*));
    }};
}

/// 使用 Rust 格式参数写入 Warn 日志。
#[macro_export]
macro_rules! log_warn {
    ($($argument:tt)*) => {{
        let _ = $crate::tools::log::warn(format!($($argument)*));
    }};
}

/// 使用 Rust 格式参数写入 Error 日志。
#[macro_export]
macro_rules! log_error {
    ($($argument:tt)*) => {{
        let _ = $crate::tools::log::error(format!($($argument)*));
    }};
}

/// 使用 Rust 格式参数写入 Fatal 日志。
#[macro_export]
macro_rules! log_fatal {
    ($($argument:tt)*) => {{
        let _ = $crate::tools::log::fatal(format!($($argument)*));
    }};
}
