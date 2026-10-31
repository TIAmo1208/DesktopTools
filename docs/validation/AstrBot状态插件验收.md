# AstrBot 状态插件验收

## 结论

AstrBot 状态插件（`plugin/AstrBot 状态/`）与支撑它的宿主能力通道已落地。插件经 `ComponentCapability::ProbeServiceStatus` 能力通道探活本机 AstrBot 控制台，不走平铺原生命令白名单；探测地址与端口是宿主常量，插件无法传入任意地址；全程不读取任何凭据。

前端门禁与受影响的 Rust 单测已通过。真实 AstrBot 进程的停启验收、完整生产构建与 Rust 格式化尚未执行，见「未验证项」。

## 验收项与方法

| 验收项 | 方法 | 结果 |
| --- | --- | --- |
| 能力按实例授权 | `component_runtime` 单测：未声明能力的实例被拒绝，声明者可调用 | 通过 |
| ini 解析能力声明 | `required_capabilities` 逗号分隔 kebab-case；未知能力与重复声明使整份清单不注册 | 通过 |
| 授权闸门未放宽 | 官方清单分支保持原语义；插件槽仅放行单条能力，不作组合 | 通过 |
| 四档判定 | `commands::probe` 单测覆盖正常结构、浮点与字符串启动时间、401、500、端口无人监听、超时 | 通过 |
| 超时不判成离线 | 超时重试后归 `unknown`，不得显示为「离线」 | 通过 |
| 连接被拒判成离线 | 无人监听时归 `off`（`closed_port_reports_off`） | 通过 |
| 返回结构 | 仅 `level`、`startTime`、`checkedAt`；`startTime` 只在 `ok` 时给出 | 通过 |
| 请求约束 | 独立 HTTP 客户端，不设 `https_only`、不启用 cookie jar、不带 `Origin`、2 秒超时 | 通过（静态核对） |
| 插件调用链 | 断言插件按命令名与对象参数调用能力通道，且只发出 `invoke_component_capability` | 通过 |
| 前端展示 | 就绪时长由 `startTime` 现算；`unknown` 不上绿、不写离线；超过 60 秒显示数据过期 | 通过 |
| 前端契约 | 探活载荷按能力区分校验，未知档与非 `ok` 带启动时间被拒绝 | 通过 |
| 真实进程停启 | 启动与停止 AstrBot 后核对卡片状态 | 未执行 |

## 证据

### 前端门禁

| 命令 | 结果 |
| --- | --- |
| `npm run typecheck` | 通过 |
| `node --test plugin/AstrBot 状态/index.test.mjs src/contracts/component.test.ts` | 17 通过 / 0 失败 |
| `git diff --check` | 干净 |

### Rust 单测

| 命令 | 结果 |
| --- | --- |
| `cargo test --lib probe` | 15 通过 / 0 失败 |
| `cargo test --lib component_runtime` | 17 通过 / 0 失败 |

执行前须按 `docs/validation/MSVC构建阻塞与绕行.md` 设置 `TMP`，否则 `ring` 无法编译。

## 编译修复记录

提交 `9740255` 落地本功能时，Rust 侧因 MSVC 环境阻塞未能编译，提交说明已注明待验证。后续绕行生效后，编译暴露出以下错误并已修复：

| 错误 | 位置 | 修复 |
| --- | --- | --- |
| `E0603 module probe is private` | `commands/mod.rs` | 模块声明改为 `pub mod probe`，供 `component_runtime` 调用 |
| `E0599 no method json` | `commands/probe.rs` | `reqwest` 未启用 `json` feature，改为读取文本后用 `serde_json::from_str` 解析，不新增依赖 |
| `E0521 borrowed data escapes` | `commands/probe.rs` | 错误链遍历的类型标注补 `'static` 生命周期 |

### 判定逻辑缺陷

编译修复后，`closed_port_reports_off` 用例失败，暴露出停机时会显示「未知」而非「离线」的功能缺陷：`reqwest 0.13` 在 Windows 上把「连接被拒」报告为超时，错误链中没有可识别的连接被拒信息。

修复方式：超时类失败改用独立 TCP 探针确认端口是否监听，连不上判 `off`，连得上才归 `unknown`。该用例现通过。

## 未验证项

| 项目 | 说明 |
| --- | --- |
| 真实 AstrBot 进程停启 | 需启动与停止 AstrBot 后按四步手工验收核对卡片状态 |
| 完整生产构建 | `npm run build` 与 `npm run tauri:build` 未在本轮执行 |
| Rust 格式化 | `npm run format:rust:check` 因 `cargo +nightly fmt` 挂起未执行 |
| 全量测试 | `cargo test --all-targets` 有 10 项失败，均在 `desktop_menu`、`collect::cleanup`、`desktop_sources`、`shell` 模块，属 Shell/COM 与文件系统权限问题，与本功能无关 |
