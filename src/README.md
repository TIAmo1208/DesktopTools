# 前端目录结构

`src/` 负责 React 页面呈现、用户交互状态、展示模型转换和类型化 Tauri bridge。领域规则、文件事实和 Windows API 适配集中在 Rust/Tauri 后端。

## 目录职责

```text
src/
├─ app/              # 应用启动、根布局、全局状态和窗口装配
├─ features/         # 工作区、桌面项目、围栏、门户、组件和设置功能
│  ├─ workspace/     # 工作区表面、选择、焦点、框选、跨屏和状态
│  ├─ desktop-items/ # 桌面项目图标、名称、选中、打开和显示模型
│  ├─ fences/        # 围栏内容、成员布局、排序和投放
│  ├─ folder-portals/# 文件夹门户内容、导航和文件操作
│  ├─ components/    # 官方组件宿主及功能内容
│  ├─ music/         # 音乐控件和媒体布局
│  ├─ settings/      # 设置页面和偏好编辑
│  └─ system-menu/   # 桌面菜单和产品命令入口
├─ bridge/           # Tauri invoke/listen 调用与结果接收
├─ contracts/        # 前端 DTO、快照、命令和事件类型
├─ shared/           # 与具体功能无关的共享 UI 和工具
└─ styles/           # 全局主题变量和窗口基础样式
```

## 桌面格子

当前 `features/tiles/` 提供公共格子外框、标题、视口、菜单、几何交互、展开状态和拖动视觉快照。围栏、文件夹门户、收集箱、音乐、时间和 Debug 的内容行为由各自功能模块负责；移动、尺寸调整、边缘吸附、跨屏、静态帧冻结和取消应沿公共格子路径执行。

桌面格子目录分层调整方案记录在 `.scratch/desktop-tile-architecture/`，方案实施前不把目标目录写成当前结构。

## 依赖方向

```text
React feature -> bridge -> Tauri command/event -> src-tauri/src
```

`features/` 可以使用 `contracts/`、`bridge/` 和 `shared/`；`bridge/` 可以使用 `contracts/`。功能内容不直接调用 Tauri 或 Windows API。
