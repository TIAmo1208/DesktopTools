# AstrBot 状态采集插件（AstrBot 侧）

这是给 **AstrBot（Star 插件）** 用的插件包，不是 Desktop Tools 的格子插件。它随 Desktop Tools 安装包一起分发，放在这里是为了让用户「手动安装」时有现成文件，Desktop Tools 运行时**不会**扫描本目录（宿主只认插件根下一层含 `manifest.ini` 的文件夹，本目录没有 `manifest.ini`）。

> 从安装到桌面出数据、包括怎么取 token 和怎么排错，见 [AstrBot 状态插件安装与配对](../AstrBot状态插件安装与配对.md)。本页只讲采集插件自身。

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

采集结果按**快照契约 v2**（见同目录 `snapshot.schema.json`）每 5 秒 POST 给本机接收端；拿不到的项一律 `null`，不用 `0` 或空数组冒充。

## 安装

### 方式一：Web 插件管理上传

在 AstrBot WebUI →「插件」→「安装插件」里上传本插件目录的压缩包，列表出现 **astrbot_plugin_desktop_status** 即装好。

### 方式二：手动复制文件夹

1. 找到 AstrBot 实例的数据目录：
   - Launcher 托管：`<AstrBot 数据根>\instances\<实例 ID>\core\data\plugins\`
   - 直接部署：`<AstrBot 根目录>\data\plugins\`
2. 把整个 `astrbot_plugin_desktop_status` 文件夹复制进去，目录名保持 `astrbot_plugin_desktop_status`。
3. 重启 AstrBot，日志出现 `Loading plugin astrbot_plugin_desktop_status` 即加载成功。

## 配置项

| 配置项 | 默认值 | 说明 |
| --- | --- | --- |
| `enabled` | `true` | 总开关。关闭后不采集、不推送、不开反向端口。 |
| `collect_interval_sec` | `5` | 采集周期（秒），同一周期内既采集快照又推送。 |
| `push_url` | `http://127.0.0.1:8765/v1/snapshot` | 推送目标；端口写 `0` 表示不推送。 |
| `listen_port` | `8766` | 反向端口，只监听 `127.0.0.1`；`0` 表示不开。 |
| `token` | 空 | 双向共享密钥，**后台直接显示、可复制**（不是隐藏项）；把该值粘贴到 Desktop Tools「设置 → 功能格子 → AstrBot 状态」的共享 token，两侧必须一致。留空时自动生成随机值并写回配置。 |
| `log_tail` | `200` | 内存环形缓冲条数，不落盘。 |
| `error_keywords` | `Traceback`/`ERROR`/`Exception`/`failed` | 告警匹配词，只写进插件日志，不改快照字段。 |
| `pricing` | 空 | 单价表（JSON 文本），留空时 `cost.total` 为 `null`。 |

## 两路通信

- **推送**：按 `collect_interval_sec` 周期 `POST push_url`，是数据主路径。
- **反向接口**：在 `listen_port` 上提供拉取兜底与平台启停，供 Desktop Tools 主动读取。

两路都只监听 `127.0.0.1`，都要带 `X-Status-Token` 头；桌面侧通过宿主的 loopback 通道连这两路，通道建立与凭据注入都由 Desktop Tools 负责，本插件不关心是谁在调用。

## 对外接口

- `GET /v1/snapshot`：返回最新裸快照（接收端拉取兜底用）。
- `POST /v1/actions`：`{"action":"set_enabled","id":"<platform>","enabled":true|false}`，写回 `cmd_config.json` 并立即生效。

错误码：`401 unauthorized`、`400 invalid_action`、`405 method_not_allowed`、`404 not_found`、`409 unsupported_platform`（平台存在但不支持启停）、`503 service_unavailable`（插件已停止）。

## service 字段语义（v2）

免鉴权探测 `GET /api/stat/start-time` 的判定口径固定为四档：

| `probe.level` | 触发条件 | 含义 |
| --- | --- | --- |
| `ok` | 2xx 且响应可解析 | 在线 |
| `warn` | `401` 或其它非 2xx | 探测被拒，**不等于服务停止** |
| `off` | 连接被拒（端口没人） | 离线 |
| `unknown` | 超时、响应解析失败或其它未知异常 | 未知，**不等于离线** |

- `service.probe` 为 `{level, checkedAt}`，`checkedAt` 是该次判定的 Unix 秒；整段为 `null` 时按「未知」处理。
- `service.online` 保留但只是派生/兼容字段：`level=ok` 写 `true`，`level=off` 写 `false`，`warn`/`unknown` 写 `null`。四档展示一律以 `probe.level` 为准，不得只看 `online`。
- `service.startTime` 仅 `probe.level=ok` 时有值；探测成功但服务没给启动时刻时用进程创建时间兜底。
- 版本兼容：`schemaVersion` 为 `2`；读取方遇到 `1` 的数据视为没有 `probe`，按「未知」处理，不得当成离线。

## 安全边界

- 插件不读取、不记录 AstrBot 的任何既有凭据（`passwordHash`、`jwt_secret`、`ws_reverse_token`、OpenAPI Key），也不调用 `/api/v1/system-config`。
- 两个端口都只绑 `127.0.0.1`，不监听 `0.0.0.0`。
- token 用 `secrets.compare_digest` 比较，不写日志；共享密钥只存在插件配置与接收端的 Windows 凭据管理器里。
- 插件崩溃会拖累 AstrBot 主进程，因此每段采集都各自 try/except，单项失败只把该项置 `null`。

## 已知边界

- 成本金额：AstrBot 只返回 token 数，`cost.total` 在未配置单价表时为 `null`。
- 插件配置里的 `pricing` 是 JSON 文本（键含 `/`，AstrBot 的配置 schema 不能嵌套折叠）。
- 桌面侧没有接收端时推送失败只记一条告警，不影响 AstrBot 运行。
