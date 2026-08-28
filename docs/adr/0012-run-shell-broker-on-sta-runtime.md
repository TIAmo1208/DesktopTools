---
status: accepted
---

# 在专用 STA 运行时执行 Shell 操作

Desktop Tools 在进程内建立一个 Shell STA 运行时，负责桌面命名空间、文件操作、项目菜单和变化通知；每个桌面宿主线程负责对应 HWND 的 OLE drop target。Rust 与 React 通过异步命令、不可变快照和操作结果通信，Shell extension 调用与 React 渲染线程隔离。专用原生 owner HWND 负责项目菜单、文件操作确认和 OLE 生命周期。
