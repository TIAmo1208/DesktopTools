# Rust/Tauri 目录结构

`src-tauri/` 负责 Tauri 应用装配、Windows 桌面宿主、Shell 操作、工作区领域规则、持久化和前端命令边界。Rust 后端是文件事实、布局事务、原生拖放和 Windows API 结果的权威执行层。

## 目录职责

```text
src-tauri/
├─ src/
│  ├─ main.rs             # 进程入口
│  ├─ lib.rs              # Tauri 应用装配和命令注册
│  ├─ app/                # 生命周期、运行时状态、偏好写入口与桌面接管
│  ├─ commands/           # 前端命令边界、按领域路由、参数校验、loopback 通道与用例调度
│  ├─ workspace/          # 布局、围栏、门户、组件和事务规则
│  ├─ shell/              # 桌面项目事实、Shell 身份、菜单、剪贴板和拖放
│  ├─ collect/            # 文本、图片、文件映射、资源、收入取出与删除恢复
│  ├─ host/               # 透明宿主、WebView2、DPI、命中图和接管
│  ├─ render/             # 原生渲染核心：绘制指令、DIP/像素几何、Direct2D 绘制、DirectComposition 宿主
│  ├─ persistence/        # 布局快照和备份
│  ├─ command_bridge.rs    # 页面命令处理器类型（平台与命令边界的接缝）
│  ├─ drop_bridge.rs       # 拖放用例端口与投放输入（平台与布局事务的接缝）
│  ├─ preferences.rs      # 偏好存储、校验、读写端口与配置/插件目录解析
│  ├─ weather/            # 天气凭据、按来源分派的请求与临时验证会话
│  ├─ component_runtime/  # 组件清单、能力、类型级尺寸与运行时状态
│  ├─ recovery/           # guardian、健康状态和异常恢复
│  ├─ platform/windows/   # Windows API、COM、OLE 和原生适配
│  ├─ contracts/          # Rust DTO、事件和结构化错误
│  └─ bin/guardian.rs     # 独立恢复进程入口
├─ capabilities/          # Tauri 权限声明
├─ components/            # Tauri 组件配置
├─ gen/                   # Tauri 生成文件
├─ icons/                 # 应用图标资源
├─ installer/             # NSIS 重装页模板与安装、卸载生命周期钩子
├─ Cargo.toml             # Rust 依赖和构建配置
├─ tauri.conf.json        # Tauri 应用配置
└─ tauri.installer.conf.json # 双安装模式 NSIS 测试安装包覆盖配置（随包写入安装目录 plugin）
```

## 关键边界

- `render/` 是原生渲染路线的内核，与 WebView 宿主并存、互不依赖：`draw.rs` 只定义绘制指令（不含主题与业务语义），`geometry.rs` 是可单测的纯逻辑（DIP ↔ 物理像素、命中判定、工作区钳制），`painter.rs` 把指令落到 Direct2D/DirectWrite，`host.rs` 为每块显示器建一块透明 `WS_POPUP` 宿主（`WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_NOREDIRECTIONBITMAP`）并在 `IDCompositionSurface` 上绘制。绘制目标必须与合成表面同源：由同一 DXGI 设备建出 `ID2D1Device`，再 `CreateBitmapFromDxgiSurface`。`desktop-tools.exe --native-surface-demo` 显示 5 秒绘制样例；`--native-surface-workspace` 只读已保存的工作区，逐屏绘制围栏、空格子外壳和桌面项目占位，约 8 秒后退出。不运行插件或读取缩略图，不接管 Explorer；没有布局文件则明确报错。`--native-surface-content` 在独立的内存投影中显示时钟、展开收集箱并绘制总数、读取已绑定城市的天气缓存；不请求天气、不写偏好。只有单屏、三类目标同属唯一旧屏且局部矩形均完整适配时，诊断才以旧屏作为当前屏别名；不更改正式投影。`--native-content-overflow-example` 只显示合成的省略号样例，不读取真实布局和业务数据。`input.rs` 是命中与焦点的纯逻辑：`workspace_scene::scene_blocks` 是**唯一排序点**（稳定按 `(z, priority)` 升序，焦点块整体移到末尾、内容与边框一起上移），绘制与命中消费同一个块列表，优先级与前端命中图一致（`workspace-background` 1 / 围栏 50 / 门户 60 / 组件本体 60 / `desktop-item` 70）；命中范围是「绘制矩形 ∩ 视口 ∩ 围栏内容框」，背景可命中但不可聚焦，Tab 与方向键只在非背景候选间移动，焦点只存内存。`--native-surface-input` 用脚本驱动这套逻辑，逐步输出光标、命中对象、区域与焦点迁移；`DESKTOP_NATIVE_INPUT_HOLD_MS` 调整每步保持时长。`menu.rs` 是桌面层右键菜单的原生弹层（`WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW`，置顶、不抢焦点），度量取视觉规范（宽 232、圆角 6、内距 4、行高 28、图标列 16/24、正文 13），键盘走定向消息、鼠标悬停与点击、Esc/失活/菜单外点击关闭；`--native-surface-menu` 在自包含场景里演示打开、键盘选择、鼠标触发与四种关闭路径。可设置 `DESKTOP_NATIVE_SURFACE_REPORT` 指定诊断 JSON 输出路径。不加参数仍启动原有正式应用。
- `--native-surface-input-live` 打开可激活的小窗口：居中，最多 900×560 DIP，裁剪显示主屏持久布局右上部分，不跨屏借用、不隐藏 Explorer。Windows 窗口过程接收输入后，沿用同一组命中和焦点函数；只在本窗口前台且有焦点时处理键鼠。Esc 或 60 秒超时退出，失焦清空临时焦点，DPI/显示环境变化则停止而不按旧比例继续命中。手工使用不投递任何探针消息。追加 `--verify-window-messages` 先不激活地显示并待命，300 秒内未激活就退出且零投递。只有演示窗口已是前台焦点且收到开始区点击或 F8，才定向 PostMessage 到本窗口；该测试验证消息链路，不是物理键鼠证据。
- `--native-surface-input-fixture` 只在内存构建测试围栏、时钟壳与门户，复用上述窗口消息与生产命中/焦点路径，等待 300 秒内的显式开始信号。此模式不打开持久布局、偏好或配置定位文件；不能作为用户实际布局的呈现证据。
- `desktop-tools.exe --native-desktop` 是显式的常驻**保护态**：与正式桌面共用单实例与 Explorer 层级附着；guardian 使用专有 `monitor-only` 握手，只等主进程退出或停止事件，绝不改 Explorer 图标可见性。不支持此协议的旧 guardian 会使启动失败，不降级到恢复模式；每屏 HWND 在隐藏状态下发布真实布局首帧。不启动 WebView 桌面、设置、浮岛或其他产品表面。没有 Shell 图像与交互命中时绝不隐藏 Explorer 项目，也不显示不可用外壳；不能作为完整桌面的替代。此进程仅接收 `desktop-tools.exe --desktop-menu exit` 正常退出，不注册产品菜单。检测到显示环境或附着父窗失效时安全停止，不热切换默认配置。

- `component_runtime/mod.rs` 读 `manifest.ini` 的 `[plugin]`、`[tiles]` 与 `[tiles.size]` 三节：`[tiles]` 每行 `键 = 显示名`，按序保留、第一节为默认类型，空节、非法键、重复键都让整份清单进 `failures`；`[tiles.size]` 每行 `类型键 = 默认宽x高` 或 `类型键 = 默认宽x高, 最小宽x高`，只对已声明类型生效，未知键、非法尺寸与重复行记 WARN 后忽略，下发为 `tileTypes[].defaultSize` 与 `tileTypes[].minimumSize`。`contracts/component.rs` 的 `tile_type_bucket` 是计数归属的唯一口径：视图按实例 `state.tileType` 分桶，缺失时记到第一节，没有 `[tiles]` 的组件共用一个桶。`workspace/mod.rs` 在创建命令上做数量校验与类型级下限校验，`component_runtime::plugin_tile_limit_reached` 按同一口径逐类计数，`maxTiles` 因此是每类各自的上限；`create_plugin_component` 先校验类型是否在清单里，再把类型写进新实例的 `state`，并按该类型的默认尺寸（未声明时用公共默认尺寸、不低于有效下限）算创建边界。
- `commands/weather.rs` 把凭据草稿类命令（临时城市搜索、验证城市、把验证通过的城市写入来源列表、保存与清除）保留给设置窗口，桌面 transport 另提供已保存凭据下的来源列表、某来源城市列表读写、按来源城市搜索与天气读取。`weather/source.rs` 定义来源标识与顺序；`weather/credentials.rs` 按来源各保存一条 Windows Credential Manager 记录（和风沿用既有应用级记录名，其他来源在其后追加来源标识）；`weather/mod.rs` 管理草稿、并发互斥、取消、保存门槛、来源列表与运行期可用状态、存活探测与按来源删除，运行期状态按来源索引：凭据记录名与 `weather-cities-<来源>.json` 城市列表文件按来源派生，配置代次、取消标记与退避记录都以来源为键，磁盘预报缓存仍按配置指纹与城市标识保存；`weather/provider.rs` 与 `weather/foreca.rs` 分别校验各自凭据、签发 JWT 或使用 Bearer 令牌、按来源校验城市与天气数据。网络任务只在用户操作时创建，禁止重定向，限制超时与响应大小；城市搜索结果和天气验证响应不落盘。天气格子在拖动缩放期间按实时尺寸切换布局，详情入口由前端公共标题栏提供；`weather/activity.rs` 负责展开实例的共享活动调度，并通过 `weather-forecast-changed` 通知预报更新。天气格子与收集箱一样由组件清单声明可创建、可移除，创建入口写入绑定标识并在同一链路开启天气功能开关。

- `preferences.rs` 维护配置目录与插件目录。配置目录仍用启动定位目录里的 `config-location.json`。插件目录不迁移：`pluginDirectory` 为空时用可执行文件所在目录下的 `plugin`（安装后即安装目录 `\plugin`），显式路径仍按原有安全边界覆盖；取不到程序目录时报错，不回退到 `%APPDATA%`。运行期内目录冻结，改偏好下次启动再生效，不复制旧目录。插件安装按 `作者@名称` 落盘并原样保留清单标识，同一作者同一标识视为同一插件。
- `installer/` 包含 NSIS 自定义重装页与生命周期钩子。`tauri.installer.conf.json` 把随包插件资源直接写到 `$INSTDIR\plugin`，`hooks.nsh` 不再在安装后复制插件，也不在卸载时按清单清理。真正卸载且非升级时删除 `$INSTDIR\plugin`。

- `commands/` 负责跨边界校验与调度，不持有长期领域状态。`tools/probe.rs` 是插件能力 `probe-service-status` 的唯一探活实现：探测地址与 2 秒超时在宿主内固定，不进入命令入参或插件设置；它新开独立的 HTTP 客户端，不启用 cookie jar，也不带浏览器来源头，把观察结果判成在线、探测被拒、未运行或未知四档，只回传判定时刻与该服务的启动时间，不透传服务端返回原文。该文件不注册成平铺原生命令，只由能力通道按实例授权后调用。
- `commands/loopback.rs` 与 `component_runtime/loopback.rs` 是插件能力 `loopback-channel` 的实现：`register_loopback_channel`、`read_loopback_channel`、`request_loopback` 三条命令在 Tauri 与本地 transport 共用同一门禁，按 `instanceId` 校验能力授权。监听只绑本机 IPv4 回环地址，`baseUrl` 只接受 `http://127.0.0.1:<port>` 或 `http://localhost:<port>` 并归一为本机 IPv4；每条通道保留有界的原始请求缓冲（条数、单条上限与保留时长见 ADR-0038），读到即消费；凭据用 `CredReadW` 读取并清零系统缓冲区，出站自动注入、入站用常量时间比较校验，校验失败回 `401` 且不入缓冲，已校验的凭据头在交给插件前移除。缓冲更新复用 `component-host-event`（`kind = loopback-data:<channelId>`），首次读取后才投递；逐屏转发时通道数据通知不依赖时间消费者。实例移除与插件卸载都会释放端口并清缓冲。宿主不解析也缓存任何业务数据，过期判定归插件。
- `workspace/` 负责自有布局、围栏归属、门户和组件布局事务，不执行文件操作；偏好写入经 `PreferencesStore` 端口由应用层提交，不直接持有设置状态。各类格子的持久展开边界统一使用工作区契约中的公共尺寸底线，临时投射单独校验；内容可读性和标题占位由前端负责。
- `shell/` 负责 Shell 项目身份、真实文件操作、菜单、剪贴板和 Shell OLE 数据对象。
- `shell/images.rs` 持有独立图像 STA、去重队列和有界 LRU；`platform/windows/shell_image.rs` 在所属线程提取 Shell 缩略图、转换 PNG 及原生拖动位图。`get_desktop_item_image` 校验当前项目身份后请求图像，Shell 变化使缓存和在途结果失效，原生拖动只读取已完成缓存。
- `host/` 负责逐显示器透明窗口、WebView2、坐标、命中图和桌面接管。桌面宿主只允许初始占位页与当前页面导航，鼠标侧键的前进/后退被拒绝，桌面层不会因导航被清空。
- `platform/windows/` 集中维护 COM、OLE、DPI、窗口消息和 Windows API 适配；页面命令只接收组合根注入的处理器，原生拖放只持有应用层注入的端口，不直接调用应用用例。`island.rs` 持有独立浮岛 HWND 与 WebView，复用桌面协议的页面/插件资产处理器、原生桥和事件转发，但命令只接受浮岛自身来源；展开态窗口盒覆盖主屏工作区；页面提交的 `cards` 以窄列窗口盒为原点，原生在投放与几何判定前按工作区原点重基准，前端不需要感知父窗放大。`island_drop.rs` 在浮岛根窗口与 WebView2 后代注册 Shell 投放目标，只在岛卡顺序含收集箱时接收收集内容；`commands/island.rs` 按偏好返回浮岛实例与注册投影，不写工作区注册集合或布局；收集项拖出结束后由 `island_cursor_inside` 按同一物理客户区规则回答光标是否仍在岛内。浮岛消息投影与关闭命令在 `commands/notifications.rs`，事件 `island-messages-changed` 由原生消息队列在集合变化时广播，浮岛不另建队列。`integrity.rs` 在启动时读取本进程完整性级别：低于中完整性时显示原生错误对话框并以退出码 1 结束，不进入 Tauri 初始化，避免低完整性进程附着 Explorer 失败后以 setup panic 结束。
- `platform/windows/popup_layer.rs` 维护唯一的置顶弹层窗口（透明、无边框、不进任务栏、`always_on_top`）：宿主提交命令表与锚点，页面先量后显并上报尺寸，Rust 换算屏幕物理像素、整体推回工作区后显示；失活按取消收束并还原前台，关闭后空闲 90 秒销毁以释放 WebView2。命令入口 `open_popup_layer` / `dismiss_popup_layer` / `update_popup_layer_payload`，页面入口 `get_popup_layer_request` / `set_popup_layer_frame` / `finish_popup_layer`，结果事件 `popup-layer-result`。决策见 ADR-0040。
- `platform/windows/drop_content.rs` 探测有界源格式、声明业务白名单、协商效果并提取自有数据；`desktop_host.rs` 在既有 OLE 入口按最高命中目标分流。`app/desktop_drop.rs` 复核实例、功能开关与运行时注册，再交给 `collect/drop.rs` 或 Shell STA；浮岛投放复用同一收集收入入口，但只校验岛卡顺序，不依赖工作区命中区域；后台不持有原生拖放数据对象。
- `collect/drop.rs` 负责一次拖入的格式去重、Markdown 追加及资源收入，正文保存使用既有 Windows 原子替换能力，失败保留已保存正文与可追踪的资源路径。
- `collect/items.rs` 管理根目录独立图片文件与文件映射记录；`collect/transfer.rs` 管理复制导出及内部逐项复制、追加，复用 `references.rs` 改写复制资源的 Markdown 引用。独立图片复制到 `collect` 根目录并使用文件名身份，扫描不递归文档资源目录，映射不复制源文件。编辑窗状态由 `commands/collect.rs` 在两种命令 transport 和 OLE 业务入口通过同一窗口注册锁查询并保护写入，正文追加逐项拒绝编辑中的目标。
- `collect/mod.rs` 有界读取收集项引用的本地图片；`commands/collect.rs` 提供图片读取命令，并校验网页、邮件链接后复用 Windows 默认打开能力。
- `collect/cleanup.rs` 管理附件归属证据与关闭清理，`collect/references.rs` 负责 Markdown 引用解析和改名转换。
- 运行时不再保留 DSH 专属领域：DSH 接入由 `plugin/` 里的插件承载，连接、配对、轮询与提醒都在插件内，见 ADR-0051。插件可声明多个已知能力（`required_capabilities` 去重后非空）；浮岛合成实例 `island.<pluginId>` 与工作区实例共用同一能力校验，也允许注册 loopback 通道。`commands/plugin.rs` 提供 `plugin_credential_set` / `plugin_credential_clear`（只写不读，命名空间 `DesktopTools/plugin/<pluginId>/<slot>`）、`plugin_credential_exchange`（宿主向 `http://127.0.0.1:<port>/<path>` 取回秘密并直接落槽，只回 `{ stored: true }`，禁代理/禁重定向、5 秒超时、响应上限 64 KiB）、`publish_island_message`（真事件 `island-plugin-message`，载荷 `{ source, key, kind, text, unixMillis }`，正文去控制字符、空与超长拒绝）与 `open_uri`（仅 `dsh:` 与 `https:`），五条命令经 `route_plugin` 进入逐屏注入桥。凭据目标按 Windows 不区分大小写的单点分类：只允许本插件命名空间或兼容的 `DesktopTools/astrbot-status`；`credentialPrefix` 只允许 `Bearer `。岛实例的可见与保留集合按「岛开关 + 卡片顺序 + 插件未停用」计算，偏好保存后立即裁剪通道。旧的原生 DSH 命令、事件、`official.dsh` 清单与 `DesktopTools/dsh-bridge` 保留凭据判断均已删除。
- `contracts/` 与前端 `src/contracts/` 对齐跨边界数据形状。

## 数据方向

```text
React feature -> bridge -> commands
  -> workspace / shell / persistence / component_runtime
  -> host / platform/windows
  -> Windows Shell / WebView2 / 文件系统
  -> contracts + events -> React feature
```

修改 Rust 或 Cargo 配置前，按根目录 `AGENTS.md` 停止开发监听；Cargo 检查、测试和构建必须串行并沿用项目指定工具链与共享构建目录。

`npm run tauri:installer` 先由 `scripts/prepare-licenses.mjs` 整理当前锁定依赖许可，再加载安装包覆盖配置构建与打包。普通开发和纯程序构建不依赖生成的许可文件。安装钩子复用应用桌面菜单命令，不持有业务状态；生命周期与分发边界见 ADR-0024。`installer/installer.nsi` 取自 tauri-bundler 的 NSIS 模板，只改动检测到已安装版本时的选项，升级 `@tauri-apps/cli` 后须重新对比上游模板并回放该修改。
