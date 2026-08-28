# 后端边界

`src-tauri/src/` 负责应用生命周期、桌面宿主编排、工作区规则、Windows Shell 访问、持久化、组件运行时和恢复机制。

`commands/` 是暴露给前端的 Tauri 边界；功能目录负责领域行为；`platform/windows/` 负责 Windows API 适配；`contracts/` 保存序列化命令、事件和快照类型；`tools/log/` 提供进程级异步日志；`bin/guardian.rs` 作为独立恢复进程运行。
