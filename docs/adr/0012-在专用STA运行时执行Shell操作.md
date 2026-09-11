---
status: accepted
---

# 在专用 STA 运行时执行 Shell 操作

Desktop Tools 在进程内建立一个 Shell STA 运行时，负责桌面命名空间、文件操作、项目菜单和变化通知；桌面宿主 UI STA 负责对应 HWND 的 OLE drop target 与源端 `DoDragDrop`。源数据在 Shell STA 准备，消费单次凭据后通过 `AgileReference<IDataObject>` 封送，在 UI STA 重新解析；Shell STA 继续处理 Windows 与 COM 消息，不等待整个源拖放结束。Rust 与 React 通过异步命令、不可变快照和操作结果通信。专用原生 owner HWND 负责项目菜单和文件操作确认，OLE 输入生命周期归接收鼠标输入的宿主线程。

源拖放期间的独立检查线程只观察物理按键和当前源线程捕获窗口，不调用 Shell COM。释放消息丢失且尚未进入 Drop 时，向 OLE 捕获窗口请求取消；已进入 Drop 后不据此中断文件操作。OLE 返回后停止并回收检查线程。虚拟桌面切换本身不触发取消。

源拖放放在宿主 UI STA，使鼠标移动、释放和 Escape 与接收窗口保持同一输入路径，避免独立 Shell STA 长期占用 OLE 捕获并阻塞后续操作。Renderer 的 `ole-preview` 确认只更新受锁保护的几何数据，直接在 transport 工作线程接收；需要操作 WebView 的其他消息仍由 UI STA 处理。投放不等待页面同步答复，缺少当前渲染确认时拒绝提交。

已在双屏 Windows 环境验证正常源投放、撤销、连续 Escape 取消及后续原生菜单响应；真实 OLE 回归验证释放丢失时能够退出且不取消已开始的 Drop。长时间连续使用及第三方目标兼容性仍需持续验收。

Shell STA 空闲等待请求时仍处理 Windows 与 COM 消息，不使用无限阻塞的通道接收代替消息泵。设置窗口的桌面项目枚举命令通过异步 Tauri 入口执行，UI 线程不等待 Shell 枚举；逐屏原生桥接继续在阻塞工作池执行命令。内部组投放使用源窗口已确认的可见排列，不使用围栏成员保留的旧桌面位置决定组内偏移。

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

## 当前实现与待验证项

静态代码证据：`src/features/tiles/DesktopTileMenu.tsx` 已有 `children` 和 `onSelect`，可复用二级菜单；`src-tauri/src/platform/windows/shell.rs` 的 `folder_background_menu` 仍受 `#[cfg(test)]` 限制，不是产品已有的任意目录背景菜单能力。`invoke_menu_command` 封装原生命令调用，`local_background_menu_creates_text_document` 包含动态菜单初始化及创建 TXT 的测试代码，其按中英文标签查找类型的方法不能直接用于产品。

此次依据只读代码核查与微软文档研究，未运行会创建文件的测试，不能表述为运行验证通过。仍需验证菜单项与创建类型的可靠映射、动态菜单生命周期、自绘兼容性、图标提取、模板解析、向导类型、Unicode 名称、取消、冲突及目录监听去重。

## API 依据

- [GetMenuItemInfoW](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getmenuiteminfow)
- [IContextMenu3::HandleMenuMsg2](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-icontextmenu3-handlemenumsg2)
- [IContextMenu::InvokeCommand](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-icontextmenu-invokecommand) 与 [CMINVOKECOMMANDINFOEX](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/ns-shobjidl_core-cminvokecommandinfoex)
- [ShellNew 创建方式](https://learn.microsoft.com/en-us/windows/win32/shell/context-menu-handlers#extending-a-new-submenu)
- [IFileOperation::NewItem](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-ifileoperation-newitem)
