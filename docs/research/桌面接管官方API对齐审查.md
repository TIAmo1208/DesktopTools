# Desktop Tools 桌面接管官方 API 对齐审查

- 审查日期：2026-08-22
- 审查范围：Windows Shell、OLE、WebView2、Win32、Tauri 2.11.5
- 需求边界：Desktop Tools 呈现并操作桌面项目；Explorer 提供桌面命名空间、Shell 操作与恢复兜底
- 资料等级：Microsoft Learn 与 Tauri 官方 API 文档

## 结论

Desktop Tools 自绘桌面项目的产品方向具备公开 API 基础。Windows Shell 可以向 Rust broker 提供桌面命名空间、PIDL、显示名称、系统图像、文件操作、真实上下文菜单、OLE 数据对象与变更通知；React/WebView2 可以承载项目、围栏和组件的呈现与交互。

公开 Shell API 直接提供项目身份、文件操作、真实菜单、系统剪贴板和 OLE 拖放语义。公开契约未提供 Explorer 桌面视图的选择与焦点状态机；选择模型、框选规则、键盘导航、围栏布局、项目排序、内部滚动、折叠状态、跨显示器归属和拖动反馈由 Desktop Tools 产品层实现，并以 Windows 系统交互作为强制验收基线。

Microsoft 公开契约覆盖 WebView2 的应用自有 HWND 和合成视觉树；Explorer `WorkerW`、`Progman + 0x052C`、`SHELLDLL_DefView` 的第三方宿主语义属于私有兼容层。当前 Windows 10 人工结果判定“创建后 `SetParent` 到 WorkerW”路径失败，正式实现已改用独立 Win32/WebView2 Windowed 宿主并聚焦当前平台运行结果。

`Tauri WindowBuilder::parent_raw` 创建期实验到独立 Win32/WebView2 适配器的顺序正确。创建期实验只验证 Tauri 生命周期是否造成当前失败；它保持一次、短周期和明确退出条件。实验失败后，独立适配器直接持有最终 HWND、WebView2 controller、OLE drop target 与消息循环。其他 Windows 版本保留兼容入口，但不再阻塞当前 Windows 10 平台实现。

## 能力对齐矩阵

| 需求 | 官方 API 基础 | 实现归属 | 判定 |
| --- | --- | --- | --- |
| 完整桌面命名空间 | `SHGetDesktopFolder`、`IShellFolder::EnumObjects` | Rust Shell broker | 方向成立，当前枚举源需替换 |
| Shell 项目标识 | 绝对/相对 PIDL、`IShellItem::Compare`、桌面绝对解析名 | Rust broker + 持久化协调器 | 运行期成立，跨重命名持久化需产品协调 |
| 显示名称和系统图像 | `SHGetNameFromIDList`、`SHGetFileInfoW`、`IShellItemImageFactory` | Rust Shell broker | 方向成立 |
| 打开、重命名、删除、复制、移动 | 默认 Shell verb、`IFileOperation` | Rust Shell broker | 方向成立 |
| 真实项目菜单 | `IContextMenu`、`IContextMenu2`、`IContextMenu3` | Rust 原生菜单宿主 | 方向成立，需补齐消息转发 |
| 系统剪贴板与跨应用拖放 | `IDataObject`、`IDropSource`、`IDropTarget`、`DoDragDrop` | Rust OLE broker | 方向成立，需原生消息循环 |
| 文件与命名空间实时同步 | `SHChangeNotifyRegister` | Rust Shell broker | 方向成立，通知后需重新查询 |
| 围栏、选择、排序、滚动 | Shell API 提供项目能力，布局由应用定义 | React + Rust 布局模型 | 产品自定义语义 |
| Web 内容透明渲染 | WebView2 windowed/window-to-visual/visual hosting、透明背景 | WebView2 宿主 | 公开能力成立 |
| Explorer 桌面层嵌入 | 通用 `SetParent` 加 Explorer 私有窗口拓扑 | 私有兼容适配器 | 当前阻塞项 |

## 1. 桌面命名空间与项目标识

[`SHGetDesktopFolder`](https://learn.microsoft.com/en-us/windows/win32/api/shlobj_core/nf-shlobj_core-shgetdesktopfolder) 返回代表 Shell 命名空间根节点的 `IShellFolder`。[`IShellFolder::EnumObjects`](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-ishellfolder-enumobjects) 按 `SHCONTF` 标志返回 `IEnumIDList`，由调用方枚举文件系统项目与虚拟项目。该入口应成为桌面项目集合的唯一来源，并依据产品可见性规则选择 `SHCONTF_FOLDERS`、`SHCONTF_NONFOLDERS` 和隐藏项策略。

[`ITEMIDLIST`](https://learn.microsoft.com/en-us/windows/win32/api/shtypes/ns-shtypes-itemidlist) 文档把绝对 PIDL 定义为相对命名空间根节点的完整标识，把子 PIDL 定义为相对父文件夹的单级标识。[Shell Namespace 介绍](https://learn.microsoft.com/en-us/windows/win32/shell/namespace-intro) 要求应用把 item ID 当作所属文件夹解释的 opaque token。PIDL 可以标识文件系统项目和回收站等虚拟项目，文件路径只覆盖 `SFGAO_FILESYSTEM` 项目。

项目模型建议采用以下边界：

```text
ShellItemSnapshot
  runtime_id              当前进程内的稳定句柄
  absolute_pidl           broker 持有的 opaque 绝对 PIDL
  desktop_parsing_name    可解析时用于重建 IShellItem
  display_name            当前本地化显示名称
  attributes              SFGAO 能力和类型
  operation_capabilities  当前可执行操作集合
  image_cache_key         图像尺寸、DPI 和版本键
  generation              命名空间快照代次
```

[`SIGDN_DESKTOPABSOLUTEPARSING`](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/ne-shobjidl_core-sigdn) 提供相对桌面的绝对解析名；`SIGDN_FILESYSPATH` 只适用于报告 `SFGAO_FILESYSTEM` 的项目。[`IShellItem::Compare`](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-ishellitem-compare) 可比较当前 `IShellItem`。这些接口支持运行期身份和重新解析。跨重命名、跨移动和跨会话的领域标识由 Desktop Tools 维护：处理 `SHCNE_RENAMEITEM/SHCNE_RENAMEFOLDER` 的旧、新 PIDL，更新项目引用；完整重扫时通过解析名、当前 PIDL 和文件系统补充标识完成协调。

围栏成员持久化应保存 `DesktopItemRef`，并允许“暂时无法解析”状态。当前 `path:<lowercase-path>` 和固定 CLSID 的覆盖范围为原型样例；快捷方式自身、虚拟项目和重命名后的项目需要 Shell 标识路径。

Shell 命名空间描述逻辑项目集合。项目属于哪台显示器、位于未归类区域还是某个围栏、采用什么顺序，全部由 Desktop Tools 布局模型决定。

## 2. 显示名称、系统图标与缩略图

[`SHGetNameFromIDList`](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-shgetnamefromidlist) 接收绝对 PIDL 和 `SIGDN`。界面标题使用 `SIGDN_NORMALDISPLAY`，重命名编辑框可以使用适合编辑的名称形式。

当前原型通过 [`SHGetFileInfoW`](https://learn.microsoft.com/en-us/windows/win32/api/shellapi/nf-shellapi-shgetfileinfow) 加 `SHGFI_PIDL | SHGFI_ICON | SHGFI_DISPLAYNAME` 获取图标和 Explorer 风格显示名，这条 API 路径有效。调用方继续负责销毁返回的 `HICON`。

正式实现建议使用 [`IShellItemImageFactory::GetImage`](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-ishellitemimagefactory-getimage) 生成指定尺寸图像。官方文档明确说明图像提取可能访问磁盘并耗时，`SIIGBF_MEMORYONLY` 或 `SIIGBF_INCACHEONLY` 适合 UI 快速路径，缓存缺失后的提取应进入后台 STA worker。图像结果转换为共享位图、PNG 缓存或受控资源 URL 后交给 React。

系统图像缓存至少区分项目标识、物理像素尺寸、DPI、缩略图/图标模式和 Shell 图像更新代次。`SHCNE_UPDATEIMAGE` 与项目属性更新用于失效缓存。

## 3. 打开、重命名、删除、复制与移动

[`IFileOperation`](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nn-shobjidl_core-ifileoperation) 使用 `IShellItem` 表达文件系统与虚拟 Shell 项目，并公开复制、移动、重命名、创建和删除操作。它替代旧式 `SHFileOperation`，同时提供进度、错误对话框和 operation sink。

Rust broker 的操作顺序应为：

1. 在 STA Shell 线程创建 `CLSID_FileOperation`。
2. 调用 `SetOwnerWindow`，把确认、错误和进度 UI 归属到稳定的原生 owner HWND。
3. 调用 [`SetOperationFlags`](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-ifileoperation-setoperationflags) 明确回收站、撤销、确认、错误 UI 和冲突策略。
4. 通过 `RenameItem`、`DeleteItem`、`CopyItem` 或 `MoveItem` 排队。
5. 调用 [`PerformOperations`](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-ifileoperation-performoperations)。
6. 始终调用 [`GetAnyOperationsAborted`](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-ifileoperation-getanyoperationsaborted)，因为用户或系统中止时 `PerformOperations` 仍可能返回成功码。
7. 结合 `IFileOperationProgressSink` 和 Shell 变更通知刷新领域状态。

`F2`、`Delete`、复制、移动应先读取项目的 Shell 属性和可用操作，再更新 React 命令状态。虚拟项目的 handler 可以拒绝某项操作；界面以 broker 返回的 capability 和 operation result 为权威。

双击或 `Enter` 打开可以调用项目的默认 Shell verb。需要与上下文菜单的默认命令保持完全一致时，通过 `IContextMenu` 查询默认命令并调用；常规打开也可使用 `ShellExecuteEx` 处理 Shell 解析名。

围栏内排序和围栏间移动属于逻辑布局操作，保持 Shell 项目位置与文件路径。拖入真实文件夹、粘贴到桌面命名空间和应用间 copy/move 属于 Shell 数据操作，由 OLE 与 `IFileOperation` 执行。

### Shell 撤销契约

[`IFileOperation::SetOperationFlags`](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-ifileoperation-setoperationflags) 的 `FOFX_ADDUNDORECORD` 表示用户触发的操作应加入 Shell 撤销记录；`FOF_ALLOWUNDO` 请求 Shell 尽可能保存撤销信息。公共 `IFileOperation` 接口的能力边界止于写入撤销记录，读取、定位和执行 Explorer 会话撤销栈位于接口范围之外。

[`IFileOperationProgressSink`](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nn-shobjidl_core-ifileoperationprogresssink) 在操作完成后返回实际源项目、目标项目、最终名称、结果项目和 `HRESULT`。Desktop Tools 在当前应用会话中使用这些公开结果建立自有可逆操作日志，并通过新的 `IFileOperation` 提交重命名、移动、复制和移入回收站的补偿操作或重做，同时为原始操作与补偿操作分别请求写入 Explorer 会话撤销记录；永久删除保持不可撤销。补偿操作继续使用原生 Shell 确认与错误界面，部分成功和用户选择按实际结果更新日志。Shell 变化通知对账 Explorer 外部撤销或重做；明确匹配时更新对应记录，结果存在歧义时终止对应记录及后续重做链。当前虚拟桌面的布局历史与共享 Shell 操作日志按完成时间组成统一时间线；文本控件获得焦点时处理自身撤销，桌面内容工作区获得焦点时 `Ctrl+Z/Y` 作用于最近一项。

截至 2026-09-01，正式实现会在 `Pre*` 回调保存源项目父级和编辑名称，并在 `Post*` 回调保存 Shell 实际结果引用；只有结果引用和补偿元数据完整的成功项目进入可执行历史。移入回收站后若 Shell 没有返回可重新解析的结果引用，该项目明确保持不可补偿，而不是猜测回收站内容。统一时间线使用完成时间选择当前虚拟桌面的最近布局单元或共享 Shell 单元，部分补偿只把实际成功且仍有结果引用的项目带入相反方向。该链路的 Windows 10/11 实机行为仍属于最终验收范围，不能由冻结原型或静态检查替代。

### 工作区备份契约

冻结的 `tauri-desktop-host` 原型没有实现自动快照、手动备份、Schema 迁移、完整性校验或统一还原，因此这些能力不能声称来自原型运行验证。正式工作区文件使用 Schema 3：版本、创建时间和完整性字段包裹全部活动虚拟桌面布局、删除布局归档、显示器身份、围栏状态和桌面项目引用。完整性字段使用固定 FNV-1a 64 位值发现意外损坏，不承担签名或跨用户真实性证明。

主工作区文件、手动备份和自动快照共用 UTF-8 JSON 与同目录原子替换路径。每个虚拟桌面在最后一次持久变化空闲 2 秒后写入完整工作区快照，并按桌面身份保留最近 20 份；还原前同步写入的完整快照计入发起还原时当前桌面的同一配额。读取先在临时副本中识别 Schema 3、迁移 Ticket 10 的 Schema 2 或旧单布局格式，再校验完整性和每份布局。完整还原先构造候选状态，必要时为备份中缺失的布局创建新 Windows 虚拟桌面，然后执行“还原前快照、原子文件替换、内存替换”；较新版本、损坏、迁移失败或保存失败不修改当前工作区，新建桌面在失败路径回滚。

## 4. 真实 Shell 上下文菜单

[`IShellFolder::GetUIObjectOf`](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-ishellfolder-getuiobjectof) 可以针对同一父文件夹下的一组 child PIDL 获取 `IContextMenu`、`IDataObject` 或 `IDropTarget`。多选菜单应传递全部选中项；跨父文件夹选择需要分组、建立 Shell item array，或按产品规则限制菜单范围。

基础调用链为 `GetUIObjectOf(IContextMenu)`、[`QueryContextMenu`](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-icontextmenu-querycontextmenu)、显示原生 `HMENU`、[`InvokeCommand`](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-icontextmenu-invokecommand)。Unicode verb 需要 `CMINVOKECOMMANDINFOEX` 和 `CMIC_MASK_UNICODE`。

完整菜单宿主还需要依次查询 `IContextMenu3` 和 `IContextMenu2`：

- [`IContextMenu2::HandleMenuMsg`](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-icontextmenu2-handlemenumsg) 处理 owner-drawn 菜单使用的 `WM_INITMENUPOPUP`、`WM_DRAWITEM`、`WM_MEASUREITEM` 等消息。
- [`IContextMenu3::HandleMenuMsg2`](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-icontextmenu3-handlemenumsg2) 进一步返回消息结果并覆盖 `WM_MENUCHAR`。

当前原型只持有 `IContextMenu` 并调用 `TrackPopupMenu`。静态菜单可以工作，owner-drawn 项、延迟构建子菜单和部分 Shell extension 菜单仍缺少消息转发。正式 broker 应使用专门的原生菜单 owner HWND 和可控 WndProc，在菜单生命周期内保存 `IContextMenu3/2`。

## 5. 剪贴板与 OLE 跨应用拖放

[`IDataObject`](https://learn.microsoft.com/en-us/windows/win32/api/objidl/nn-objidl-idataobject) 是 OLE 和 Shell 数据传输容器。Shell 数据对象可以同时携带 `CF_HDROP`、`CFSTR_SHELLIDLIST`、虚拟文件内容和 `CFSTR_PREFERREDDROPEFFECT` 等格式。[Shell Clipboard Formats](https://learn.microsoft.com/en-us/windows/win32/shell/clipboard) 明确覆盖文件系统对象、虚拟对象以及 copy/move/delete-on-paste 协议。

向 Explorer 或其他应用拖出项目的原生链路为：

1. 对选中 child PIDL 调用 `IShellFolder::GetUIObjectOf(IDataObject)`，保留 Shell handler 提供的全部格式。
2. 实现 [`IDropSource`](https://learn.microsoft.com/en-us/windows/win32/api/oleidl/nn-oleidl-idropsource)，负责继续、取消、完成和鼠标反馈。
3. 在 `OleInitialize` 的 STA 线程调用 [`DoDragDrop`](https://learn.microsoft.com/en-us/windows/win32/api/ole2/nf-ole2-dodragdrop)，传入允许的 `DROPEFFECT_COPY/MOVE/LINK`。
4. 根据返回 effect 更新布局引用和 operation result。

从 Explorer 或其他应用拖入 Desktop Tools 的原生链路为：

1. 每个可接收拖放的宿主 HWND 调用 [`RegisterDragDrop`](https://learn.microsoft.com/en-us/windows/win32/api/ole2/nf-ole2-registerdragdrop)。官方要求调用线程使用 `OleInitialize` 并持续 pump messages。
2. Rust `IDropTarget` 实现 `DragEnter`、`DragOver`、`DragLeave` 和 `Drop`，依据命中图判断未归类区域、围栏、Shell 文件夹项目或组件。
3. 使用 [`SHCreateShellItemArrayFromDataObject`](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-shcreateshellitemarrayfromdataobject) 把 data object 转为 `IShellItemArray`。该数组持有源 data object，生命周期止于 drop 调用完成。
4. 逻辑归属变更写入布局模型；真实 copy/move/link 交给目标 Shell `IDropTarget` 或 `IFileOperation`。

React 内部拖放适合项目重排、围栏归属和视觉反馈。拖动跨出或跨入 WebView 边界时，Rust OLE broker 接管同一 drag session。HTML Drag and Drop 只携带浏览器可见的数据格式，完整 Shell 兼容路径需要保留原生 `IDataObject`。

`Ctrl+C/X/V` 使用同一 OLE data object 协议。复制与剪切把 Shell `IDataObject` 放入系统剪贴板，并设置 preferred drop effect；粘贴解析当前 clipboard data object，再交给桌面命名空间或目标 Shell 文件夹处理。

## 6. 文件系统与 Shell 命名空间变化

[`SHChangeNotifyRegister`](https://learn.microsoft.com/en-us/windows/win32/api/shlobj_core/nf-shlobj_core-shchangenotifyregister) 把文件系统和 Shell 变更消息发送给指定 HWND。桌面 broker 应分别为桌面命名空间根、用户桌面物理 PIDL 和公共桌面物理 PIDL 建立 registration，并按实际枚举来源验证事件覆盖。每次调用传入一条 `SHChangeNotifyEntry` 和 `cEntries=1`；官方明确要求这一取值以保证 deregister 正常工作。直接子项监听使用 `fRecursive=FALSE`，从而限制通知范围。

每个 registration 使用 `SHCNRF_NewDelivery`；官方明确推荐该模式，并要求通过 [`SHChangeNotification_Lock`](https://learn.microsoft.com/en-us/windows/win32/api/shlobj_core/nf-shlobj_core-shchangenotification_lock) 读取受影响 PIDL 和事件，再调用 unlock。

订阅事件至少覆盖创建、删除、目录创建/删除、项目/目录重命名、`SHCNE_UPDATEITEM`、`SHCNE_UPDATEDIR`、属性和图像更新。通知窗口由 Shell STA 线程拥有并持续 pump messages。

官方文档说明，大量 `SHCNE_UPDATEITEM` 可以合并为单个 `SHCNE_UPDATEDIR`。因此通知是状态失效与协调触发器。broker 收到精确 rename 事件时更新旧、新 PIDL 映射；收到目录级或关联级事件时重新枚举受影响范围并发布新的 snapshot generation。

当前原型中的 `shell.changed` 监控 Explorer HWND 拓扑，服务于宿主恢复。桌面项目同步需要独立的 `SHChangeNotifyRegister` 管道。

## 7. WebView2 宿主、透明和输入

[WebView2 宿主模式](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/windowed-vs-visual-hosting) 官方定义三种选择：

- Windowed hosting：WebView2 内容直接位于 HWND，操作系统负责空间输入、焦点、Tab 和可访问性，是常规应用的起点。
- Window-to-Visual hosting：输入模型接近 windowed，输出进入 HWND 承载的 `IDCompositionVisual`，重点改善混合 DPI 和输入队列问题。
- Visual hosting：应用把 WebView2 visual 接入 DirectComposition 或 Windows.UI.Composition，并自行转发鼠标、触摸、光标、焦点和可访问性相关行为，提供最高合成控制力。

当前 Tauri 原型使用 Windowed hosting。Window-to-Visual 可在 WebView2 初始化前选择对应 hosting mode，其开发模型继续使用 windowed controller，并保留相同的 Controller 管理契约。

[`CreateCoreWebView2Controller`](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2environment#createcorewebview2controller) 接收显示和输入使用的 parent HWND，并创建子窗口。直接父 HWND 改变时，宿主必须调用 `put_ParentWindow`。[`ICoreWebView2Controller`](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2controller) 还要求宿主管理：

- `Bounds`：使用 parent HWND 坐标空间；
- `IsVisible`：只控制 WebView2，外层 HWND 仍由应用显示或隐藏；
- `ParentWindow`：controller 的直接宿主 HWND；
- `NotifyParentWindowPositionChanged`：直接 parent 或任一 ancestor HWND 移动时发送通知。

当前原型保持 WebView2 的直接 parent 为 Tauri 外层 HWND，随后改变外层 HWND 的 Explorer parent、样式和位置。此过程应调用 `NotifyParentWindowPositionChanged` 并记录 controller 的 `ParentWindow/Bounds/IsVisible`。这些检查可以定位控制器状态，它们只形成诊断门槛；最终通过条件仍为每台显示器的真实像素和输入。

[`ICoreWebView2Controller2::DefaultBackgroundColor`](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2controller2) 支持 alpha `0` 或 `255`。透明值让 WebView 渲染宿主应用内容作为背景。官方同时记录 API 设置背景色前可能出现白色闪烁，进程启动前设置 `WEBVIEW2_DEFAULT_BACKGROUND_COLOR=00000000` 可以覆盖该间隙。透明 WebView2 表面仍依赖宿主 HWND 或 visual tree 的合成结果。

Visual hosting 使用 [`ICoreWebView2CompositionController`](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2compositioncontroller) 设置 `RootVisualTarget`、提交 DirectComposition device，并通过 `SendMouseInput/SendPointerInput` 传入空间输入。该路径适合独立原生适配器的第二级实验，因为它把 WebView2 输出、输入和合成状态全部交给宿主控制。

## 8. Tauri 创建期实验与独立适配器

Tauri 2.11.5 的 [`WindowBuilder::parent_raw`](https://docs.rs/tauri/2.11.5/tauri/window/struct.WindowBuilder.html#method.parent_raw) 在窗口创建阶段设置 parent HWND；生成的窗口带 `WS_CHILD` 并受 parent 客户区裁剪。`WindowBuilder` 的公开可见性受 `unstable` feature 控制。[`Window::add_child`](https://docs.rs/tauri/2.11.5/tauri/window/struct.Window.html#method.add_child) 同样受 `unstable` feature 控制。

当前代码在创建普通 Tauri 外层窗口和子 WebView2 后调用 `attach_host`，改写 `WS_POPUP/WS_CHILD` 并执行跨进程 `SetParent`。[`SetParent`](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setparent) 要求调用方同步窗口样式与 UI state；跨进程 DPI awareness 组合还会触发 child 进程 DPI 重置。创建期 `parent_raw` 可以移除这一后置重挂载变量。

建议按以下顺序推进宿主验证：

1. **Windowed 应用自有 HWND 基线**：同一 React 页面在普通透明自有 HWND 上通过双显示器像素、透明、工作区、输入和 OLE 验证。
2. **Window-to-Visual 应用自有 HWND 基线**：初始化前设置官方 hosting mode 环境变量，以相同页面核对 DPI、输入队列、透明和像素结果。两组基线用于选定创建期实验的 WebView2 hosting mode。
3. **一次 Tauri 创建期实验**：发现最终 render host 后，使用 `WindowBuilder::parent_raw(render_host)` 创建外层窗口，再调用 `Window::add_child`；删除创建后的样式改写和 `SetParent`。本阶段只验证宿主，不扩展 Shell broker 功能。
4. **独立 Win32/WebView2 windowed 适配器**：创建期实验失败后，由 Rust 原生模块直接创建最终 child HWND 和 controller，持有 Bounds、visibility、position notification、WndProc、STA/OLE 消息循环和恢复状态。Tauri 保留设置窗口、控制面与应用级 IPC。
5. **独立 Visual hosting 实验**：windowed 适配器仍出现透明合成问题时，使用 `ICoreWebView2CompositionController` 建立 DirectComposition tree，并实现输入转发。
6. **当前平台接管判定**：Windows 10 当前环境验证桌面层像素、输入、Win+D、Explorer 重启与恢复；其他 Windows 版本不再作为当前实现阻塞。

`parent_raw` 验证创建时机与 Tauri 管理路径。它接收的目标宿主来自 Explorer 私有桌面层能力，集成契约等级继续属于私有兼容。独立原生适配器可以提升可观测性和 controller 控制力，也继续承担 Explorer 私有宿主兼容风险。

## 9. Explorer 私有兼容层边界

Microsoft [`GetShellWindow`](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getshellwindow) 公开 Shell desktop window HWND。[`FindWindowExW`](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-findwindowexw) 公开按窗口类名搜索 direct child 的通用能力。这两个契约的定义范围止于 Shell desktop HWND 和通用窗口搜索；`Progman -> WorkerW -> SHELLDLL_DefView -> SysListView32` 的层级、兄弟顺序与第三方渲染宿主语义来自 Explorer 私有实现。

`0x052C` 位于 `WM_USER + 300`。[`WM_USER`](https://learn.microsoft.com/en-us/windows/win32/winmsg/wm-user) 把 `WM_USER` 到 `0x7FFF` 定义为 private window class 消息，并要求跨应用交换消息的双方预先共享其含义。`Progman + 0x052C` 的“创建 WorkerW”含义来自 Explorer 私有实现。

因此 `WorkerW` 发现、`0x052C`、隐藏 `SHELLDLL_DefView` 和 Explorer 重启后的重新附着继续集中在 `ExplorerDesktopHostAdapter`。适配器对外只暴露 `prepare`、`commit_takeover`、`restore`、`observe_shell_restart` 和 host diagnostics；Explorer HWND 的所有权完全收敛到该适配器。

## 10. 需求中的产品自定义语义

以下行为由公开 Shell API 提供数据和操作能力，最终交互规则由 Desktop Tools 锁定：

- 单击、`Ctrl` 多选、框选、焦点 anchor、方向键空间导航与跨围栏导航；
- 双击阈值、项目图标与标题的联合命中区域；
- 平铺模式的网格密度、换行、省略和自动排列；
- 列表模式的系统图像、名称、类型、修改时间与排序；
- 围栏调整尺寸后的 reflow、内部滚动条和滚动位置恢复；
- 围栏移动、折叠、删除和跨显示器迁移；
- 围栏内拖动排序、围栏间逻辑归属变更、未归类区域落点；
- 文件系统 copy/move/link 与逻辑成员变更的 drop effect 选择；
- Shell operation 正在执行、取消、部分成功、冲突和 elevation 的前端状态。

系统图像由 Shell broker 提供。文件夹、快捷方式和虚拟项目使用系统图标；图片、视频和文档通过 `IShellItemImageFactory` 请求缩略图，缩略图就绪前使用系统图标。缩略图复用 Windows Shell 缓存并进入进程内 LRU，Shell 图像和属性变化通知使对应缓存项失效。

这些规则应进入领域文档和验收测试。Rust broker 发布不可变 Shell snapshot 与 operation result；React 发布 selection、layout intent 和 hit map；持久化层只提交已经由 Shell 确认成功的项目变更。

## 11. 当前原型对齐结果

| 当前实现 | 对齐结果 | 下一原型改动 |
| --- | --- | --- |
| 用户桌面目录 + 公共桌面目录 + 5 个固定 CLSID | 覆盖样例项目 | 改为桌面 `IShellFolder::EnumObjects` |
| `path:<lowercase>` / 固定 CLSID 作为 ID | 适合样例运行 | 引入 opaque PIDL、解析名和 rename 协调 |
| `SHGetFileInfoW(SHGFI_PIDL)` | 符合公开 Shell API | 保留并增加异步 image factory |
| `ShellExecuteW` 默认打开 | 可覆盖常规打开 | 统一到 Shell item/verb 结果模型 |
| `IContextMenu` + `TrackPopupMenu` | 覆盖基础菜单 | 增加 `IContextMenu3/2` 消息转发和多选 |
| Explorer HWND 层级变化监控 | 服务宿主恢复 | 新增独立 `SHChangeNotifyRegister` 项目同步 |
| React 内部拖动 | 覆盖产品布局交互 | 增加原生 OLE drag source/target |
| 创建后改样式并 `SetParent` | Windows 10 视觉失败 | 执行一次创建期 `parent_raw` 实验 |
| `WorkerW/0x052C/DefView` 直接散布在平台代码 | 私有兼容路径 | 收敛到单一宿主适配器 |

## 12. 当前平台自动桌面接管门槛

接管提交必须等待以下条件全部成立：

1. 每台显示器的宿主 HWND、WebView2 controller 和前端 snapshot 就绪。
2. 每台显示器通过真实截图或像素探针确认 Desktop Tools 产生可见像素。
3. 每台显示器的透明区域继续显示壁纸，工作区边界停在任务栏上方。
4. 首份 pointer、keyboard、native menu 和 OLE drop target 语义命中图已发布。
5. guardian 已保存 Explorer 原始图标状态并完成独立进程握手。
6. 以上条件成立后自动隐藏 Explorer 图标并提交 `takeover.committed`，不等待人工输入且不设置启动自动退出。

当前 Windows 10 22H2 平台执行强制验收。矩阵覆盖启动闪烁、双显示器与混合 DPI、任务栏工作区、Win+D、Explorer 重启、锁屏恢复、正常退出、`Ctrl+C`、主进程崩溃和 guardian 恢复。日志属于诊断证据；实际像素、输入和恢复结果构成验收证据。

## 13. Windows 虚拟桌面契约

Microsoft 公开 [`IVirtualDesktopManager`](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nn-shobjidl_core-ivirtualdesktopmanager) 提供三项顶层窗口能力：判断窗口是否位于当前虚拟桌面、读取窗口所属桌面 GUID，以及把窗口移动到已知桌面 GUID。该公开契约覆盖窗口归属查询与迁移。

Desktop Tools 的每虚拟桌面独立布局还需要枚举虚拟桌面、读取当前桌面身份、监听创建、删除和切换，以及验证身份跨 Explorer 生命周期的稳定性。这些能力由版本相关的私有 COM 服务提供，并集中在独立 `VirtualDesktopAdapter` 中。当前 Windows 10 22H2 平台验证完整能力；其他 Windows 版本保留隔离兼容定义但不再阻塞当前实现。

冻结的 `tauri-desktop-host` 原型没有实现虚拟桌面枚举、创建、删除或生命周期通知，因此这部分不能声称来自原型运行验证。正式兼容层通过 `ImmersiveShell` 的 `IServiceProvider` 查询 `VirtualDesktopManagerInternal` 服务，并按目标系统隔离私有 ABI：Windows 10 22H2 使用对应的 Manager IID、Desktop IID 与创建、删除、查找 vtable；Windows 11 24H2+/25H2 使用 Microsoft TypeAgent [`WindowsVirtualDesktopService`](https://github.com/microsoft/TypeAgent/blob/main/dotnet/autoShell/Services/WindowsVirtualDesktopService.cs) 当前采用的 Manager IID、Desktop IID 与包含 `SwitchDesktopAndMoveForegroundView` 的 vtable。Desktop Tools 将服务 CLSID、接口 IID 和 vtable 定义限制在 `platform/windows/virtual_desktop.rs`，上层只接收稳定身份、环境快照和生命周期事实。

每个虚拟桌面呈现同一完整 Shell 桌面项目集合，并分别保存围栏归属、位置、组件和滚动状态。虚拟桌面划分 Desktop Tools 布局状态，Windows Shell 桌面命名空间继续作为共享项目来源。

## 14. 官方组件原型与正式实现边界

冻结原型在 `ui/index.html` 中直接声明“本地时间”静态组件，在 `ui/app.js` 中通过 `renderClock`、`makeDraggable` 和刷新按钮完成时间呈现、拖动、刷新及命中图更新，视觉规则位于 `ui/styles.css`。原型没有组件清单、能力授权、实例注册、版本化状态或工作区持久化，因此这些能力不能声称经过原型验证。

正式实现保留原型已经验证的透明表面、组件命中区域、标题拖动和刷新路径，并由 Component Runtime 增加官方清单校验、宿主能力授权、实例隔离失败状态、宿主事件、逐虚拟桌面组件快照以及跨显示器临时投射。公开 Component SDK、第三方分发、市场和沙箱策略仍属于后续里程碑，不进入 Milestone 1。

## 15. 生命周期恢复的原型与正式实现边界

冻结原型通过 `reload_renderer` 对当前 WebView 执行 `window.location.reload()`，因此验证的是“保留原生宿主和命中图的人工轻量重载”，没有自动 Renderer 失效检测。原型的 Shell monitor 会在 Explorer 宿主或显示器变化后重建 Windowed WebView2，并重新等待首帧、图像、输入和命中图门槛；guardian 在主进程异常退出后重新发现 Explorer，但只恢复启动时记录的项目可见状态。

正式实现沿用相同的轻量重载和整组重建路径，并增加逐屏 Renderer 心跳、重载重试、连续适配器失败恢复、恢复后窗口状态检查，以及通过 `SHGetSettings(SSF_HIDEICONS)` 持续观察用户“显示桌面图标”设置。正常退出、主进程异常和 `Ctrl+C` 最终均恢复最近观察到的用户设置；正常启动不再设置自动退出，当前平台的实际信号、崩溃、Explorer 重启和 WebView2 进程失效结果继续以实机行为为准。

## 16. 当前项目交叉引用

- Milestone 1 验收边界：ADR 0005。
- 自绘桌面项目接管：ADR 0006；ADR 0010 保留为被取代的历史记录。
- Explorer 私有兼容层和宿主验证层级：ADR 0011。
- Shell STA 运行时：ADR 0012。
- 桌面项目引用：ADR 0013。
- 布局与 Shell 撤销边界：ADR 0014。
- Windows 虚拟桌面兼容层：ADR 0015。

## 官方资料

- [SHGetDesktopFolder](https://learn.microsoft.com/en-us/windows/win32/api/shlobj_core/nf-shlobj_core-shgetdesktopfolder)
- [IShellFolder::EnumObjects](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-ishellfolder-enumobjects)
- [Introduction to the Shell Namespace](https://learn.microsoft.com/en-us/windows/win32/shell/namespace-intro)
- [ITEMIDLIST](https://learn.microsoft.com/en-us/windows/win32/api/shtypes/ns-shtypes-itemidlist)
- [SIGDN](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/ne-shobjidl_core-sigdn)
- [SHGetFileInfoW](https://learn.microsoft.com/en-us/windows/win32/api/shellapi/nf-shellapi-shgetfileinfow)
- [IShellItemImageFactory::GetImage](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-ishellitemimagefactory-getimage)
- [IFileOperation](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nn-shobjidl_core-ifileoperation)
- [IFileOperation::SetOperationFlags](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-ifileoperation-setoperationflags)
- [IFileOperationProgressSink](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nn-shobjidl_core-ifileoperationprogresssink)
- [IShellFolder::GetUIObjectOf](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-ishellfolder-getuiobjectof)
- [IContextMenu](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nn-shobjidl_core-icontextmenu)
- [IContextMenu2](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nn-shobjidl_core-icontextmenu2)
- [IContextMenu3](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nn-shobjidl_core-icontextmenu3)
- [Shell Data Object](https://learn.microsoft.com/en-us/windows/win32/shell/dataobject)
- [IDataObject](https://learn.microsoft.com/en-us/windows/win32/api/objidl/nn-objidl-idataobject)
- [IDropSource](https://learn.microsoft.com/en-us/windows/win32/api/oleidl/nn-oleidl-idropsource)
- [IDropTarget](https://learn.microsoft.com/en-us/windows/win32/api/oleidl/nn-oleidl-idroptarget)
- [DoDragDrop](https://learn.microsoft.com/en-us/windows/win32/api/ole2/nf-ole2-dodragdrop)
- [RegisterDragDrop](https://learn.microsoft.com/en-us/windows/win32/api/ole2/nf-ole2-registerdragdrop)
- [SHChangeNotifyRegister](https://learn.microsoft.com/en-us/windows/win32/api/shlobj_core/nf-shlobj_core-shchangenotifyregister)
- [IVirtualDesktopManager](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nn-shobjidl_core-ivirtualdesktopmanager)
- [Windowed vs. Visual hosting of WebView2](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/windowed-vs-visual-hosting)
- [ICoreWebView2Controller](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2controller)
- [ICoreWebView2Controller2](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2controller2)
- [ICoreWebView2CompositionController](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2compositioncontroller)
- [SetParent](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setparent)
- [WM_USER](https://learn.microsoft.com/en-us/windows/win32/winmsg/wm-user)
- [Tauri 2.11.5 WindowBuilder::parent_raw](https://docs.rs/tauri/2.11.5/tauri/window/struct.WindowBuilder.html#method.parent_raw)
- [Tauri 2.11.5 Window::add_child](https://docs.rs/tauri/2.11.5/tauri/window/struct.Window.html#method.add_child)
