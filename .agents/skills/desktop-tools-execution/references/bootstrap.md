# bootstrap：环境与产物

进入构建、启动或真机取证前读本页。规则原文见 `docs/agents/环境前提.md` 与 `docs/agents/构建运行.md`。

## 一次性预检

```bash
node scripts/提示产物路径.mjs
netstat -ano | grep ':1420'
tasklist /FI "IMAGENAME eq desktop-tools.exe" /NH
```

- 1420 有监听：确认是本仓库的 `npm run dev`，不是其他服务。
- `desktop-tools.exe` 在运行：先正常退出，否则开发版会因互斥体静默退出。
- 产物路径缺失：按下面的顺序补。

## TMP

`npm run test:rust`、`tauri:dev`、`tauri:build`、`tauri:installer` 已由 `scripts/with-tmp.mjs` 注入 `TMP=build\tmp`。直接调用 `cargo` 时：

```powershell
New-Item -ItemType Directory -Force "$PWD\build\tmp" | Out-Null
$env:TMP = "$PWD\build\tmp"
cargo check --manifest-path src-tauri/Cargo.toml
```

只设 `TEMP` 无效。`cl.exe D8050` 先查这里，不要换 MSVC 版本或改编译参数。

## 互斥体与 exit 0

已安装实例占用 `Local\DesktopTools.DesktopMenu.Instance` 时，开发版按副实例处理：exit 0，日志止于「初始化完成」，没有窗口。看到这个组合不要当成启动成功，先退出已安装实例。

## 产物路径

| 产物 | 路径 | 何时更新 |
| --- | --- | --- |
| 设置窗/桌面宿主前端 | `build/frontend` | 改前端后 `npm run build`，再重开设置窗 |
| 开发插件目录 | `build/target/debug/plugin` | 改 `plugin/` 后 `npm run sync:plugin` |
| Rust 可执行文件 | `build/target/debug/desktop-tools.exe` | `npm run tauri:dev` 或构建命令 |

`npm run tauri:dev` 启动时会打印前两项的实际路径。设置窗不是 Vite 页面，改完不重建就会看到旧 UI。

## 端口、Edge 与后台任务

- 断言页依赖 `127.0.0.1:1420`；运行器会核对首页含 `Desktop Tools`，被其他进程占用时打印 PID 并以退出码 2 结束。
- 断言与 Rust/前端构建串行执行；并发会互相等锁，制造 CDP 超时。
- 收尾确认 1420 无监听；无可见 Edge 窗口时清理本轮无头残留。
- 后台任务句柄不跨回合：长任务同回合分块，或把结果增量写入 JSONL/日志。

## 命令无输出（0xC0000142）

任何命令都拿不到输出（包括 `echo hi`）且退出码 `3221225794`：这是进程启动阶段失败，不是命令本身的问题。判定为执行环境级失败，停下来处理环境，不逐条重试业务命令。

## 工具读取失败

`ops_read_files`、`ops_glob` 或官方 `read` 报 `dsh-ops-worker.exe ENOENT`：说明 dsh-ops 运行器缺失或被杀软隔离。先改用 `bash` 读取完成当前任务，再把修复 dsh-ops 作为独立事项；不要在每次读取时重复重试。
