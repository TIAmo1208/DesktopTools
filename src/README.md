# 前端目录结构

`src/` 负责 React 页面、交互状态、展示模型和类型化 bridge。领域规则、文件事实和 Windows API 由 Rust/Tauri 后端持有。

## 当前目录

```text
src/
├─ app/                     # 页面入口、根布局与窗口装配
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
│  │  ├─ music/             # 媒体订阅、控制、响应式布局与最小尺寸
│  │  ├─ clock/             # 活动时间订阅与呈现
│  │  ├─ weather/           # 城市选择、天气模块、响应式布局与显示到期处理
│  │  └─ debug/             # 宿主诊断与测试内容
│  ├─ components/           # 运行时注册、实例能力、开关与失败隔离装配
│  ├─ desktop-items/        # 围栏与未归类区域共享的 Shell 项目呈现
│  ├─ notifications/        # 统一消息入口
│  ├─ settings/             # 设置页面及偏好
│  └─ system-menu/          # 桌面菜单入口
├─ bridge/                  # 原生命令、事件、结果解码与订阅生命周期
├─ contracts/               # 前端 DTO、快照、命令和事件类型
├─ shared/                  # 共享 UI 与工具
└─ styles/                  # 全局主题和窗口基础样式
```

## 入口与职责

`features/settings/WeatherSettings` 提供和风 JWT 草稿输入、私钥导入与显隐、临时城市搜索、分项验证、保存替换和清除。`bridge/weather.ts` 对接设置窗口专用凭据命令，以及桌面天气的城市读写、搜索和预报命令；敏感草稿不进入普通偏好、浏览器存储或事件广播。关闭窗口与分类导航共享最新草稿状态，取消后拒绝迟到结果；`weatherSettings.browser.html` 使用生产组件和替代桥验证相关交互。`WeatherTile` 沿用公共格子呈现预报，通过一次性计时处理当前天气的 24 小时展示上限，隐藏、折叠或宿主暂停时撤销计时、恢复活动时重新检查，不因此增加天气请求。预报读取沿用公共格子实际展开状态与 `desktop-refresh-changed` 事件，暂停或折叠时拒绝旧响应，恢复时保留已有缓存并重新读取；展开的天气实例通过桥接监听预报更新事件并登记后端共享活动调度。

小时与日预报按条目结束时间过滤，并只为最近到期点设置一次性计时；隐藏、折叠和暂停停止计时，恢复后重新检查。当天标签与最高最低温按目标城市日期匹配，过期过滤不触发联网补齐。

`main.tsx` 装配应用并按需加载收集项编辑窗口。`WorkspaceSurface` 组合逐屏权威投影与原生命令；`DesktopWorkspace` 负责未归类项目、围栏、官方组件及附加门户的共同工作区，持有全局选择和 OLE 协调。`CollectBox` 呈现文本、图片和文件映射，维护当前格子的选择、清理确认及复制拖出生命周期；`collectItemState.ts` 处理混排／分离、筛选及实例状态，`collectMarkdown.ts` 提取文本摘要。

`collect-box/collectEditorImages.ts` 为 Vditor 图片补充本地显示数据、适配放大预览，并修正图片说明转义与目标地址编码，保持 Markdown 图片语义；`CollectBox` 与 `CollectEditor` 通过 `bridge/collect.ts` 把链接交给系统打开。

`CollectBox` 的图片槽使用可见性与尺寸观察器，通过 `readCollectImage` 请求缩略图或 GIF 悬停原图；`CollectEditor` 管理编辑、保存和关闭清理的调用生命周期。两者通过 `bridge/collect.ts` 访问后端，文件校验与回收归后端负责。

`DesktopTileBase` 内部统一调用几何与展开 Hook，组合标题显隐、菜单、八向缩放及输入生命周期。调用方只提供几何参数和类型化业务回调；标题从公共 context 读取拖动入口，视口与活动内容读取同一实际折叠状态。`tileGeometry` 保存共享算法和可选 `minimum` 的默认值解析，使用 `contracts/tile` 的公共数据底线；内容层将内容需求与外壳、标题占位换算为外框下限。`tileDragPreview` 保存有界静态视觉；跨屏消息和 OLE 投放仍归工作区。

`OfficialComponentHost` 只同步运行时、过滤实例并分发内容；`ComponentTile` 组合组件身份、外观命令、接收就绪状态和失败隔离，不识别具体功能。`CollectBoxTile` 持有摘要与工具，其“新建收集项”菜单复用内容模块发布的新建动作；折叠尺寸由 `collapsedSize` 提供。`CollectBox` 复用内部 `DesktopTileViewport` 滚动列表，底部投放带和计数保持固定；`collectBox.browser.html` 使用生产组件和替代原生桥验证选择、菜单、删除及拖动取消。`MusicTile` 叠加内容和外框最小尺寸，`LocalClock` 持有活动时间订阅。

`WeatherTile` 通过 `ResizeObserver` 读取公共视口的实际内容盒，在拖动缩放和标题显隐时重新选择模块组合，不依赖释放后保存的外框尺寸；相同尺寸不重复更新，卸载时清理观察器。天气模式的内容阈值集中在 `weatherSizes`，自动判断、菜单尺寸及固定模式最小尺寸共用；固定状态及模式复用工作区组件 state，当前字号阈值仍按比例缩放，自动模式按完整、7 天、小时、摘要的顺序匹配，不使用滞回，实际文字测量待实现。模块切换支持进入退出过渡与连续反向衔接，服从有效动画设置；原生视觉效果待人工验收。

样式随拥有 DOM 的组件归档，公共外框、标题、视口和组件装配分别维护自己的同名 CSS。`DesktopWorkspace.css` 维护工作区主题，`FenceTile.css` 维护围栏内容与投放状态。

## 依赖方向

`desktop-items/useDesktopItemImage.ts` 观察现有图像槽的可见性、尺寸和宿主暂停状态，经 `bridge/desktopItems.ts` 请求系统缩略图；实例身份与快照修订拒绝迟到结果，`DesktopItemGrid.css` 负责固定外框内的等比居中。`desktopItemImage.browser.html` 覆盖真实 Hook 的加载回退、迟到结果与暂停恢复。

原生内容拖放沿 `useCrossMonitorPreview` 和 `workspaceDropTarget` 交付类型白名单、可见格子与收集项几何。`ComponentTile` 声明实例类型，内容层通过 `expandOnDrop` 请求匹配后临时展开；公共外框不处理收集或媒体业务。`CollectBox` 的指针事件只协调一次准备与启动，实际拖出复用 `bridge/desktopItems.ts` 的 Windows OLE 路径；收入仍由原生宿主统一执行。`tiles/contentDrop.browser.html` 验证设置归属、临时展开、条目命中确认与拒绝不穿透。

```text
工作区装配 -> 内容与组件运行时装配 -> 公共格子
React feature -> bridge -> Tauri command/event -> src-tauri/src
```

`features/` 使用 `contracts/`、`bridge/` 和 `shared/`；`bridge/` 使用 `contracts/`。公共格子不导入业务内容目录。
