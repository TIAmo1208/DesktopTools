---
status: accepted
---

# 隔离 Explorer 私有兼容层

Explorer 私有桌面层能力集中在可替换的兼容适配器中。Tauri 负责控制面、设置和应用生命周期；原生桌面宿主负责 HWND、WebView2 Controller、WndProc、每台宿主的 OLE target 和恢复状态。验证层级依次为公开宿主基线、Tauri 创建期宿主实验、独立原生宿主和组合宿主；当前 Windows 10 22H2 平台验证双屏真实像素、透明壁纸、任务栏工作区、输入、Win+D、正常退出和崩溃恢复。运行期间适配器失效时，guardian 恢复 Explorer 状态，主程序停止桌面接管、保留布局数据并记录错误；其他 Windows 版本不再阻塞当前平台实现。
