---
status: accepted
---

# 在专用 STA 运行时执行 Shell 操作

Desktop Tools 在进程内建立一个 Shell STA 运行时，负责桌面命名空间、文件操作、项目菜单和变化通知；桌面宿主 UI STA 负责对应 HWND 的 OLE drop target 与源端 `DoDragDrop`。源数据在 Shell STA 准备，消费单次凭据后通过 `AgileReference<IDataObject>` 封送，在 UI STA 重新解析；Shell STA 继续处理 Windows 与 COM 消息，不等待整个源拖放结束。Rust 与 React 通过异步命令、不可变快照和操作结果通信。专用原生 owner HWND 负责项目菜单和文件操作确认，OLE 输入生命周期归接收鼠标输入的宿主线程。

源拖放期间的独立检查线程只观察物理按键和当前源线程捕获窗口，不调用 Shell COM。释放消息丢失且尚未进入 Drop 时，向 OLE 捕获窗口请求取消；已进入 Drop 后不据此中断文件操作。OLE 返回后停止并回收检查线程。虚拟桌面切换本身不触发取消。

源拖放放在宿主 UI STA，使鼠标移动、释放和 Escape 与接收窗口保持同一输入路径，避免独立 Shell STA 长期占用 OLE 捕获并阻塞后续操作。Renderer 的 `ole-preview` 确认只更新受锁保护的几何数据，直接在 transport 工作线程接收；需要操作 WebView 的其他消息仍由 UI STA 处理。投放不等待页面同步答复，缺少当前渲染确认时拒绝提交。

Shell STA 使用请求事件与 Windows 消息队列的原生联合等待；请求到达时设置事件，STA 通过 `MsgWaitForMultipleObjects` 同时等待请求和 Windows/COM 消息，无请求且无消息时无限阻塞。请求通道断开、`WM_QUIT`、剪贴板消息、COM 回调和既有错误处理保持原语义。设置窗口的桌面项目枚举命令通过异步 Tauri 入口执行，UI 线程不等待 Shell 枚举；逐屏原生桥接继续在阻塞工作池执行命令。内部组投放使用源窗口已确认的可见排列，不使用围栏成员保留的旧桌面位置决定组内偏移。

原生上下文菜单使用隐藏 `ShellMenuOwner` 作为 `TrackPopupMenu` 的 owner。弹出前尝试将 owner 置为前台，并在菜单跟踪结束后投递 `WM_NULL`；owner 处理 `WM_ACTIVATEAPP` 失活和 `WM_CANCELMODE`，调用 `EndMenu` 结束当前线程的菜单跟踪。该规则用于处理菜单外部点击、窗口失活和取消路径；尚未经真实 Windows 焦点场景验证。

## 系统图像提取线程

系统缩略图提取使用一个独立图像 STA 工作线程，与处理枚举、菜单和文件操作的 Shell STA 分离，避免无缓存文件或慢缩略图处理器延迟交互命令。图像线程按需处理请求，空闲时阻塞等待请求与 Windows/COM 消息，不轮询、不为每个项目创建线程。

COM 初始化、Shell 图像接口创建、位图转换和释放均在所属图像线程完成；线程间传递项目引用、请求身份和已复制的结果，不直接共享未经封送的 COM 接口。相同请求合并，失效请求的迟到结果不得覆盖当前图像。关闭时停止接收请求、丢弃未执行任务并回收线程资源；不得将终止线程作为中断第三方处理器的方法。

适用范围为桌面项目、围栏成员及其原生拖动图像。加载期间和提取失败保留系统图标，图像提取不成为桌面接管或 OLE 启动的同步等待门槛。图像内容边界见 [ADR-0005](0005-锁定里程碑1交付边界.md)。独立线程由 `shell/images.rs` 实现，平台提取和位图转换由 `platform/windows/shell_image.rs` 承担；第三方处理器阻塞时的退出响应、消息处理及资源释放尚未经真实环境验证。

请求以项目引用、物理尺寸和失效代次去重。当前 LRU 最多保存 256 个结果，像素及编码数据合计不超过 16 MiB；无缩略图结果也计入容量。队列最多接收 256 个不同请求，超出时返回图标回退。当前 Shell 通知除重命名外只提供整体失效，因此通知保守清空图像缓存，并在提交前检查代次，不宣称已实现逐文件精确失效。正常空闲不轮询，关闭等待已进入的系统调用完成，不能保证第三方处理器永久阻塞时及时退出。

## 门户新建菜单的已确认方向

门户继续使用统一的程序自绘桌面格子菜单，按类型增加“刷新”和“新建”；系统“新建”子菜单项在程序二级菜单中呈现。文件与文件夹项目的原生上下文菜单不因此改变。自绘菜单可以枚举并代理执行原生命令，不要求直接显示原生菜单窗口；这不代表所有第三方扩展均已验证兼容。

Rust 在 Shell STA 中获取门户当前目录的背景菜单，初始化动态“新建”子菜单，向前端提供名称、可用状态、可提取图标与临时命令标识。后端保留对应 `IContextMenu`、`HMENU` 和命令映射；原生命令通过原始命令偏移调用 `InvokeCommand`。菜单上下文结束并完成所选命令调用后释放资源，不能因自绘菜单先关闭而提前销毁执行所需对象。标识仅在本次菜单上下文有效，不持久化，不按中文或英文标签推断文件类型。

## 先命名后创建的能力边界

用户要求新建时先呈现待命名项，确认名称后才创建磁盘内容，取消不产生文件。Windows 没有覆盖所有“新建”类型的统一先命名接口：`IContextMenu::InvokeCommand` 没有通用最终文件名参数；`IFileOperation::NewItem` 支持名称和模板，但不能替代任意第三方 `ShellNew` 命令。

| 创建类型 | 先命名后创建的条件 |
| --- | --- |
| 文件夹 | 确认名称后创建目录 |
| `NullFile` | 可靠识别后创建空文件 |
| `Data` | 保留注册的预定义内容 |
| `FileName` | 正确解析并使用模板文件 |
| `Command` | 可能启动程序或向导，不能统一保证 |

待命名流程必须由后端校验名称、扩展名、权限和重名冲突，不覆盖已有文件；`Esc` 取消不产生磁盘内容。待命名项不参与打开、复制和拖放；创建失败保留输入并显示错误；成功结果与目录监听合并，避免重复项目。创建目标固定为发起操作时的目录。

## 新建流程决策

文件夹及可靠识别的 `NullFile`、`Data`、`FileName` 类型使用先命名后创建流程；向导型或无法可靠解析的类型交给系统原生命令执行，后者不保证取消命名时不产生文件。此分流保留系统新建选项，同时将取消不落盘的保证限定在可控制的待命名流程内，不承诺所有扩展支持统一命名。

待命名期间导航或更换绑定目录，先取消待命名项再切换，不创建文件；已经提交的创建操作仍在提交时确定的目录完成，不随门户导航改变目标。

## 当前实现与未验证项

`src/features/tiles/DesktopTileMenu.tsx` 已有 `children` 和 `onSelect`，可复用二级菜单；`src-tauri/src/platform/windows/shell.rs` 的 `folder_background_menu` 仍受 `#[cfg(test)]` 限制，不是产品已有的任意目录背景菜单能力。按中英文标签推断创建类型的做法不能用于产品。

此次依据只读代码核查与微软文档研究，未在真实环境运行新建流程。菜单项与创建类型的可靠映射、动态菜单生命周期、自绘兼容性、图标提取、模板解析、向导类型、Unicode 名称、取消、冲突及目录监听去重均尚未验证。

## API 依据

- [GetMenuItemInfoW](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getmenuiteminfow)
- [IContextMenu3::HandleMenuMsg2](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-icontextmenu3-handlemenumsg2)
- [IContextMenu::InvokeCommand](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-icontextmenu-invokecommand) 与 [CMINVOKECOMMANDINFOEX](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/ns-shobjidl_core-cminvokecommandinfoex)
- [ShellNew 创建方式](https://learn.microsoft.com/en-us/windows/win32/shell/context-menu-handlers#extending-a-new-submenu)
- [IFileOperation::NewItem](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-ifileoperation-newitem)
