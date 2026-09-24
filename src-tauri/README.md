# Rust/Tauri 目录结构

`src-tauri/` 负责 Tauri 应用装配、Windows 桌面宿主、Shell 操作、工作区领域规则、持久化和前端命令边界。Rust 后端是文件事实、布局事务、原生拖放和 Windows API 结果的权威执行层。

## 目录职责

```text
src-tauri/
├─ src/
│  ├─ main.rs             # 进程入口
│  ├─ lib.rs              # Tauri 应用装配和命令注册
│  ├─ app/                # 生命周期、运行时状态和桌面接管
│  ├─ commands/           # 前端命令边界、校验和用例调度
│  ├─ workspace/          # 布局、围栏、门户、组件和事务规则
│  ├─ shell/              # 桌面项目事实、Shell 身份、菜单、剪贴板和拖放
│  ├─ host/               # 透明宿主、WebView2、DPI、命中图和接管
│  ├─ persistence/        # 布局快照和备份
│  ├─ component_runtime/  # 组件清单、能力和运行时状态
│  ├─ recovery/           # guardian、健康状态和异常恢复
│  ├─ platform/windows/   # Windows API、COM、OLE 和原生适配
│  ├─ contracts/          # Rust DTO、事件和结构化错误
│  └─ bin/guardian.rs     # 独立恢复进程入口
├─ capabilities/          # Tauri 权限声明
├─ components/            # Tauri 组件配置
├─ gen/                   # Tauri 生成文件
├─ icons/                 # 应用图标资源
├─ Cargo.toml             # Rust 依赖和构建配置
└─ tauri.conf.json        # Tauri 应用和打包配置
```

## 关键边界

- `commands/` 负责跨边界校验与调度，不持有长期领域状态。
- `workspace/` 负责自有布局、围栏归属、门户和组件布局事务，不执行文件操作。
- `shell/` 负责 Shell 项目身份、真实文件操作、菜单、剪贴板和 Shell OLE 数据对象。
- `host/` 负责逐显示器透明窗口、WebView2、坐标、命中图和桌面接管。
- `platform/windows/` 集中维护 COM、OLE、DPI、窗口消息和 Windows API 适配。
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
