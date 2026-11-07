# Desktop Tools 项目规则

Desktop Tools 是 Windows 桌面内容工作区（Tauri + React）：前端负责呈现、交互状态和类型化 bridge，Rust 后端持有领域规则、文件事实和 Windows API 执行。

通用执行节奏、授权边界、工具止损、分层验证与交接规则由全局 `~/.dsh/AGENTS.md` 下发；本文件只写项目特有内容与导航。

## 项目导航

- 使用者文档：根 `README.md` 说明产品定位、当前功能、构建运行与功能边界，不链接内部事实文档。
- 代码结构：`src/README.md`、`src-tauri/README.md` 说明实际目录、模块职责、入口与依赖方向。
- 领域事实：`CONTEXT.md` 记录领域语言与不变量；已确认决策见 `docs/adr/`；视觉规范见 `docs/design/视觉规范.md`（设置界面行级细则见 `docs/design/设置界面规范.md`）；验收方法见 `docs/validation/验收流程.md`，本机验收记录写在本机 `build/validation/记录/`（不入库）。

按触发条件读取，不整份套用：

| 场景                       | 入口                               |
| -------------------------- | ---------------------------------- |
| 需求、规格和工单           | `docs/agents/工单系统.md`          |
| 需求分流标签               | `docs/agents/需求分流标签.md`      |
| 领域文档布局               | `docs/agents/领域文档.md`          |
| 撰写 ADR、按事实文档改代码 | `docs/agents/事实文档执行.md`      |
| Rust 格式化                | `docs/agents/Rust格式化.md`        |
| 构建、命令与环境前提       | `docs/agents/构建运行.md`          |
| 代码风格与注释             | `docs/agents/代码风格.md`          |
| 前端实现与生命周期约定     | `docs/agents/前端约定.md`          |
| 设计确认流程               | `docs/design/设计确认流程.md`      |
| UX/UI 工具调用（Penpot）   | `docs/agents/UI工具调用.md`        |
| CSS 归属约定               | `docs/design/CSS约定.md`           |
| 设置界面视觉细则           | `docs/design/设置界面规范.md`      |
| 主题可覆盖的令牌           | `docs/design/主题变量契约.md`      |
| AstrBot 格子设计           | `docs/design/AstrBot状态格子.md`   |
| 执行验收与取证             | `docs/agents/验收取证.md`          |
| 抓图分级与截图纪律         | `docs/agents/取证工具.md`          |

- 前端分层：`src/app` 页面装配，`src/features` 工作区与各格子内容，`src/bridge` 原生命令、事件与订阅生命周期，`src/contracts` 跨边界类型，`src/styles` 主题与窗口基础样式。
- 数据方向：`React feature -> bridge -> Tauri command/event -> src-tauri/src -> contracts + events -> React feature`。`src/features` 使用 `contracts/`、`bridge/` 和 `shared/`；`bridge/` 使用 `contracts/`；公共格子不导入业务内容目录。

## 构建与门禁

- 类型检查 `npm run typecheck`；受影响测试用 `node --test <测试文件>`；项目全量测试 `npm test`（含前端、组件与 `cargo test --all-targets`）；提交前 `git diff --check`。
- 设置窗口在开发期加载 `build/frontend` 构建产物，不是 vite 页面；改前端后先 `npm run build` 再重开设置窗，否则看到的是旧 UI。
- 本机跑开发版前先退出已安装实例：桌面菜单互斥体 `Local\DesktopTools.DesktopMenu.Instance` 被占用时，开发版按副实例处理并静默退出（exit 0，日志止于「初始化完成」）。
- 本机 Rust 操作前须把 `TMP` 指向 `build\tmp`，否则 `cl.exe D8050` 阻断构建。命令、全量门禁与构建提速口径见 `docs/agents/构建运行.md`。

## 安全边界

- 凭据由 Rust 侧存入 Windows Credential Manager；敏感草稿不进入日志、普通偏好、浏览器存储或事件广播。
- 文件打开、重命名、复制、移动和删除通过 Windows Shell 执行；删除收集记录不删除文件映射指向的源文件。

## 事实文档

- 修改功能、目录结构、模块职责或入口时，同一任务内同步受影响的 README、`CONTEXT.md`、相关 ADR 与视觉规范；仅更新实际受影响的文档。
- 改代码前读相关 `CONTEXT.md`、ADR 与视觉规范，只落实已写明的定义、决策和边界；文档未覆盖的新需求先记录，完成未阻塞的工作后汇总询问。
- README 只写当前实现与使用者需要的边界，不链接 `CONTEXT.md`、ADR 等内部事实文档；决策原因与内部约束留在事实文档里。
- 同一行为变更与其文档同步组织在同一提交；修复已定位的回归时注明引入问题的提交或具体改动，无法确定时标为未确认。

## 临时方案与暂存

- `.scratch/` 是本地临时工作区（已 gitignore），只放待查问题与修复过程中的中间文件；不得建在 `src/`、`src-tauri/` 下。按问题分目录，问题记录固定叫 `问题.md`，其余按用途命名（`probe-*.py`、`log-*.txt`、`证据快照/`）。问题记录不得散落在 `.scratch/` 根。
- 问题修复且事实已同步到 `CONTEXT.md` 或 ADR 后删除该目录；执行中、待决策、待验收的保留。
- 暂存按文件路径显式指定，不用目录级 `git add <目录>`：目录级暂存会把刚生成的临时文件一并收入索引。

## Git

- 提交信息使用中文，按实际改动组织为一个或多个修改类型分组，如 `develop` 或 `bugfix`。
- 每个分组首行填写修改类型，后续每行以 `- ` 开头填写一项修改内容；组内连续排列，分组之间保留一个空行：

    ```text
    develop
    - 添加功能1
    - 修改功能

    bugfix
    - 修复问题1
    - 修复问题2
    ```

- 提交信息使用真实换行写入。命令行提交优先使用 `printf '%s\n' 'develop' '- 添加功能1' '- 修改功能' '' 'bugfix' '- 修复问题1' '- 修复问题2' | git commit -F -`。
- 提交或 amend 后，使用 `git show -s --format='%B' HEAD` 核对首行、条目顺序、空行数量和换行内容，再检查提交哈希、仓库引用与工作区状态。
- 重写提交历史时，保留作者、提交时间和工作区现有改动，并核对新旧提交映射、提交信息和仓库引用。

## 注释与命名

- 类、接口声明、接口实现、变量和重要逻辑块都要有简短中文用途说明。
- 事件接口使用 `onSomething` 命名；事件处理函数使用 `handleSomething` 命名。
- 日志分级、语言与保留原文的规则见 `docs/agents/代码风格.md`。

## 实现与设计

- 计划、功能扩展方式、轮询与事件驱动取舍、异步回调的 ref 与清理规则见 `docs/agents/前端约定.md`。
- 新增或改变界面时按 `docs/design/设计确认流程.md` 走确认流程；视觉基线是 `docs/design/视觉规范.md`。
- 样式写在哪、归谁维护见 `docs/design/CSS约定.md`；主题可覆盖的令牌见 `docs/design/主题变量契约.md`。
