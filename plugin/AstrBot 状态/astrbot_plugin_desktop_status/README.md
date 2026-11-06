# AstrBot 状态采集插件（AstrBot 侧）

这是给 **AstrBot（Star 插件）** 用的插件包，不是 Desktop Tools 的格子插件。它随 Desktop Tools 安装包一起分发，放在这里是为了让用户「手动安装」时有现成文件，Desktop Tools 运行时**不会**扫描本目录（宿主只认插件根下一层含 `manifest.ini` 的文件夹，本目录没有 `manifest.ini`）。

## 它做什么

在 AstrBot 进程内采集：

| 数据 | 来源 |
| --- | --- |
| 在线 / 已就绪时长 / 版本 | 免鉴权 `GET /api/stat/start-time`、`GET /api/stat/versions` |
| 平台连接状态、断连原因、汇总 | `platform_manager.get_all_stats()` |
| CPU / 内存 / 线程数 | `psutil` |
| 消息总量与小时趋势 | `platform_stats` 表（`StatService` 拿不到时直接查库） |
| 实时日志（环形缓冲，最近 200 条） | 挂在 AstrBot 根 logger 上的限速 handler |
| Token 用量 | `provider_stats` 表 |
| 一键启停平台 | `platform_manager.reload()` + 写回 `cmd_config.json` |

采集结果按**快照契约 v1**（见同目录 `snapshot.schema.json`）每 5 秒 POST 给本机接收端；拿不到的项一律 `null`，不用 `0` 或空数组冒充。

插件不读取、不记录 AstrBot 的任何既有凭据（`passwordHash`、`jwt_secret`、`ws_reverse_token`、OpenAPI Key），也不调用 `/api/v1/system-config`。

## 手动安装

1. 找到 AstrBot 实例的数据目录：
   - Launcher 托管：`<AstrBot 数据根>\instances\<实例 ID>\core\data\plugins\`
   - 直接部署：`<AstrBot 根目录>\data\plugins\`
2. 把整个 `astrbot_plugin_desktop_status` 文件夹复制进去，目录名为 `astrbot_plugin_desktop_status`。
3. 重启 AstrBot，日志出现 `Loading plugin astrbot_plugin_desktop_status` 即加载成功。
4. 在 AstrBot 的插件配置里确认三项：
   - `push_url`：接收端地址，默认 `http://127.0.0.1:8765/v1/snapshot`（端口写 `0` 表示不推送）
   - `listen_port`：反向接口端口，默认 `8766`（写 `0` 表示不开）
   - `token`：留空则插件自动生成随机共享密钥并写回配置；接收端必须配置同一个值

## 对外接口（只监听 `127.0.0.1`，都要 `X-Status-Token`）

- `GET /v1/snapshot`：返回最新裸快照（接收端拉取兜底用）
- `POST /v1/actions`：`{"action":"set_enabled","id":"<platform>","enabled":true|false}`，写回 `cmd_config.json` 并立即生效

错误码：`401 unauthorized`、`400 invalid_action`、`405 method_not_allowed`、`404 not_found`、`409 unsupported_platform`（平台存在但不支持启停）、`503 service_unavailable`（插件已停止）。

## 安全边界

- 两个端口都只绑 `127.0.0.1`，不监听 `0.0.0.0`。
- token 用 `secrets.compare_digest` 比较，不写日志；共享密钥只存在插件配置与接收端本地文件里。
- 插件崩溃会拖累 AstrBot 主进程，因此每段采集都各自 try/except，单项失败只把该项置 `null`。

## 已知边界

- 成本金额：AstrBot 只返回 token 数，`cost.total` 在未配置单价表时为 `null`。
- 插件配置里的 `pricing` 是 JSON 文本（键含 `/`，AstrBot 的配置 schema 不能嵌套折叠）。
- 接收端实现这次不在本目录：Desktop Tools 侧的读取能力仍在接线中；没有接收端时推送失败只记一条告警，不影响 AstrBot 运行。
