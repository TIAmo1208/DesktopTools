use super::LogConfig;
use chrono::{DateTime, Local};
use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender};
use std::time::Duration;

/// 保存一条等待后台线程处理的日志记录。
pub(super) struct LogRecord
{
    /// 文本日志等级。
    pub(super) level_name: &'static str,
    /// 已完成 Rust 格式化的消息正文。
    pub(super) message: String,
    /// Debug 记录的源文件。
    pub(super) source_file: Option<&'static str>,
    /// Debug 记录的源代码行号。
    pub(super) source_line: Option<u32>,
    /// 当前记录的文件输出决策。
    pub(super) file_output: bool,
    /// 当前记录的控制台输出决策。
    pub(super) console_output: bool,
}

/// 表示发送给后台日志线程的有序命令。
pub(super) enum LogCommand
{
    /// 处理一条日志记录。
    Record(LogRecord),
    /// 同步切换文件输出状态。
    SetFileOutput(bool, SyncSender<Result<(), String>>),
    /// 同步切换日志目录。
    SetFilePath(PathBuf, SyncSender<Result<(), String>>),
    /// 更新文件刷新间隔。
    SetFlushInterval(Duration),
    /// 排空队列并关闭后台线程。
    Shutdown(SyncSender<()>),
}

/// 在后台执行控制台输出、文件轮转和缓冲刷新。
pub(super) struct LogWorker
{
    /// 当前文件输出目录。
    directory: PathBuf,
    /// 当前文件输出状态。
    file_output: bool,
    /// 当前刷新间隔。
    flush_interval: Duration,
    /// 当前日期对应的日志文件名。
    active_date: Option<String>,
    /// 当前日志文件的缓冲写入器。
    writer: Option<BufWriter<File>>,
}

impl LogWorker
{
    /// 根据初始配置创建后台工作状态。
    pub(super) fn new(config: &LogConfig) -> Self { Self { directory: config.directory.clone(), file_output: config.file_output, flush_interval: normalized_interval(config.flush_interval), active_date: None, writer: None } }

    /// 初始化文件输出；失败时保留控制台运行能力。
    pub(super) fn initialize_file_output(&mut self) -> io::Result<()>
    {
        if let Err(error) = self.ensure_writer(&Local::now())
        {
            self.file_output = false;
            return Err(error);
        }

        Ok(())
    }

    /// 运行后台命令循环，直至收到关闭命令或全部发送端释放。
    pub(super) fn run(mut self, receiver: Receiver<LogCommand>)
    {
        loop
        {
            match receiver.recv_timeout(self.flush_interval)
            {
                Ok(LogCommand::Record(record)) => self.process_record(record),
                Ok(LogCommand::SetFileOutput(enabled, reply)) =>
                {
                    // 文件输出切换结果。
                    let result = self.set_file_output(enabled);
                    let _ = reply.send(result);
                }
                Ok(LogCommand::SetFilePath(directory, reply)) =>
                {
                    // 日志目录切换结果。
                    let result = self.set_file_path(directory);
                    let _ = reply.send(result);
                }
                Ok(LogCommand::SetFlushInterval(interval)) =>
                {
                    self.flush_interval = normalized_interval(interval);
                }
                Ok(LogCommand::Shutdown(reply)) =>
                {
                    self.flush_and_close();
                    let _ = reply.send(());
                    break;
                }
                Err(RecvTimeoutError::Timeout) => self.flush(),
                Err(RecvTimeoutError::Disconnected) =>
                {
                    self.flush_and_close();
                    break;
                }
            }
        }
    }

    /// 格式化一条记录并按入队时的目标决策分发。
    fn process_record(&mut self, record: LogRecord)
    {
        // 本条记录共用的本地时间。
        let now = Local::now();
        // 与原工具一致的日志正文。
        let text = format!("[{}]:[{}]\t{}", now.format("%H:%M:%S"), record.level_name, record.message);

        if record.console_output
        {
            // 当前等级对应的 ANSI 颜色。
            let color = console_color(record.level_name);
            if let (Some(file), Some(line)) = (record.source_file, record.source_line)
            {
                println!("{color}{text}\t {file}:{line}\x1b[0m");
            }
            else
            {
                println!("{color}{text}\x1b[0m");
            }
        }

        if record.file_output
            && self.file_output
            && let Err(error) = self.write_record(&now, &text)
        {
            eprintln!("日志文件输出已关闭：{error}");
            self.file_output = false;
            self.flush_and_close();
        }
    }

    /// 将完整记录追加到当前日期的日志文件。
    fn write_record(&mut self, now: &DateTime<Local>, text: &str) -> io::Result<()>
    {
        self.ensure_writer(now)?;

        if let Some(writer) = self.writer.as_mut()
        {
            writer.write_all(text.as_bytes())?;
            writer.write_all(b"\n")?;
        }

        Ok(())
    }

    /// 确保写入器指向当前本地日期的文件。
    fn ensure_writer(&mut self, now: &DateTime<Local>) -> io::Result<()>
    {
        // 当前本地日期文本。
        let date = now.format("%Y-%m-%d").to_string();
        if self.writer.is_some() && self.active_date.as_deref() == Some(date.as_str())
        {
            return Ok(());
        }

        self.flush_and_close();
        std::fs::create_dir_all(&self.directory)?;

        // 当前日期的日志文件路径。
        let output_path = self.directory.join(format!("{date}.log"));
        // 文件创建前的存在状态。
        let create_new_file = !output_path.exists();
        // 以追加方式打开的日志文件。
        let file = OpenOptions::new().create(true).append(true).open(&output_path)?;

        self.writer = Some(BufWriter::new(file));
        self.active_date = Some(date);

        if create_new_file
        {
            println!("[{}]:[INFO]\t日志：已创建文件：\"{}\"", now.format("%H:%M:%S"), output_path.display());
        }

        Ok(())
    }

    /// 同步启用或关闭文件输出。
    fn set_file_output(&mut self, enabled: bool) -> Result<(), String>
    {
        if !enabled
        {
            self.file_output = false;
            self.flush_and_close();
            return Ok(());
        }

        self.file_output = true;
        if let Err(error) = self.ensure_writer(&Local::now())
        {
            self.file_output = false;
            self.flush_and_close();
            return Err(format!("目录 \"{}\"：{error}", self.directory.display()));
        }

        Ok(())
    }

    /// 同步排空旧目录并切换到新目录。
    fn set_file_path(&mut self, directory: PathBuf) -> Result<(), String>
    {
        if !self.file_output
        {
            return Err("文件输出当前处于关闭状态".to_string());
        }

        // 失败时用于恢复的原目录。
        let previous_directory = self.directory.clone();
        self.flush_and_close();
        self.directory = directory;

        if let Err(error) = self.ensure_writer(&Local::now())
        {
            // 新目录打开失败时的诊断文本。
            let reason = format!("目录 \"{}\"：{error}", self.directory.display());
            self.directory = previous_directory;
            let _ = self.ensure_writer(&Local::now());
            return Err(reason);
        }

        Ok(())
    }

    /// 刷新当前文件缓冲区并报告写入失败。
    fn flush(&mut self)
    {
        if let Some(writer) = self.writer.as_mut()
            && let Err(error) = writer.flush()
        {
            eprintln!("日志文件刷新失败：{error}");
        }
    }

    /// 刷新并关闭当前日志文件。
    fn flush_and_close(&mut self)
    {
        self.flush();
        self.writer = None;
        self.active_date = None;
    }
}

/// 将刷新间隔限制为原工具支持的最小一秒。
pub(super) fn normalized_interval(interval: Duration) -> Duration { interval.max(Duration::from_secs(1)) }

/// 返回指定日志等级的 ANSI 控制台颜色。
fn console_color(level_name: &str) -> &'static str
{
    match level_name
    {
        "DEBUG" => "\x1b[37m",
        "INFO" => "\x1b[32m",
        "WARN" => "\x1b[33m",
        "ERROR" | "FATAL" => "\x1b[31m",
        _ => "",
    }
}
