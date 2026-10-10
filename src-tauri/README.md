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
│  ├─ collect/            # 文本、图片、文件映射、资源、收入取出与删除恢复
│  ├─ host/               # 透明宿主、WebView2、DPI、命中图和接管
│  ├─ persistence/        # 布局快照和备份
│  ├─ weather/            # 天气凭据、临时验证会话及和风 HTTP/JWT
│  ├─ component_runtime/  # 组件清单、能力和运行时状态
│  ├─ recovery/           # guardian、健康状态和异常恢复
│  ├─ platform/windows/   # Windows API、COM、OLE 和原生适配
│  ├─ contracts/          # Rust DTO、事件和结构化错误
│  └─ bin/guardian.rs     # 独立恢复进程入口
├─ capabilities/          # Tauri 权限声明
├─ components/            # Tauri 组件配置
├─ gen/                   # Tauri 生成文件
├─ icons/                 # 应用图标资源
├─ installer/             # NSIS 安装与卸载生命周期钩子
├─ Cargo.toml             # Rust 依赖和构建配置
├─ tauri.conf.json        # Tauri 应用配置
└─ tauri.installer.conf.json # 当前用户 NSIS 测试安装包覆盖配置
```

## 关键边界

- `commands/weather.rs` 仅允许设置窗口提交天气用例，不向原生桌面 transport 注册凭据入口。`weather/credentials.rs` 按应用配置目录隔离 Windows Credential Manager 记录；`weather/mod.rs` 管理草稿、并发互斥、取消及保存门槛；`weather/provider.rs` 校验官方 Host、签发 JWT 并验证城市和天气数据。网络任务只在用户操作时创建，禁止重定向，限制超时与响应大小；城市和天气验证响应不落盘。桌面天气请求调度、缓存与城市持久化尚未实现。

- `commands/` 负责跨边界校验与调度，不持有长期领域状态。
- `workspace/` 负责自有布局、围栏归属、门户和组件布局事务，不执行文件操作。各类格子的持久展开边界统一使用工作区契约中的公共尺寸底线，临时投射单独校验；内容可读性和标题占位由前端负责。
- `shell/` 负责 Shell 项目身份、真实文件操作、菜单、剪贴板和 Shell OLE 数据对象。
- `shell/images.rs` 持有独立图像 STA、去重队列和有界 LRU；`platform/windows/shell_image.rs` 在所属线程提取 Shell 缩略图、转换 PNG 及原生拖动位图。`get_desktop_item_image` 校验当前项目身份后请求图像，Shell 变化使缓存和在途结果失效，原生拖动只读取已完成缓存。
- `host/` 负责逐显示器透明窗口、WebView2、坐标、命中图和桌面接管。
- `platform/windows/` 集中维护 COM、OLE、DPI、窗口消息和 Windows API 适配。
- `platform/windows/drop_content.rs` 探测有界源格式、声明业务白名单、协商效果并提取自有数据；`desktop_host.rs` 在既有 OLE 入口按最高命中目标分流。`app/desktop_drop.rs` 复核实例、功能开关与运行时注册，再交给 `collect/drop.rs` 或 Shell STA；后台不持有原生拖放数据对象。
- `collect/drop.rs` 负责一次拖入的格式去重、Markdown 追加及资源收入，正文保存使用既有 Windows 原子替换能力，失败保留已保存正文与可追踪的资源路径。
- `collect/items.rs` 管理根目录独立图片文件与文件映射记录；`collect/transfer.rs` 管理复制导出及内部逐项复制、追加，复用 `references.rs` 改写复制资源的 Markdown 引用。独立图片复制到 `collect` 根目录并使用文件名身份，扫描不递归文档资源目录，映射不复制源文件。编辑窗状态由 `commands/collect.rs` 在两种命令 transport 和 OLE 业务入口通过同一窗口注册锁查询并保护写入，正文追加逐项拒绝编辑中的目标。
- `collect/mod.rs` 有界读取收集项引用的本地图片；`commands/collect.rs` 提供图片读取命令，并校验网页、邮件链接后复用 Windows 默认打开能力。
- `collect/cleanup.rs` 管理附件归属证据与关闭清理，`collect/references.rs` 负责 Markdown 引用解析和改名转换。
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

`npm run tauri:installer` 先由 `scripts/prepare-licenses.mjs` 整理当前锁定依赖许可，再加载安装包覆盖配置构建与打包。普通开发和纯程序构建不依赖生成的许可文件。安装钩子复用应用桌面菜单命令，不持有业务状态；生命周期与分发边界见 [ADR-0024](../docs/adr/0024-采用当前用户NSIS安装包.md)。
