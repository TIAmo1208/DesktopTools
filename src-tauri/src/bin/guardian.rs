#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

/// 启动独立恢复进程并输出恢复日志。
fn main()
{
    // 恢复进程写独立日志目录，避免与主进程在同一天文件里重复记录。
    let mut config = desktop_tools_lib::tools::log::LogConfig::default();
    if let Ok(directory) = desktop_tools_lib::tools::log::default_directory()
    {
        config.directory = directory.join("guardian");
    }
    config.console_level = desktop_tools_lib::tools::log::LogLevel::None; // 恢复进程不占用主进程控制台，日志只写自己的文件。
    if let Err(error) = desktop_tools_lib::tools::log::init(config)
        && cfg!(debug_assertions)
    {
        eprintln!("日志系统初始化失败：{error}");
    }
    if let Err(error) = desktop_tools_lib::recovery::run_guardian()
    {
        desktop_tools_lib::log_fatal!("恢复：guardian 运行失败，错误：{error}");
        let _ = desktop_tools_lib::tools::log::shutdown();
        std::process::exit(2);
    }
    let _ = desktop_tools_lib::tools::log::shutdown();
}
