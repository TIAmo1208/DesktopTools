---
name: desktop-tools-execution
description: DesktopTools（Tauri + React，Windows 桌面内容工作区）仓库内的执行总入口：任务分诊、TMP/互斥体/产物路径等环境前提、按改动类型选择验证层、制品与证据纪律。任何在本仓库接需求、修缺陷、改 UI、跑构建/测试/断言页或做真机验收的任务先加载。
whenToUse: 在 E:\Project\WebView\DesktopTools 改代码、跑验证、同步事实文档或做真机验收时；纯问答不需要。
---

# DesktopTools 执行入口

先分诊，再按需读 references。不要一次读完 `docs/agents/`。

路径约定：`docs/`、`scripts/`、`build/`、`plugin/` 均相对仓库根；`references/` 相对本 skill 目录。

## 1. 任务分诊

| 任务类型 | 先读 | 继续 |
| --- | --- | --- |
| 修复、回归 | `docs/agents/修复验收轮次.md` | `references/acceptance.md` |
| 构建、启动、环境 | `docs/agents/构建运行.md` | `references/bootstrap.md` |
| UI、视觉、交互验证 | `docs/validation/验收流程.md` | `references/evidence.md` |
| 主题、插件样式 | `docs/design/主题变量契约.md` | `references/theme.md` |
| 安装包、版本、分发 | `docs/agents/构建运行.md` | `references/installer.md` |
| 新界面、改布局 | `docs/design/设计确认流程.md` | 确认后再实现 |
| 纯文档、ADR | `docs/agents/事实文档执行.md` | 直接做 |

一次任务跨两类以上时，先拆成可独立验收的子任务，不要混在一轮里。

## 2. 三条硬规则

1. **制品一致**：调试和验收用同一份构建产物；改前端先 `npm run build`，改 Rust 先构建，改主题先同步。只跑源码测试不算调试。
2. **复现同层**：缺陷在哪一层就在哪一层写会失败的复现；纯逻辑用单测，样式/交互动用断言页，原生用窗口盒或真机。
3. **证据不冒充**：软件模拟、浏览器断言、运行时探针、真机日志、人眼截图分开记录；低层通过不能证明高层正确。

## 3. 环境（每轮做一次）

细节与排错顺序见 `docs/agents/环境前提.md`；这里只列最常用的约束。

- Rust 命令走 npm script：`npm run test:rust`、`npm run tauri:dev`、`npm run tauri:build`、`npm run tauri:installer` 已注入 `TMP`。直接调用 `cargo` 时先把 `TMP` 指向 `build\tmp`。
- 设置窗加载 `build/frontend`，不是 Vite 页面；改前端后先 `npm run build` 再重开设置窗。
- 主题/插件改动后先 `npm run sync:plugin`；开发运行扫描 `build/target/debug/plugin`。
- 跑开发版前先退出已安装实例：互斥体 `Local\DesktopTools.DesktopMenu.Instance` 被占用时，开发版会静默退出（exit 0，日志止于「初始化完成」）。
- 断言页依赖 `npm run dev` 的 `127.0.0.1:1420`；被其他进程占用时运行器会打印 PID 并以退出码 2 结束，按环境问题处理。
- 后台任务句柄不跨回合：长任务在同一回合内分块，或增量写 JSONL/日志后再汇总。

## 4. 停止与升级

- `0xC0000142` 且任何命令都无输出：判定为执行环境级失败，停下来处理环境，不换工具链、不逐条重试。
- `cl.exe D8050`：先检查 `TMP`，不要改 MSVC 版本、`_CL_`、`/Z7`、`/Brepro` 或排查安全软件。
- 开发版 exit 0 且日志止于「初始化完成」：按互斥体处理，不要当成启动成功。
- 用户未确认：不得宣称修复完成；审核 pass 也不等于验收通过。
- 环境性失败：按 `docs/validation/验收流程.md` §6 的清单判定，清单外的失败按真实缺陷处理。

## 5. 收尾

- 只结束本轮启动的开发服务器、无头 Edge、开发版实例与后台任务；确认 1420 无监听。
- 只暂存通过验收的改动；同一行为变更与受影响的事实文档放在同一提交。
- 验收记录写 `build/validation/记录/`，写清构建、命令、退出码、证据类型与未验证项。

## References

- [bootstrap](references/bootstrap.md)：环境预检、产物路径、互斥体、端口与后台任务。
- [acceptance](references/acceptance.md)：修复七步、红绿、制品指纹、审核包、收尾。
- [evidence](references/evidence.md)：L0–L4、八项人眼清单、探针、样式表缓存。
- [theme](references/theme.md)：主题目录同步、缓存键、CSSOM 核对顺序。
- [installer](references/installer.md)：安装包验证的前置与安全边界。
- [review-packet](references/review-packet.md)：冻结 diff 与证据包模板。
