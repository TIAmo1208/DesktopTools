---
status: accepted
---

# 隔离 Explorer 私有兼容层

## 已落实的宿主与 DPI 路径

正式实现使用独立 Win32/WebView2 Windowed 宿主。`WorkerW`、`0x052C` 和 Explorer 窗口拓扑属于私有兼容机制，通用 Win32 API 不保证它们的宿主语义。

跨进程 `SetParent` 后窗口实际 DPI 可能与目标显示器 DPI 不同。正式 `platform/windows/desktop_host.rs` 使用 Controller3 设置 `BoundsMode=USE_RAW_PIXELS`、关闭自动显示器缩放检测，并按目标显示器设置 `RasterizationScale`；页面 DPR、命中区域换算与目标比例保持一致，实际 HWND DPI 作为独立诊断信息。显示环境变化由宿主协调和重建处理，不把窗口级 `WM_DPICHANGED` 建议路径写成已实现。

Windowed 宿主通过显式栅格化控制处理跨进程 DPI 影响。正式接管门槛见 [ADR-0016](0016-正式项目架构.md)。

### 实现边界与官方契约

每个宿主按目标显示器应用页面 DPR 与命中坐标；实际显示效果尚未经真实环境验证。

- [SetParent](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setparent)：跨进程 DPI 重置与窗口样式责任。
- [ICoreWebView2Controller3](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2controller3)：显式栅格化、自动缩放检测与 BoundsMode。
- [ICoreWebView2Controller](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2controller)：父窗口、Bounds 和祖先位置通知。
- [GetDpiForWindow](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getdpiforwindow)：实际窗口 DPI 的含义。
- [IsWindowVisible](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-iswindowvisible)：可见样式不保证未被遮挡或裁剪。
- [WM_USER](https://learn.microsoft.com/en-us/windows/win32/winmsg/wm-user)：私有窗口消息范围，不承诺 Explorer `0x052C` 语义。

Explorer 私有桌面层能力集中在可替换的兼容适配器中。Tauri 负责控制面、设置和应用生命周期；原生桌面宿主负责 HWND、WebView2 Controller、WndProc、每台宿主的 OLE target 和恢复状态。实现路径依次为公开宿主基线、Tauri 创建期宿主实验、独立原生宿主和组合宿主；当前 Windows 10 22H2 平台覆盖双屏、透明壁纸、任务栏工作区、输入、Win+D、正常退出和崩溃恢复。运行期间适配器失效时，guardian 恢复 Explorer 状态，主程序停止桌面接管、保留布局数据并记录错误；其他 Windows 版本不再阻塞当前平台实现。
