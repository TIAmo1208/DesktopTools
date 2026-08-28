# Windows 桌面宿主官方 API 审查

- 审查日期：2026-08-21
- 审查对象：`prototypes/tauri-desktop-host`
- 锁定技术栈：Tauri `2.11.5`、WebView2、Rust、React
- 资料范围：Tauri 2 Rust API、Microsoft WebView2、Win32、Windows Shell 官方文档

## 结论

Rust、Tauri 2、WebView2、React 组成的产品技术栈符合 Desktop Tools 的渲染、IPC、设置界面和原生 Shell 操作需求。当前原型的关键风险位于桌面宿主接入层：它把 Tauri 管理的顶层窗口改成 Explorer 跨进程 `WorkerW` 子窗口，并依赖 `Progman`、`WorkerW`、`SHELLDLL_DefView`、`SysListView32` 和消息 `0x052C`。这些 Explorer 类名、窗口层级和消息语义属于私有实现路径，Microsoft 公共 API 契约没有覆盖它们。

Windows 10 人工结果已经使当前原型判定为失败：壁纸正常、Explorer 快捷方式隐藏、Desktop Tools 项目内容缺失。日志只证明 JavaScript 已运行、窗口具有 `WS_VISIBLE` 状态以及隐藏动作完成；Microsoft 明确说明 `IsWindowVisible` 返回非零时窗口仍可能被完全遮挡或被父窗口裁剪。因此 `frontend.ready`、`hosts.visibility` 和 `takeover.committed` 不能作为像素呈现证据。

当前 Issue 已将 Windows 10 视觉结果记录为宿主路线失败，并把日志事件限定为诊断证据。Desktop Tools 继续采用自绘桌面项目，宿主验证按公开基线、创建期实验、独立原生宿主和组合宿主分层推进。

## 证据分类

本文使用四类证据，结论按以下优先级解释：

1. **官方事实**：Tauri 2.11.5 API 文档和 Microsoft Learn 的公开契约。
2. **当前实现事实**：当前仓库实现实际调用的 API 与私有标识。
3. **运行证据**：日志与用户人工观察到的可见结果。
4. **工程推论**：由前三类证据得出的原型判定和后续建议。

## 1. Tauri 2 的正式窗口与 WebView 契约

### 官方事实

Tauri `Window` 是“Tauri 管理的窗口”。`Window::add_child` 的公开语义是在该窗口中添加子 WebView；该接口及 `WindowBuilder`、`WebviewBuilder` 在 Tauri `2.11.5` 中均受 `unstable` feature 约束。

- [`Window::add_child`](https://docs.rs/tauri/2.11.5/tauri/window/struct.Window.html#method.add_child)
- [`WindowBuilder`](https://docs.rs/tauri/2.11.5/tauri/window/struct.WindowBuilder.html)
- [`WebviewBuilder`](https://docs.rs/tauri/2.11.5/tauri/webview/struct.WebviewBuilder.html)

Tauri 的 `WindowBuilder::parent` 在 Windows 上建立 owner/owned-window 关系。官方说明包括 owned window 位于 owner 上方、owner 销毁时一并销毁、owner 最小化时隐藏。

- [`WindowBuilder::parent`](https://docs.rs/tauri/2.11.5/tauri/window/struct.WindowBuilder.html#method.parent)

Tauri `2.11.5` 提供 Windows 专用低层接口 `WindowBuilder::parent_raw(HWND)`。API 契约明确：窗口在创建时获得 `WS_CHILD`，并受父窗口客户区裁剪。该接口能够在窗口创建阶段接受外部 HWND；目标 HWND 的语义和稳定性仍由调用方负责。

- [Tauri 2.11.5 `WindowBuilder::parent_raw`](https://docs.rs/tauri/2.11.5/tauri/window/struct.WindowBuilder.html#method.parent_raw)

Tauri 支持透明窗口和透明 WebView。Windows 上 `WindowBuilder::background_color` 忽略 alpha；WebView 背景只支持 alpha `0` 或 `255`。透明窗口能力来自 `transparent(true)` 与 WebView 透明背景的组合，`background_color(Color(..., 0))` 本身无法证明外层 HWND 逐像素透明。

- [`WindowBuilder::transparent` 与 `background_color`](https://docs.rs/tauri/2.11.5/tauri/window/struct.WindowBuilder.html#method.transparent)
- [`Webview::set_background_color`](https://docs.rs/tauri/2.11.5/tauri/webview/struct.Webview.html#method.set_background_color)

Tauri 提供 `Webview::with_webview` 访问 Windows 原生 WebView2 controller。这是读取 `ParentWindow`、`Bounds`、`IsVisible` 以及调用 WebView2 通知接口的正式扩展点。

- [`Webview::with_webview`](https://docs.rs/tauri/2.11.5/tauri/webview/struct.Webview.html#method.with_webview)

### 当前实现事实

当前项目显式启用 `tauri = { version = "2", features = ["unstable"] }`。`main.rs` 创建隐藏透明 Tauri 窗口，调用 `window.add_child(...)` 创建 WebView，再通过原始 HWND 操作改写 Tauri 窗口层级：

- `main.rs:300-339`：`WindowBuilder`、透明外层窗口、`Window::add_child` 和透明 WebView。
- `main.rs:394`：WebView 创建后调用 `attach_host`。
- `platform.rs:554`：改写 `WS_POPUP/WS_CHILD`、扩展样式并调用 `SetParent`。

### 判定

Tauri 管理自有窗口和自有子 WebView 属于框架覆盖范围。Tauri 还提供创建期 `parent_raw` 低层入口。当前实现先创建顶层窗口，再改样式并执行 `SetParent`；这条后置重挂载路径偏离了框架提供的创建期父窗口流程。`add_child` 的 `unstable` 状态及 Explorer 私有父 HWND 继续构成升级和兼容风险。

## 2. WebView2 的 HWND、透明与可见性契约

### 官方事实

`CreateCoreWebView2Controller(parentWindow, ...)` 要求宿主提供用于显示和接收输入的 HWND。WebView2 会在该 HWND 下创建子窗口。WebView2 移动到另一个父窗口时，调用方必须使用 `put_ParentWindow`，以更新 tooltip、无障碍树等关联状态。

- [`ICoreWebView2Environment::CreateCoreWebView2Controller`](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2environment?view=webview2-1.0.3967.48#createcorewebview2controller)

`ICoreWebView2Controller` 规定：

- `Bounds` 使用父 HWND 的坐标空间；
- `IsVisible` 只控制 WebView2，宿主 HWND 仍需由应用调用 `ShowWindow`；
- `ParentWindow` 是创建 controller 时传入的宿主 HWND；
- 父 HWND 或任一祖先 HWND 移动后，宿主应调用 `NotifyParentWindowPositionChanged`。

- [`ICoreWebView2Controller`](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2controller?view=webview2-1.0.3967.48)

`ICoreWebView2Controller2::DefaultBackgroundColor` 支持全透明 alpha `0` 和不透明 alpha `255`。官方同时记录：通过 API 设置背景色可能在生效前产生白色闪烁，环境变量 `WEBVIEW2_DEFAULT_BACKGROUND_COLOR` 可覆盖这一启动间隙。

- [`ICoreWebView2Controller2::put_DefaultBackgroundColor`](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2controller2?view=webview2-1.0.3967.48#put_defaultbackgroundcolor)

### 当前实现事实

WebView2 controller 的直接父窗口始终是 Tauri 外层窗口；项目随后把这个外层窗口移动到 `WorkerW` 并改变其坐标和样式。当前源码没有通过 `with_webview` 读取 controller 的 `ParentWindow/Bounds/IsVisible`，也没有在外层窗口祖先关系与位置变化后显式调用 `NotifyParentWindowPositionChanged`。

### 判定

WebView2 支持透明渲染和应用自有 HWND 宿主。当前实现修改宿主的祖先窗口、样式和坐标，缺少 WebView2 controller 级状态核对与祖先位置通知。该缺口足以让“页面完成加载”与“像素出现在目标桌面层”成为两个独立结果。

## 3. `SetParent` 与 Explorer `WorkerW` 路径

### 官方事实

`SetParent` 是正式 Win32 API，可修改子窗口的父 HWND。调用方负责同步 `WS_CHILD/WS_POPUP`；跨进程 DPI awareness 不一致会触发 child 进程 DPI 重置。该契约只覆盖通用父子窗口操作，没有承诺 Explorer 内部窗口可作为第三方桌面内容宿主。

- [`SetParent`](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setparent)

`0x052C` 等于 `WM_USER + 300`。Microsoft 把 `WM_USER` 至 `0x7FFF` 定义为私有窗口类消息范围，并明确要求跨应用发送时双方应预先设计并共享消息语义。Windows SDK 的公开 Shell 契约没有为 `Progman + 0x052C` 定义“创建 WorkerW”的语义。

- [`WM_USER`](https://learn.microsoft.com/en-us/windows/win32/winmsg/wm-user)

Windows 提供 `GetShellWindow` 获取 Shell 桌面窗口 HWND。该 API没有公开 `Progman -> WorkerW -> SHELLDLL_DefView -> SysListView32` 层级，也没有公开“图标宿主后的 WorkerW”标识。

- [`GetShellWindow`](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getshellwindow)

`FindWindowExW` 和 `ShowWindow` 是正式窗口 API。它们根据调用方提供的类名定位窗口并改变显示状态；Explorer 私有类名和层级的稳定性仍由调用方承担。

- [`FindWindowExW`](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-findwindowexw)
- [`ShowWindow`](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-showwindow)

### 当前实现事实

- `platform.rs:54`：把 `0x052C` 命名为 Explorer 创建桌面 `WorkerW` 的私有消息。
- `platform.rs:313-389`：按 `Progman`、`SHELLDLL_DefView`、`SysListView32` 和相邻 `WorkerW` 推导图标宿主及渲染宿主。
- `platform.rs:391-431`：跨进程向 `Progman` 发送 `0x052C`。
- `platform.rs:444-503`：对 `SHELLDLL_DefView` 调用 `ShowWindowAsync` 隐藏/恢复 Explorer 项目。
- `platform.rs:554-613`：把 Tauri HWND 改为 Explorer `WorkerW` 子窗口。

### 判定

当前桌面层方案由正式 Win32 原语和私有 Explorer 拓扑共同组成。正式原语保证每次 API 调用的基础行为；Explorer 类名、兄弟顺序、消息参数、绘制层级及跨版本兼容性没有公共保证。Windows 10 22H2 与 Windows 11 25H2 必须分别通过真实像素和交互验收，日志门槛只能作为诊断证据。

## 4. Windows Shell 项目与图标可见性

### 官方事实

Windows Shell 提供完整桌面命名空间入口：`SHGetDesktopFolder` 返回桌面根 `IShellFolder`，`IShellFolder::EnumObjects` 枚举该命名空间内容。`IContextMenu` 可通过 `IShellFolder::GetUIObjectOf` 获取并调用对象的 Shell 菜单。

- [`SHGetDesktopFolder`](https://learn.microsoft.com/en-us/windows/win32/api/shlobj_core/nf-shlobj_core-shgetdesktopfolder)
- [`IShellFolder::EnumObjects`](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-ishellfolder-enumobjects)
- [`IContextMenu`](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nn-shobjidl_core-icontextmenu)

Shell 还公开 `SHGetSetSettings`、`SHELLSTATE::fHideIcons` 和 `SSF_HIDEICONS` 读取或设置桌面图标状态。Microsoft 对 `SHGetSetSettings` 标注“后续版本可能更改或不可用”；它仍是公开文档中的 Shell 状态接口，语义强度高于直接隐藏 `SHELLDLL_DefView`。

- [`SHGetSetSettings`](https://learn.microsoft.com/en-us/windows/win32/api/shlobj_core/nf-shlobj_core-shgetsetsettings)
- [`SHELLSTATEW::fHideIcons`](https://learn.microsoft.com/en-us/windows/win32/api/shlobj_core/ns-shlobj_core-shellstatew)
- [`SSF_HIDEICONS`](https://learn.microsoft.com/en-us/windows/win32/shell/ssf-constants)

### 当前实现事实

`platform.rs:1009-1078` 读取用户桌面目录、公共桌面目录，再追加固定的 5 个 CLSID。该集合是当前实现的白名单，覆盖范围小于 Shell 桌面根命名空间。`platform.rs:1136` 通过 `IShellFolder::GetUIObjectOf<IContextMenu>` 获取项目菜单，这部分方向符合公开 Shell API。

### 判定

“完整桌面项目集合”的实现应改为 `SHGetDesktopFolder + IShellFolder::EnumObjects`，再对 PIDL 获取名称、图标、稳定标识和 `IContextMenu`。当前“双目录 + 5 个 CLSID”只能作为受限样例数据源。

图标隐藏有两条边界清晰的选择：

1. 使用公开 Shell 状态接口管理 `fHideIcons`，保存并恢复用户原值，同时验证 Explorer 刷新行为和崩溃恢复。
2. 继续使用 `ShowWindow(SHELLDLL_DefView)` 获得临时隐藏效果，并把它标记为私有兼容层，按 Windows 版本建立回归矩阵。

## 5. 当前 Windows 10 运行证据

最新短时日志 `.scratch/desktop-host-prototype/logs/20260821-165630-probe.log` 记录：

- 第 25、31 行：两台显示器均产生 `frontend.ready`；
- 第 34、37 行：渲染宿主与两个 Tauri 宿主报告可见；
- 第 38、39 行：Explorer 项目隐藏并记录 `takeover.committed`；
- 第 43-45 行：退出时隐藏渲染宿主、恢复 Explorer 项目并完成 cleanup；
- 第 47 行：WebView2/Chromium 输出 `Failed to unregister class Chrome_WidgetWin_0. Error = 1412`。

用户人工观察：壁纸正常、Desktop Tools 项目内容缺失、Explorer 快捷方式隐藏。

Microsoft 对 `IsWindowVisible` 的定义是：目标及祖先拥有 `WS_VISIBLE` 时返回非零；窗口即使完全被遮挡或被父窗口裁剪，也可能返回非零。

- [`IsWindowVisible`](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-iswindowvisible)

因此本轮日志证明“前端执行完成、窗口样式链可见、Explorer 图标已隐藏、恢复路径执行”，人工结果证明“目标像素呈现失败”。原型 Issue 已采用人工像素结果作为当前宿主路线的验收结论。

## 6. 原型方向判定

| 方向 | 官方契约状态 | 当前结论 |
| --- | --- | --- |
| Rust 负责 Windows 原生集成、恢复和 guardian | Rust 调用 Win32/COM 公共 API | 保留 |
| React 负责桌面项目、围栏和组件呈现 | WebView2 支持 Web 内容渲染 | 保留 |
| Tauri 负责 IPC、应用生命周期、设置窗口 | Tauri 正式能力 | 保留 |
| Tauri 透明自有窗口 + 自有子 WebView | 透明能力正式，`add_child` 为 unstable | 可用于渲染基线实验 |
| Tauri HWND 创建后经原始 `SetParent` 变成 Explorer `WorkerW` 子窗口 | Tauri 提供创建期 `parent_raw`；当前后置重挂载绕过框架流程；父 HWND 仍为私有 Explorer 拓扑 | 当前后置重挂载路线停止继续修补 |
| `0x052C` 请求 Explorer 创建 `WorkerW` | `WM_USER` 私有消息 | 仅作为受控兼容实验 |
| `ShowWindow(SHELLDLL_DefView)` 暂时隐藏图标 | API 正式，目标类与层级私有 | 作为兼容层并保留 guardian |
| 双桌面目录 + 5 个 CLSID | Shell 提供完整命名空间枚举 API | 改为桌面 `IShellFolder` 枚举 |
| `IContextMenu` 获取真实 Shell 菜单 | 正式 Shell COM API | 保留并补齐消息转发测试 |

## 7. 下一方向

### 原型判定

当前 Tauri/WorkerW 原型按 Milestone 1 强制验收矩阵记为 Windows 10 22H2 失败。Windows 11 25H2 仍需独立记录；Windows 10 的失败已经足以阻止该实现直接进入正式代码。

### 推荐实施边界

1. **保留产品栈**：Rust、Tauri、WebView2、React 继续承担当前项目职责。
2. **建立公开宿主基线**：依次验证 Windowed 与 Window-to-Visual 自有 HWND 的透明像素、工作区和输入。
3. **执行一次创建期宿主实验**：在最终父 HWND 下创建 Tauri 宿主与子 WebView，并以双系统真实像素作为通过条件。
4. **验证独立原生宿主**：创建期实验失败后，由原生桌面宿主持有 HWND、WebView2 Controller、WndProc、OLE target 与恢复状态；Tauri 保留控制面、设置和应用生命周期。
5. **验证组合宿主**：独立 Windowed 合成失败时，由原生桌面宿主验证 CompositionController Visual hosting。
6. **锁定严格阻塞门槛**：全部宿主路线仍有强制失败时，Milestone 1 进入阻塞状态。

### 当前项目架构结论

Desktop Tools 自绘并接管完整桌面项目，Explorer 提供桌面命名空间、Shell 操作和恢复兜底。Explorer 私有桌面层能力集中在可替换的兼容适配器中，宿主失败进入严格阻塞状态。

## 8. 当前项目交叉引用

- Milestone 1 验收边界：ADR 0005。
- 自绘桌面项目接管：ADR 0006；ADR 0010 保留为被取代的历史记录。
- Explorer 私有兼容层和宿主验证层级：ADR 0011。
- Shell STA 运行时：ADR 0012。
- 桌面项目引用：ADR 0013。
