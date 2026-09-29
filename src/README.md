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

`main.tsx` 装配应用并按需加载收集项编辑窗口。`WorkspaceSurface` 组合逐屏权威投影与原生命令；`DesktopWorkspace` 负责未归类项目、围栏、官方组件及附加门户的共同工作区，持有全局选择和 OLE 协调。

`DesktopTileBase` 内部统一调用几何与展开 Hook，组合标题显隐、菜单、八向缩放及输入生命周期。调用方只提供几何参数和类型化业务回调；标题从公共 context 读取拖动入口，视口与活动内容读取同一实际折叠状态。`tileGeometry` 保存共享算法和可选 `minimum` 的默认值解析，使用 `contracts/tile` 的公共数据底线；内容层将内容需求与外壳、标题占位换算为外框下限。`tileDragPreview` 保存有界静态视觉；跨屏消息和 OLE 投放仍归工作区。

`OfficialComponentHost` 只同步运行时、过滤实例并分发内容；`ComponentTile` 组合组件身份、外观命令和失败隔离，不识别具体功能。`CollectBoxTile` 持有摘要与工具，其格子菜单通过 `create_collect_box` 复用统一创建流程，不计算初始位置或展开尺寸；折叠尺寸仍由 `collapsedSize` 提供。`MusicTile` 叠加内容和外框最小尺寸，`LocalClock` 持有活动时间订阅。

样式随拥有 DOM 的组件归档，公共外框、标题、视口和组件装配分别维护自己的同名 CSS。`DesktopWorkspace.css` 维护工作区主题，`FenceTile.css` 维护围栏内容与投放状态。

## 依赖方向

```text
工作区装配 -> 内容与组件运行时装配 -> 公共格子
React feature -> bridge -> Tauri command/event -> src-tauri/src
```

`features/` 使用 `contracts/`、`bridge/` 和 `shared/`；`bridge/` 使用 `contracts/`。公共格子不导入业务内容目录。职责决策见 [ADR-0018](../docs/adr/0018-共享格子外观并隔离布局撤销.md)，验证范围见[公共格子分层验收](../docs/validation/公共格子分层验收.md)。
