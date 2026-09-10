/// 启动独立恢复进程并输出恢复日志。
fn main()
{
    if let Err(error) = desktop_tools_lib::tools::log::init(Default::default())
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
