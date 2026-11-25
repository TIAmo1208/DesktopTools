# 前端目录结构

`src/` 负责 React 页面、交互状态、展示模型和类型化 bridge。领域规则、文件事实和 Windows API 由 Rust/Tauri 后端持有。

## 当前目录

```text
src/
├─ app/                     # 页面入口、根布局与窗口装配
│  └─ preferences/          # 全局偏好 Provider 与外观解析：每窗口一份，跨窗口只同步契约
├─ features/
│  ├─ workspace/            # 跨类型装配、全局选择、命中、OLE 和跨屏协调
│  │  ├─ surface/           # WorkspaceSurface、DesktopWorkspace 及宿主呈现
│  │  ├─ interaction/       # 指针意图、选择、框选和跨屏消息
│  │  └─ state/             # 权威快照交付与临时状态
│  ├─ tiles/                # 公共外框、菜单、视口、几何、展开和静态帧
│  │  └─ header/            # 公共标题骨架、摘要与 commands 工具区
│  ├─ tile-content/
│  │  ├─ fence/             # FenceTile、成员布局、排序、滚动与投放
│  │  ├─ folder-portal/     # 门户装配、目录导航、选择及真实文件操作
│  │  ├─ collect-box/       # 收集项、摘要、工具、独立编辑窗与回归测试
│  │  ├─ music/             # 桌面媒体控制、响应式布局、歌词区与最小尺寸
│  │  ├─ clock/             # 活动时间订阅与呈现
│  │  ├─ weather/           # 共享城市弹窗组件（设置窗内联，桌面格子放在置顶弹层里）
│  │  └─ debug/             # 宿主诊断与测试内容
│  ├─ music/                # 格子与浮岛共用的引用计数音乐源、锚点与歌词行定位
│  ├─ island/               # 独立三态页面、锚点内容层、音乐/收集箱/天气卡、消息卡、实测几何与偏好顺序拖动
│  ├─ popup-layer/          # 置顶弹层页面：菜单、确认框与天气弹窗，先量后显
│  ├─ components/           # 运行时注册、实例能力、开关与失败隔离装配
│  ├─ desktop-items/        # 围栏与未归类区域共享的 Shell 项目呈现
│  ├─ notifications/        # 统一消息入口
│  ├─ settings/             # 设置页面外壳与按子域划分的页面、流程和基元
│  │  ├─ appearance/        # 外观、主题、字体与主题插件运行时
│  │  ├─ features/          # 功能格子页
│  │  ├─ grid/              # 网格页
│  │  ├─ island/            # 浮岛页、卡片顺序与样张
│  │  ├─ plugins/           # 插件页、任务菜单与安装任务规则
│  │  ├─ ui/                # 设置基元：简介、开关、信息与音乐设置行
│  │  └─ weather/           # 天气页、城市列表、领域流程与操作闸
│  └─ system-menu/          # 桌面菜单入口
├─ bridge/                  # 原生命令、事件、结果解码与订阅生命周期
├─ contracts/               # 前端 DTO、快照、命令和事件类型
├─ shared/                  # 共享 UI 与工具
└─ styles/                  # 全局主题和窗口基础样式
```

## 入口与职责

`features/settings/plugins/PluginSettingsPage` 是设置窗口的「插件」页。一个插件是插件根下的一个文件夹，入口与版本信息来自文件夹内的 `manifest.ini`；清单未到时不画插件列表，没有插件时显示尚未安装。每个插件一行，含条目数、启停开关、卸载按键和进入箭头。停用写入偏好里的 `disabledPlugins`，不加载该插件入口模块；启用后按清单的 `script` 同源 `import()` 加载，取模块默认导出的 `{ renderTile?, settingsEntries? }`，由宿主按清单 id 绑定。点进插件后设置条目直接排在二级页内容区（不再用下拉展开），条目内容由作者导出的 `render` 画进容器，没有 `render` 时显示宿主兜底句；插件既没有工作区格子、浮岛卡也未启用时不渲染需要实例授权的条目，并在内容区顶部提示先添加格子或启用浮岛卡。右上角「添加到桌面」只在清单允许创建且存在可创建类型时渲染。行上可以卸载：确认后删除整个插件文件夹，保留该插件的设置在配置目录 `settings-plugins.json` 里的「作者@插件ID」键，并当场移除该插件的格子与布局；安装或卸载成功后清单即时刷新，不必重启。右键菜单可以打开插件目录、停用或启用、卸载。工作区里清单已经不存在的实例列在「失效插件」，确认后只移除该格子。停用的插件在桌面上整格隐藏，不显示任何占位或错误文案。清单写了 `[tiles]` 时，二级页加号在多于一种类型时变成下拉列表，右键的「添加到桌面」变成二级菜单；创建时把类型写进实例 `state` 的 `tileType`，`OfficialComponentHost` 读出来放进外壳上下文的 `tileType`，插件据此画不同内容。数量上限按类型分别计算，已满的类型在菜单里单独禁用，加号在全部类型都满时才禁用。清单另写 `[tiles.size]` 时按类型给出默认与最小尺寸，创建入口用该类型的默认尺寸建格子。插件页的状态、订阅与安装/卸载生命周期集中在 `usePluginSettings`，纯行/目录/提示 helper 在 `pluginSettingsModel`，组件只做布局与菜单拼装。

插件清单的 `iconPath` 由前端清单契约带回，`bridge/pluginAssets` 把相对路径拼成 `/plugins/<相对路径>`；浮岛锚点（20）与展开卡标题行（16）用 `<img>` 按官方原色渲染身份图标（`object-fit: contain`、透明背景），缺失或加载失败回落通用插件图标。插件外壳上下文由 `bridge/pluginScripts` 定义，桌面格子、浮岛卡与设置条目共用同一份；`bridge/pluginCapabilities` 按实例已授权能力裁剪出 `setCredential`、`clearCredential`、`exchangeCredential`、`publishIslandMessage`、`openUri` 与 loopback 三件（`registerLoopbackChannel`、`readLoopbackChannel`、`requestLoopback`），未声明能力时为 `undefined`；命令在 `bridge/components`、`bridge/loopback` 封装并在 `pluginCapabilityHost` 组合成生产上下文，桌面格子、浮岛卡与插件设置条目共用同一份。`exchangeCredential` 走 `plugin_credential_exchange`，宿主取回秘密后直接落槽，只返回 `{ stored: true }`。
插件卡在浮岛常态与 hover 都挂载到隐藏宿主（移出视口、不用 `display:none`），插件可持续轮询并调用 `setIslandSummary` 提供胶囊/hover 的一行摘要；hover 自绘只作只读视图、独立清理，不释放常驻 `renderTile`。未调用时回落清单名称。展开态复用同一挂载点，浮岛隐藏、锁屏与全屏隐藏时卸载，恢复后重新挂载一次。展开插件卡容器只保留 88 DIP 最小值、随插件根内容增高，宿主观察每个 `[data-plugin-container]`，插件内部分组切换会触发一次强制重测。`setIslandSummary(null)`（或空串）表示显式隐藏锚点；未调用才回落清单名称。摘要可带 `status` 与 `navigation: { uri }`：声明 uri 的插件锚点与消息条目可点击，宿主按岛实例调用 `open_uri`；前端与宿主都按 `dsh:`/`https:`、≤512 字符、无控制字符校验，失败在 hover 详情或展开卡显示 `role=status`（胶囊仅播报），不自动移除条目、不弹 Toast。hover 详情的摘要行与消息正文复用 `ScrollingText` 单行往返滚动，离开复位、减少动效不动。消息通知卡以 `official.island-messages` 进入同一份 `islandCardOrder`，默认插在天气之后（无天气时首个插件前、无插件末尾），由 `islandMessageCardInitialized` 保证只插一次，且只在保存成功后置标记；展开方向键排序对完整保存顺序交换相邻可见卡，消息槽与空槽不会被实例集合重建删掉，消息卡显示时自身也可移动；顺序满 32 项时不插、不挤掉既有卡。`islandMessageCardVisible` 控制展开态显示，与 bell 锚点独立。插件的岛内提醒走 `publish-island-message`：宿主事件 `island-plugin-message` 携带 `{ source, key, kind, text, unixMillis }`，`features/island/island.tsx` 把它并入既有本地消息条目，按 `plugin:<source>:<key>` 去重、按到达时间排序，暂停期间丢弃，不弹 Toast、不新建队列。

`features/settings/weather/WeatherSettings` 提供来源列表、添加与编辑弹窗、来源右键菜单与城市管理。来源列表显示已配置来源的地址、认证模式与可用状态，行尾展开按键在列表项边框内展开该来源的城市列表；`features/settings/weather/SettingsCityList` 是来源展开区与编辑弹窗“城市列表”分组共用的城市列表，包含城市项、行政区副标题、删除图标与末行“添加城市或地区”，删除共用同一确认。来源右键菜单提供显示详情、检测可用、添加城市、添加天气格子与删除，“添加天气格子”的二级菜单按该来源城市创建已绑定来源与城市的格子。凭据草稿输入、私钥导入与显隐、临时城市搜索与分项验证都集中在添加与编辑弹窗内。`bridge/weather.ts` 对接设置窗口专用凭据命令，以及桌面天气的来源列表、城市读写、按来源搜索与预报命令；敏感草稿不进入普通偏好、浏览器存储或事件广播。关闭窗口与分类导航共享最新草稿状态，取消后拒绝迟到结果；页面状态与流程集中在 `useWeatherSettings`，组件只做布局与事件绑定；`weatherSettings.browser.html` 使用生产组件和替代桥验证相关交互。

`features/weather/WeatherCityDialog` 是四处城市入口共用的城市弹窗（添加或编辑时的验证城市、来源右键菜单“添加城市”、来源展开区与编辑弹窗的“添加城市或地区”、天气格子菜单里的“添加新城市或地区”）；桌面格子里的那一处由 `popup-layer/PopupWeather` 在置顶弹层窗口内装配，搜索与验证在弹层里完成，确认结果回传宿主后由格子写入城市列表并绑定：标题、主命令文案与状态由调用方传入，搜索、验证与确认只走回调，组件自身不导入桥接层，草稿会话与按来源命令的差异留在调用方。`features/weather/WeatherSourcePicker` 是格子侧的“选择 API”轻量弹层，只列出设置中已配置的来源并把选中的来源交回调用方，由调用方清空格子的城市标识。

`WeatherTile` 沿用公共格子呈现预报，通过一次性计时处理当前天气的 24 小时展示上限，隐藏、折叠或宿主暂停时撤销计时、恢复活动时重新检查，不因此增加天气请求。格子按自身保存的来源与城市标识推导缺失状态：未配置来源、未选择来源、未选择城市、城市已从列表移除、来源凭据已删除各有对应文案与按键，判定不依赖城市对象。预报读取沿用公共格子实际展开状态与 `desktop-refresh-changed` 事件，暂停或折叠时拒绝旧响应，恢复时保留已有缓存并重新读取；展开的天气实例通过桥接监听预报更新事件并登记后端共享活动调度。

小时与日预报按条目结束时间过滤，并只为最近到期点设置一次性计时；隐藏、折叠和暂停停止计时，恢复后重新检查。当天标签与最高最低温按目标城市日期匹配，过期过滤不触发联网补齐。

`main.tsx` 装配应用并按需加载收集项编辑窗口。`WorkspaceSurface` 组合逐屏权威投影与原生命令；`DesktopWorkspace` 负责未归类项目、围栏、官方组件及附加门户的共同工作区，持有全局选择和 OLE 协调。`CollectBox` 呈现文本、图片和文件映射，维护当前格子的选择、清理确认及复制拖出生命周期；`collectItemState.ts` 处理混排／分离、筛选及实例状态，`collectMarkdown.ts` 提取文本摘要。

`collect-box/collectEditorImages.ts` 为 Vditor 图片补充本地显示数据、适配放大预览，并修正图片说明转义与目标地址编码，保持 Markdown 图片语义；`CollectBox` 与 `CollectEditor` 通过 `bridge/collect.ts` 把链接交给系统打开。

`CollectBox` 的图片槽使用可见性与尺寸观察器，通过 `readCollectImage` 请求缩略图或 GIF 悬停原图；`CollectEditor` 管理编辑、保存和关闭清理的调用生命周期。两者通过 `bridge/collect.ts` 访问后端，文件校验与回收归后端负责。

`DesktopTileBase` 内部统一调用几何与展开 Hook，组合标题显隐、菜单、八向缩放及输入生命周期。调用方只提供几何参数和类型化业务回调；标题从公共 context 读取拖动入口，视口与活动内容读取同一实际折叠状态。`tileGeometry` 保存共享算法和可选 `minimum` 的默认值解析，使用 `contracts/tile` 的公共数据底线；内容层将内容需求与外壳、标题占位换算为外框下限。`tileDragPreview` 保存有界静态视觉；跨屏消息和 OLE 投放仍归工作区。

`island/island.tsx` 从独立浮岛快照装配三态卡片，`island.css` 独立持有外观，不导入格子外壳；`islandGeometry.ts` 负责实测卡片到屏幕 DIP 提交、透明间隙命中、工作区内放置、顺序移动与展开态卡片外侧的滚动条槽；`islandContent.ts` 持有内容层纯模型（锚点身份与顺序、十态差异、叠加与排序、hover 选中、折叠与空态、超时与隐藏合并、拖放文案），不依赖 React、DOM 或定时器，渲染层只消费其结果。页面以不可见令牌探针读取主题现有 `.fence-workspace` 下的表面、描边和文字色，响应偏好、主题样式表与系统深浅色变化；浮岛根不继承工作区尺寸或定位。音乐卡消费 `music/musicSource` 的歌词与时间轴，只为可见位置投影计时；频谱只订阅已有采集，无采集保持基线，不启停桌面全局音频采集。`islandMessages.ts` 只决定消息在三态中的取舍与阅读期暂存，去重与过期仍归原生消息队列。`IslandCollectBox` 呈现最近收集项、支持单击打开与展开态拖出，加载态也占展开内容宽以避免二次调宽；`islandCollect.ts` 复用桌面排序与摘要规则；`IslandWeather` 从偏好读取来源与城市，复用共享预报事件，`islandWeatherModel.ts` 处理缺失态与当天摘要。展开态消息由 `IslandMessagePanel` 收进一张卡，缺省收起到最新一条；展开/收起走宿主统一收起记忆（`islandCardCollapsed`/`islandCardCollapseMemory`），关闭记忆后回到默认收起。浮岛收集箱的拖入由平台 `island_drop.rs` 注册到根窗口与 WebView2 后代，收入规则仍由 Rust 收集业务执行；拖出复用桌面收集箱的系统拖放准备与启动命令。

`components/PluginSlot` 持有桌面与浮岛共用的脚本加载、作者设置上下文、菜单注册、渲染清理和失败隔离，不导入格子样式；`OfficialComponentHost` 继续持有桌面格子外框。

`OfficialComponentHost` 同步运行时、过滤实例并分发内容；`plugin:slot` 在当前屏幕已有实例时按清单的 `script` 去重加载 `/plugins/<script>`，再把模块默认导出的 `renderTile` 挂进既有外框；外壳上下文带 `tileType`（取自实例 `state`，缺失时为空串）与 `menu`（插件右键菜单注册接口），插件据此区分同一插件的多种格子并注入自己的菜单项。`bridge/pluginMenu.ts` 按实例保存最近一轮注册的菜单项，实例重挂、卸载或重新渲染前清空，`pluginTile.ts` 负责把清单的类型级最小尺寸映射到公共外框。逐屏原生命令白名单包含 `get_plugin_settings`。插件能力统一走能力通道：清单声明 `required_capabilities` 后，插件格子在 `renderTile` 里以 `instanceId` 与能力名调用，`contracts/component.ts` 按能力区分载荷——`read-host-time` 返回 `unixMillis`，`probe-service-status` 返回 `{ level, startTime, checkedAt }`（`level` 只有在线、探测被拒、未运行、未知四档，`startTime` 只在在线时给出且由格子按当前时间现算时长，`checkedAt` 用于判断数据是否过期）；`loopback-channel` 走 `register_loopback_channel`、`read_loopback_channel`、`request_loopback` 三条命令，`contracts/loopback.ts` 校验通道句柄、原始请求与原始响应，`bridge/loopback.ts` 封装这三条命令并提供按实例通道去重的通知订阅工厂。通道只收发原始 HTTP，解析与过期判定归插件。`ComponentTile` 组合组件身份、外观命令、接收就绪状态和失败隔离，不识别具体功能。`CollectBoxTile` 持有摘要与工具，其“新建收集项”菜单复用内容模块发布的新建动作；折叠尺寸由 `collapsedSize` 提供。`CollectBox` 复用内部 `DesktopTileViewport` 滚动列表，底部投放带和计数保持固定；`collectBox.browser.html` 使用生产组件和替代原生桥验证选择、菜单、删除及拖动取消。`MusicTile` 叠加内容和外框最小尺寸，`LocalClock` 持有活动时间订阅。

`WeatherTile` 通过 `ResizeObserver` 读取公共视口的实际内容盒，在拖动缩放和标题显隐时重新选择模块组合，不依赖释放后保存的外框尺寸；相同尺寸不重复更新，卸载时清理观察器。天气模式的内容阈值集中在 `weatherSizes`，自动判断、菜单尺寸及固定模式最小尺寸共用；固定状态及模式复用工作区组件 state，当前字号阈值仍按比例缩放，自动模式按完整、7 天、小时、摘要的顺序匹配，不使用滞回，实际文字测量待实现。模块切换支持进入退出过渡与连续反向衔接，服从有效动画设置；原生视觉效果待人工验收。

样式随拥有 DOM 的组件归档，公共外框、标题、视口和组件装配分别维护自己的同名 CSS。`styles/fenceTokens.css` 维护桌面围栏令牌（桌面工作区与浮岛共用的唯一内置值来源），`DesktopWorkspace.css` 维护工作区布局，`FenceTile.css` 维护围栏内容与投放状态。

## 依赖方向

`desktop-items/useDesktopItemImage.ts` 观察现有图像槽的可见性、尺寸和宿主暂停状态，经 `bridge/desktopItems.ts` 请求系统缩略图；实例身份与快照修订拒绝迟到结果，`DesktopItemGrid.css` 负责固定外框内的等比居中。`desktopItemImage.browser.html` 覆盖真实 Hook 的加载回退、迟到结果与暂停恢复。

原生内容拖放沿 `useCrossMonitorPreview` 和 `workspaceDropTarget` 交付类型白名单、可见格子与收集项几何。`ComponentTile` 声明实例类型，内容层通过 `expandOnDrop` 请求匹配后临时展开；公共外框不处理收集或媒体业务。`CollectBox` 的指针事件只协调一次准备与启动，实际拖出复用 `bridge/desktopItems.ts` 的 Windows OLE 路径；收入仍由原生宿主统一执行。`tiles/contentDrop.browser.html` 验证设置归属、临时展开、条目命中确认与拒绝不穿透。

```text
工作区装配 -> 内容与组件运行时装配 -> 公共格子
React feature -> bridge -> Tauri command/event -> src-tauri/src
```

`features/` 使用 `contracts/`、`bridge/` 和 `shared/`；`bridge/` 使用 `contracts/`。公共格子不导入业务内容目录。全局偏好 Provider 位于 `app/preferences/`，功能域只经 `usePreferences` 读取，不从设置目录导入。设置子域只经 `settings/ui/index.ts` 引用基元；前端与 Rust 的模块依赖由 `scripts/检查架构边界.mjs` 以基线校验，新增违规会让 `test:scripts` 失败。
