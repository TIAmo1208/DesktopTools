# 原生窗口适配器混合 DPI 接口核对

日期：2026-08-27

范围：`prototypes/tauri-desktop-host` 的 `native-windowed-adapter`，Windows 10 双屏混合 DPI 运行路径。

## 直接结论

2026-08-27 的基线日志显示，跨进程 `SetParent` 后实际宿主 HWND 统一采用父级 DPI；本轮已增加 Controller3 手动栅格化配置，按目标显示器比例为每个 WebView2 页面设置独立渲染比例。

125% 复测中，显示器枚举结果为主屏 `120 DPI`、副屏 `96 DPI`；`SetParent` 完成后，两台外层宿主、两台 Controller 子窗口和两个 WebView 前端均为 `120 DPI / DPR 1.25`。副屏显示异常来自窗口树的实际 DPI 与保存的副屏 `scale_factor=1.0` 分离。

该结果符合 Microsoft 对跨进程 `SetParent` 的说明：父子 HWND 使用不同 DPI awareness 模式时，跨进程 `SetParent` 会强制重置子窗口进程的 DPI awareness。Microsoft 同时规定同一 HWND 树中的子窗口需匹配父窗口缩放。当前应用 HWND 挂到 Explorer `WorkerW` 后，Tao 启动阶段申请的 Per-Monitor V2 仍受跨进程父级重置影响；逐屏渲染比例由 Controller3 显式管理，实际 HWND DPI继续单独记录。

## 运行证据

证据日志：`.scratch/desktop-host-prototype/logs/20260827-165308-run-native-windowed-adapter.log`

| 阶段 | 主屏 `monitor-0` | 副屏 `monitor-1` | 结论 |
| --- | --- | --- | --- |
| Tauri 显示器枚举 | 第 73 行：`scale_factor=1.25`、`DPI=120` | 第 74 行：`scale_factor=1`、`DPI=96` | 显示器枚举正确区分两台显示器 |
| `SetParent` 后外层 HWND / Controller HWND | 第 78 行：`120 / 120` | 第 82 行：`120 / 120` | 副屏窗口树采用主屏 DPI |
| WebView 页面初始化 | 第 88 行：`2048x1112`、`DPR=1.25` | 第 90 行：`2048x1120`、`DPR=1.25` | 副屏 WebView2 采用主屏栅格化比例 |
| 输入区域缩放 | 第 100 行：`scale_factor=1.25` | 第 95、101 行：`scale_factor=1` | Rust 命中区域与副屏 WebView CSS 坐标使用不同换算比例 |

150% 轮次继续呈现相同结构：显示器枚举为 `144 / 96 DPI`，两台宿主与 Controller 均为 `144 DPI`，两个前端均为 `DPR=1.5`。这证明问题覆盖任意主屏缩放值，和指定的 125% 数值无关。

### 裁剪一致性与真正逐屏 DPI 的边界

把副屏 `HostHitState.scale_factor` 从枚举值 `1.0` 改成当前前端实际 `DPR=1.25`，可以让 `SetWindowRgn`、命中测试和 WebView CSS 坐标重新一致，副屏裁剪显示可随之恢复。该改动属于裁剪一致性修复，副屏 WebView 仍按主屏 125% 比例栅格化。

真正逐屏 DPI 要求副屏 Controller 采用副屏的目标比例 `96 / 96 = 1.0`，并使前端 DPR、Rust 命中换算和窗口区域同时采用 `1.0`。主屏切到 150% 时同理：主屏采用 `1.5`，副屏继续采用 `1.0`。

## 官方 API 契约与当前实现

### 1. Per-Monitor V2

Microsoft 推荐使用应用清单声明 `<dpiAwareness>PerMonitorV2</dpiAwareness>`。程序化设置应在任何 HWND 创建前调用 `SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)`。PMv2 让顶层和子 HWND 接收 DPI 变化通知，并让应用看到各显示器原始像素。

当前锁定的 Tao 0.35.3 在事件循环初始化时先调用 `SetProcessDpiAwarenessContext(PER_MONITOR_AWARE_V2)`，失败后回退到 PMv1。仓库自身没有 DPI awareness 清单。这个启动设置覆盖应用自有窗口的初始创建；后续跨进程 `SetParent` 引入强制重置条件。

来源：

- [Setting the default DPI awareness for a process](https://learn.microsoft.com/en-us/windows/win32/hidpi/setting-the-default-dpi-awareness-for-a-process)
- [DPI_AWARENESS_CONTEXT](https://learn.microsoft.com/en-us/windows/win32/hidpi/dpi-awareness-context)
- [High DPI Desktop Application Development on Windows](https://learn.microsoft.com/en-us/windows/win32/hidpi/high-dpi-desktop-application-development-on-windows)

### 2. `SetParent`

Microsoft 的 DPI compatibility 表规定，Windows 8.1、Windows 10 1607 及更早版本、Windows 10 1703 及更高版本的跨进程 `SetParent` 均会强制重置子窗口进程的 DPI awareness。High DPI 指南进一步说明，同一 HWND 树无法承载不同 DPI awareness 模式的父子窗口。

当前 `attach_native_host` 先把自有窗口样式改为 `WS_CHILD`，再调用 `SetParent(hwnd, WorkerW)`，样式顺序符合 `SetParent` 文档。代码丢弃 `SetParent` 返回值，随后只用 `GetParent` 验证父级。日志表明本次调用成功，同时副屏窗口 DPI 被改成主屏 DPI。

来源：[SetParent](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setparent)

### 3. `GetDpiForWindow`

`GetDpiForWindow` 返回值由目标 HWND 的实际 DPI awareness 决定：Unaware 返回 96，System-aware 返回系统 DPI，Per-monitor-aware 返回窗口所在显示器 DPI。该 API 适合验证 `SetParent` 后窗口真正采用的 DPI。

当前实现只把 `GetDpiForWindow` 用于 `native-host.created` 日志。命中图和 `HostRecord` 继续使用枚举阶段的 `monitor.scale_factor`。因此日志已经检测出副屏 `GetDpiForWindow=120` 与目标 `96` 的差异，布局路径仍继续运行。

来源：[GetDpiForWindow](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getdpiforwindow)

### 4. `WM_DPICHANGED`

窗口移到不同 DPI 显示器或当前显示器 DPI 改变时，Windows 发送 `WM_DPICHANGED`。`wParam` 携带新 DPI，`lParam` 携带建议窗口矩形；Microsoft 要求应用使用该矩形调用 `SetWindowPos`。PMv2 会把通知发送给顶层和子 HWND。

当前自定义 `host_window_proc` 处理 `WM_NCHITTEST` 和 `WM_MOUSEACTIVATE`，显示器状态变化由轮询触发整组宿主重建。宿主重建能够更新物理工作区，重建后的跨进程父子关系仍会再次进入统一 DPI 状态。

来源：[WM_DPICHANGED](https://learn.microsoft.com/en-us/windows/win32/hidpi/wm-dpichanged)

### 5. `MonitorFromWindow` 与 `GetDpiForMonitor`

`MonitorFromWindow` 按窗口矩形与显示器矩形的最大相交面积返回 `HMONITOR`。每台宿主在放置后调用该 API，可以确认 HWND 实际落在哪台显示器。

`GetDpiForMonitor` 返回值受调用方 DPI awareness 影响。Microsoft 将它标记为非 DPI-aware API，并为 per-monitor-aware 调用方推荐 `GetDpiForWindow`。当前原生显示器枚举使用 `GetDpiForMonitor(MDT_EFFECTIVE_DPI)`；本次日志中它返回了正确的 `120 / 96 DPI`，适合描述目标显示器设置。实际窗口缩放仍应以 `GetDpiForWindow` 和窗口 awareness 为运行证据。

来源：

- [MonitorFromWindow](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-monitorfromwindow)
- [GetDpiForMonitor](https://learn.microsoft.com/en-us/windows/win32/api/shellscalingapi/nf-shellscalingapi-getdpiformonitor)

### 6. WebView2 `ParentWindow`、`Bounds` 与位置通知

`ICoreWebView2Controller::ParentWindow` 是 WebView2 用于渲染和接收输入的直接父 HWND。`Bounds` 使用该父 HWND 的相对坐标。当前实现通过 Wry `build_as_child(&raw_window)` 创建 Controller，Controller 的直接父窗口继续属于应用自有宿主，这一关系符合官方契约。

`NotifyParentWindowPositionChanged` 专门通知 WebView2 主父 HWND 或任意祖先 HWND 的移动，作用覆盖 accessibility 和 WebView 对话框定位。它和 `Bounds` 是独立契约，也不负责指定栅格化 DPI。当前实现于 `SetParent` 和定位后调用一次该方法，调用时机符合本次祖先位置变化。

来源：[ICoreWebView2Controller](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2controller?view=webview2-1.0.3537.50)

### 7. WebView2 `RasterizationScale`

`ICoreWebView2Controller3::ShouldDetectMonitorScaleChanges` 控制 WebView2 是否自动跟踪显示器 DPI。启用时，WebView2 更新 `RasterizationScale` 并触发 `RasterizationScaleChanged`；关闭时，宿主负责写入 `RasterizationScale`。`RasterizationScale` 作用于网页、弹出层、菜单和滚动条。`BoundsMode=USE_RAW_PIXELS` 时，`Bounds` 保持原始物理像素大小，修改栅格化比例只改变内容缩放。

当前实现已通过 `ICoreWebView2Controller3` 设置 `BoundsMode=USE_RAW_PIXELS`、关闭 `ShouldDetectMonitorScaleChanges`，并按目标显示器的 `MonitorInfo.scale_factor` 写入 `RasterizationScale`；随后读取三项实际值写入 `webview2.dpi.configured` 日志。跨进程父级导致的 HWND 实际 DPI 仍通过 `GetDpiForWindow` 单独记录，前端 DPR、命中区和裁剪区域继续使用宿主记录的显示器缩放因子。

来源：[ICoreWebView2Controller3](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2controller3?view=webview2-1.0.3537.50)

## 与官方建议的符合度

| 项目 | 当前状态 | 判定 |
| --- | --- | --- |
| 每台显示器创建独立自有宿主 | 已实现 | 符合逐屏承载基础 |
| 启动前申请 PMv2 | Tao 在事件循环初始化时执行；仓库无显式 manifest | 部分符合，manifest 是官方推荐形式 |
| 跨进程 `SetParent` 后保持逐屏 PMv2 | 日志显示两台窗口统一采用主屏 DPI | 不符合逐屏 DPI 目标，行为符合 Microsoft 的 forced reset 说明 |
| `GetDpiForWindow` 驱动实际布局 | 用于记录跨进程父级后的实际窗口 DPI；目标布局使用显示器快照 | 原型保留实际 DPI 与目标 DPI 的双重证据 |
| `WM_DPICHANGED` 使用建议矩形 | 当前通过轮询重建宿主 | 未按官方消息契约处理 |
| Controller `ParentWindow` 与相对 `Bounds` | 已实现 | 符合 |
| 祖先移动后通知 Controller | `SetParent` 后调用一次 | 符合本次移动路径 |
| WebView2 逐屏栅格化 | Controller3 手动写入每台显示器的 `RasterizationScale`，自动监视关闭 | 已落地，Windows 10 用户复测通过 |
| Rust 命中区与 WebView CSS 使用同一比例 | HostRecord、命中区、裁剪和 Controller3 统一使用目标显示器比例 | 已落地，Windows 10 用户复测通过 |

## 最小修正边界

当前 `WorkerW + 跨进程 SetParent` 路径使用显式的 WebView2 逐屏栅格化：

1. 每台宿主创建为顶层窗口并放到目标显示器后，用 `MonitorFromWindow` 核对显示器身份，用 `GetDpiForWindow` 读取重设父窗口前的目标窗口 DPI。
2. `SetParent` 后记录 `GetDpiForWindow` 和 Controller3 当前 `RasterizationScale`，把 forced reset 作为明确运行状态。
3. 对 Controller3 设置 `ShouldDetectMonitorScaleChanges=FALSE`，按对应显示器的有效 DPI 更新 `RasterizationScale`；`BoundsMode` 保持 raw pixels，Controller `Bounds` 保持父窗口物理客户区。
4. 前端 `devicePixelRatio`、`HostRecord.scale_factor`、命中测试 CSS/物理坐标换算和 `SetWindowRgn` 使用同一台显示器的有效比例。
5. 显示器 DPI 变化时更新上述比例与物理 Bounds。当前原型通过整组宿主重建完成更新；窗口级 `WM_DPICHANGED` 路径留给正式宿主。

使用应用自有顶层 HWND 并保持跨进程父子关系之外的层级方案，可直接保留 PMv2 的自动逐屏语义。当前原型继续采用 `WorkerW` 时，Controller3 手动栅格化是范围内最小的官方 WebView2 API 修正路径。
