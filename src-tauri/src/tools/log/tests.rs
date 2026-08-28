use super::{
    LogConfig, LogLevel, init, set_console_level, set_file_level, set_file_output, set_file_path,
    set_flush_interval, set_log_level, shutdown,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// 串行保护使用进程级日志实例的测试。
static TEST_LOCK: Mutex<()> = Mutex::new(());

/// 创建当前测试进程独占的临时目录。
fn unique_test_path(name: &str) -> PathBuf {
    // 避免测试目录冲突的时间戳。
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("系统时间应晚于 Unix epoch")
        .as_nanos();

    std::env::temp_dir().join(format!(
        "desktop-tools-log-{}-{timestamp}-{name}",
        std::process::id()
    ))
}

/// 读取指定目录中唯一的每日日志文件。
fn read_log(directory: &Path) -> String {
    // 目录中的日志文件列表。
    let mut files = fs::read_dir(directory)
        .expect("日志目录应存在")
        .map(|entry| entry.expect("日志目录项应可读取").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "log"))
        .collect::<Vec<_>>();
    files.sort();

    assert_eq!(files.len(), 1, "测试期间应只生成一个每日日志文件");
    fs::read_to_string(&files[0]).expect("日志文件应可读取")
}

/// 验证原工具的文件、等级、切换、并发和生命周期契约。
#[test]
fn restores_tool_log_behavior() {
    // 进程级日志测试串行锁。
    let _guard = TEST_LOCK.lock().expect("日志测试锁应可获取");
    // 当前测试的根目录。
    let root = unique_test_path("behavior");
    // 路径切换前的日志目录。
    let first_directory = root.join("first path").join("nested");
    // 路径切换后的日志目录。
    let second_directory = root.join("second path");

    init(LogConfig {
        file_level: LogLevel::Debug,
        console_level: LogLevel::None,
        file_output: true,
        directory: first_directory.clone(),
        flush_interval: Duration::from_secs(10),
    })
    .expect("日志系统应完成初始化");

    crate::log_info!("切换前记录 {}", 1);
    crate::log_debug!("Debug 记录");
    crate::log_warn!("Warn 记录");
    crate::log_error!("Error 记录");
    crate::log_fatal!("Fatal 记录");

    set_file_path(second_directory.clone()).expect("日志目录应完成同步切换");
    crate::log_info!("切换后记录");

    set_file_output(false).expect("文件输出应关闭");
    crate::log_info!("关闭期间记录");
    set_file_output(true).expect("文件输出应重新启用");
    crate::log_info!("重新启用记录");

    set_file_level(LogLevel::Warn).expect("文件等级应更新");
    crate::log_info!("等级过滤记录");
    crate::log_warn!("等级接受记录");
    set_log_level(LogLevel::Debug).expect("全部输出等级应更新");
    set_console_level(LogLevel::None).expect("控制台等级应恢复关闭状态");
    set_flush_interval(Duration::ZERO).expect("刷新间隔应归一化");

    // 验证长消息保持完整的正文。
    let long_message = format!("长消息开始-{}-长消息结束", "x".repeat(12_000));
    crate::log_info!("{}", long_message);

    // 并发日志生产线程。
    let workers = (0..4)
        .map(|thread_index| {
            thread::spawn(move || {
                for sequence in 0..250 {
                    crate::log_info!("并发记录 thread={thread_index} sequence={sequence}");
                }
            })
        })
        .collect::<Vec<_>>();

    for worker in workers {
        worker.join().expect("并发日志线程应完成");
    }

    shutdown().expect("日志系统应排空队列并关闭");

    // 路径切换前的日志正文。
    let first_text = read_log(&first_directory);
    // 路径切换后的日志正文。
    let second_text = read_log(&second_directory);

    assert!(first_text.contains("切换前记录 1"));
    assert!(first_text.contains("Debug 记录"));
    assert!(first_text.contains("Fatal 记录"));
    assert!(!first_text.contains("切换后记录"));
    assert!(second_text.contains("切换后记录"));
    assert!(!second_text.contains("关闭期间记录"));
    assert!(second_text.contains("重新启用记录"));
    assert!(!second_text.contains("等级过滤记录"));
    assert!(second_text.contains("等级接受记录"));
    assert!(second_text.contains(&long_message));
    assert_eq!(second_text.matches("并发记录 thread=").count(), 1_000);

    // 文件输出关闭时使用的缺席目录。
    let no_file_directory = root.join("no-file");
    init(LogConfig {
        file_level: LogLevel::Debug,
        console_level: LogLevel::None,
        file_output: false,
        directory: no_file_directory.clone(),
        flush_interval: Duration::from_secs(10),
    })
    .expect("关闭文件输出时仍应完成初始化");
    crate::log_info!("无文件记录");
    shutdown().expect("第二个日志生命周期应关闭");
    assert!(!no_file_directory.exists());

    // 阻止目录创建的普通文件路径。
    let blocker = root.join("blocker");
    fs::write(&blocker, "blocker").expect("阻塞文件应创建");
    init(LogConfig {
        file_level: LogLevel::Info,
        console_level: LogLevel::None,
        file_output: true,
        directory: blocker,
        flush_interval: Duration::from_secs(10),
    })
    .expect("文件失败时控制台日志运行时仍应初始化");
    assert!(set_file_output(true).is_err());
    shutdown().expect("文件失败后的日志生命周期应关闭");

    fs::remove_dir_all(&root).expect("测试日志目录应清理");
}
