# Desktop Tools

Desktop Tools 是面向 Windows 的桌面内容工作区，负责呈现和组织桌面项目、围栏、文件夹门户与组件。Windows Explorer 继续提供桌面命名空间、Shell 操作和恢复兜底。

## 当前文件结构

```text
DesktopTools/
├─ src/                                  # React 前端
│  ├─ app/                               # 应用启动、根布局、全局状态装配
│  ├─ features/
│  │  ├─ workspace/                      # 桌面内容工作区
│  │  │  ├─ surface/                     # 透明桌面表面、显示器工作区呈现
│  │  │  ├─ interaction/                 # 命中、焦点、选择、框选、拖动
│  │  │  ├─ state/                       # 前端交互状态与临时编辑状态
│  │  │  └─ view-models/                 # 后端数据到展示模型的转换
│  │  ├─ desktop-items/                  # 桌面项目图标、名称、选中、打开
│  │  ├─ fences/                         # 围栏标题、成员、滚动、折叠、布局
│  │  ├─ folder-portals/                 # 文件夹门户展示
│  │  ├─ components/                     # Desktop Tools 组件展示
│  │  ├─ settings/                       # 设置界面
│  │  └─ system-menu/                    # 桌面菜单和产品命令入口
│  ├─ bridge/                            # Tauri invoke/listen 调用封装
│  ├─ contracts/                         # 前端 DTO 与事件类型
│  ├─ shared/                            # 通用 UI、主题、格式化工具
│  └─ styles/                            # 全局主题与窗口基础样式
│
├─ src-tauri/
│  └─ src/                               # Rust 后端
│     ├─ main.rs                         # 进程入口
│     ├─ lib.rs                          # Tauri 应用装配
│     ├─ app/                            # 生命周期、运行时状态、退出流程
│     ├─ commands/                       # 前端命令边界
│     ├─ workspace/                      # 桌面布局领域
│     ├─ shell/                          # Windows Shell 操作代理
│     ├─ host/                           # 透明桌面宿主
│     ├─ persistence/                    # 布局快照与备份
│     ├─ component_runtime/              # 组件清单、能力和运行时
│     ├─ recovery/                       # guardian、健康状态、重启恢复
│     ├─ platform/windows/               # Windows API 适配
│     ├─ contracts/                      # Rust 命令、事件、快照 DTO
│     └─ bin/guardian.rs                 # 独立恢复进程
│
├─ resources/                            # 图标、安装资源、静态资产
├─ docs/                                 # 领域文档、ADR、接口说明
└─ .scratch/                             # 需求、规格、工单、验证证据
```

## 构建与运行

发布构建使用 `npm run tauri:build`，由 Tauri CLI 构建前端并启用 `custom-protocol`，从内嵌资源加载设置与桌面页面。单独执行 `cargo build --release` 只选择优化 profile，不会启用 Tauri 发布协议，不能作为发布构建。需要直接调用 Cargo 时，先执行 `npm run build`，再执行 `cargo build --release --manifest-path src-tauri/Cargo.toml --features custom-protocol --bins`。

主程序和 guardian 位于 `build/target/release/`。Release 日志只写入 `%LOCALAPPDATA%/TIAmo/DesktopTools/`，不向控制台输出；设置窗口通过托盘或桌面菜单按需创建，关闭时销毁页面，不退出后台桌面宿主。运行验收需确认设置页面内容和逐屏桌面渲染，进程存活不能代替验收；资源采样应包含主进程、guardian 及其 WebView2 子进程。


## 职责边界

- `src/features/` 负责页面结构、视觉呈现、用户手势和前端临时状态。
- `src/bridge/` 负责 Tauri 命令调用、事件订阅、参数组装和结果接收。
- `src-tauri/src/commands/` 负责前端命令的校验与后端用例调度。
- `src-tauri/src/workspace/` 负责布局、围栏、选择、焦点和事务规则。
- `src-tauri/src/shell/` 负责桌面项目事实、系统图像、原生菜单、剪贴板和拖放。
- `src-tauri/src/host/` 负责逐显示器透明宿主、WebView2、DPI、命中图和桌面接管。
- `src-tauri/src/persistence/` 负责快照、备份、迁移和布局归档。
- `src-tauri/src/recovery/` 负责 guardian、健康状态、渲染器重启和异常恢复。
- `src-tauri/src/platform/windows/` 集中维护 Windows API 适配代码。

## 数据方向

```text
React Feature
  -> src/bridge
  -> Tauri command/event
  -> src-tauri/src/commands
  -> workspace / shell / persistence / component_runtime
  -> platform/windows
  -> Windows Shell / WebView2 / 文件系统
  -> contracts + events
  -> React Feature
```

## 参考文档

- [正式架构规格](docs/adr/0016-正式项目架构.md)
- [领域词汇](CONTEXT.md)

当前架构、职责边界与功能演进约束以 ADR 0016 和现有源码为准。
