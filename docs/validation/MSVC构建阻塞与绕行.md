# MSVC cl.exe D8050 构建阻塞与绕行方案

## 结论

本机 Rust 侧构建在 `ring` 的 C 代码编译阶段失败，`cl.exe` 报 `D8050 : cannot execute '...c1.dll': failed to get command line into debug records`。已确认这是本机构建环境的路径问题，不是工具链版本、编译参数或安全软件问题。经过多轮对照实验，可复现的绕行方案是：**把 `TMP` 指向项目工作树内的目录**。

```cmd
New-Item -ItemType Directory -Force "E:\Project\DesktopTools\build\tmp" | Out-Null
$env:TMP = "E:\Project\DesktopTools\build\tmp"
npm run tauri:build
```

日常开发使用同一组环境变量：

```cmd
New-Item -ItemType Directory -Force "E:\Project\DesktopTools\build\tmp" | Out-Null
$env:TMP = "E:\Project\DesktopTools\build\tmp"
npm run tauri:dev
```

`build/` 已由 `.cargo/config.toml` 作为 `target-dir` 使用，`tmp` 子目录随构建目录一起属于可再生成内容。首次使用前必须创建该目录，`TMP` 指向不存在的路径会立即复现 D8050。

## 现象

| 项目 | 事实 |
| --- | --- |
| 报错原文 | `cl : Command line error D8050 : cannot execute '<MSVC 路径>\c1.dll': failed to get command line into debug records` |
| 首次失败位置 | `ring v0.17.14` 的 `crypto/curve25519/curve25519.c` |
| 影响范围 | 所有依赖 C 编译的 crate 同时失败，后续 Rust 编译与测试无法进行 |
| 编译器自身 | 正常。同一 `cl.exe` 在独立命令行编译同一文件退出码 0 |

## 已排除的方向

以下方向经过实测或源码核对，确认与本问题无关，后续不必重复尝试。

| 方向 | 结论 | 依据 |
| --- | --- | --- |
| MSVC 版本 | 排除 | 14.38 与 14.44 表现完全一致 |
| `vcvars64` 环境 / `cmd` 直接执行 | 排除 | 两种方式同样报 D8050 |
| 清空 `CL`、`INCLUDE` | 排除 | 本就是未设置状态，清空后仍失败 |
| 编译参数形态 | 排除 | 完整复刻 Cargo 传给 `cl` 的参数手工执行退出码 0 |
| 并发编译干扰 | 排除 | `CARGO_BUILD_JOBS=1` 仍失败 |
| `_CL_` / `/Z7` / `/Brepro` 覆盖 | 排除 | `_CL_=/Z7-`、`_CL_=/Brepro-` 及其组合均失败；已用 `_CL_=/DCL_PROBE=1` 证明该通道本身有效 |
| cc-rs 使用响应文件 | 排除 | cc-rs 1.4.4 源码不含响应文件逻辑 |
| 环境变量污染 | 排除 | `RUSTC_WRAPPER`、`sccache`、`CL`、`INCLUDE`、`CFLAGS` 均未设置 |
| 安全软件 / 沙箱拦截 | 排除 | 新建一个不存在的目录作为 `TMP` 即可用 `cl` 直接复现，全程无安全软件参与 |
| 临时目录权限或不存在 | 排除 | 用户 Temp 可写且可正常编译，问题取决于路径**位置**而非可写性 |

## 可复现事实

对同一条 `cl /nologo /c /Z7 /Brepro` 命令，只改变 `TMP`：

| `TMP` 取值 | 结果 |
| --- | --- |
| 不设置（继承用户 Temp） | 通过 |
| 用户 `AppData\Local\Temp` 显式设置 | 通过 |
| 一个不存在的目录 | **失败，原文 D8050** |
| 项目工作树内目录（`build/tmp`） | 通过 |
| 项目工作树外目录（`E:\` 根下、用户 Temp 之外） | 失败 |

补充事实：只设置 `TEMP` 不生效，必须设置 `TMP`；产物目录（`-Fo`）与结果无关。

在完整构建链上（cargo → build script → cc-rs → cl），`TMP` 指向项目工作树内目录时 `ring` 可正常编译，指向工作树外时稳定失败。

## 未定根因

可复现的规律是「`TMP` 是否位于项目工作树内」，但未定位到具体系统属性。候选因素包括项目根目录的父级 ACL（存在 `Everyone: Deny DeleteSubdirectoriesAndFiles` 显式 ACE）经子进程继承后的影响，以及在 cargo → cc-rs → cl 的进程链中 `%TMP%` 展开与实际写入路径的差异。现有数据无法区分这些候选。

如需定案，可用 Process Monitor 抓取 `cl.exe` 对 `%TMP%` 的 `CreateFile` 调用一次确认。

在根因未定之前，按「可用绕行 + 未定根因」对待，不要据此推断为权限或安全软件问题。

## 使用约束

- 首次使用必须创建 `build/tmp` 目录；`TMP` 指向不存在的路径会立即复现 D8050。
- 必须设置 `TMP`，仅设置 `TEMP` 无效。
- 该设置只影响当前命令行会话，不修改系统级环境变量。
- 换用其他构建目录（例如自定义 `CARGO_TARGET_DIR`）时，`TMP` 需指向同一工作树内的可用目录。
- 该绕行只解决本机构建问题，不改变项目的构建配置、工具链版本或 CI 行为。
