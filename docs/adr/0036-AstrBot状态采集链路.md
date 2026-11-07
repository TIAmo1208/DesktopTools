---
status: accepted
---

# AstrBot 状态采集链路

## 背景

桌面「AstrBot 状态」格子现在只有一行字：控制台在不在、核心加载完成多久。用户需要平台连接状态、断连原因、CPU/内存、消息总量与小时趋势、实时日志、Token 用量。

这些数据有三条结构性阻断：

1. AstrBot dashboard REST 的 `system` scope 不在 `ALL_OPEN_API_SCOPES` 里，WebUI 创建 API Key 时没有该选项，所以 `/api/v1/stats`、`/logs/live` 等端点用任何 Key 都是 403。
2. 仓库里 AstrBot 没有 `CORSMiddleware`，响应没有 `Access-Control-Allow-*`；Desktop Tools 插件页 CSP 又是 `connect-src 'self'`，页内 `fetch` 与 `EventSource` 一律发不出去。
3. 插件槽一次只能声明一个宿主能力（ADR-0029），已有能力都是「宿主归一化后的最小结构」，没有任何一个能承载一份完整快照；新增能力又必须对数据源保持通用（见 [插件 loopback 通道](0038-插件loopback通道.md)）。

## 决策

不走 dashboard API，改为**在 AstrBot 进程内采集**，桌面侧经 [通用 loopback 通道](0038-插件loopback通道.md) 接收原始数据。

### 数据产生侧

- 提供一个 AstrBot Star 插件 `astrbot_plugin_desktop_status`，随 Desktop Tools 安装包放在 `plugin/AstrBot 状态/astrbot_plugin_desktop_status/`，由用户手动复制到 AstrBot 数据目录的 `data/plugins/`。
- 采集周期默认 5 秒；每轮采集服务、平台、资源、消息、日志、Token 用量，逐段 try/except，单项失败只把该项置 `null`，绝不把异常抛回 AstrBot 主循环。
- 插件只读免鉴权端点、`platform_manager`、`psutil` 与本地数据库；不读任何既有凭据，不调用 `/api/v1/system-config`。

### 数据契约

- 快照结构为契约 **v2**，字段集合与 `null` 语义以 `plugin/AstrBot 状态/astrbot_plugin_desktop_status/snapshot.schema.json` 为准：时间戳一律 Unix 秒，拿不到就是 `null`，不用 `0` 或空数组冒充。
- 契约没有 `receivedAt` 或任何传输字段：`sequence` 与 `receivedAt` 是宿主缓冲条目的元数据，不是快照的一部分。
- 宿主不解析也不校验这份 schema；字段合法性由桌面侧插件自己解析后兜底。

### 服务探测的四档口径（v2）

契约 v1 的 `service.online` 只有 `true` / `false` / `null`，装不下「探测被拒」。v2 因此新增 `service.probe`（对象，可为 `null`）：

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `service.probe.level` | `ok` / `warn` / `off` / `unknown` | 四档结论，与设计文档一一对应 |
| `service.probe.checkedAt` | Unix 秒整数 | 该次判定的时刻 |

- `ok` = 在线；`off` = 离线；`warn` = 探测被拒；`unknown` = 未知。四档必须各有独立文案，**未知不等于离线，探测被拒不等于服务停止**。
- `service.online` 保留，但降级为**派生/兼容字段**：采集侧按 `level === "ok"` 写 `true`，按 `level === "off"` 写 `false`，其余档写 `null`。四档展示一律以 `probe.level` 为准，前端不在两处各判一次。
- `service.probe` 整段为 `null` 时按「未知」处理；不得因为 `online` 为 `false` 就判离线。
- 采集侧的判定口径固定为：免鉴权探测返回 2xx 且能解析出对象 → `ok`；2xx 但响应解析失败 → `unknown`；`401` 或其它非 2xx → `warn`；连接被拒 → `off`；超时与其它未知异常 → `unknown`。传输层失败时采集侧再探一次 TCP 端口：端口没人（Windows 上连接被拒常表现为超时）判 `off`，端口有人但 HTTP 层仍失败判 `unknown`。
- `service.startTime` 只在 `ok` 时取值；探活成功但服务没给启动时刻时用进程创建时间兜底。
- 版本兼容：`schemaVersion` 从 `1` 升到 `2`。接收端与插件按版本分流——`1` 的数据视为没有 `probe`，按「未知」处理，不得当成离线；`2` 及以上按 `probe.level`。其它未识别版本整份按「未知」处理并记一次告警。

### 传输

- 推送：插件按周期 `POST http://127.0.0.1:8765/v1/snapshot`。
- 拉取与反向动作：插件在 `127.0.0.1:8766` 暴露 `GET /v1/snapshot` 与 `POST /v1/actions`（启停平台）。启停会写回 `cmd_config.json` 再 `platform_manager.reload()`，重启后仍生效。
- 双向共用 `X-Status-Token`（插件配置里自动生成的随机本机密钥，与 AstrBot 凭据无关），比较用 `secrets.compare_digest`，不写日志；两个端口都只绑 `127.0.0.1`。
- 错误码：`401`、`400`、`405`、`404` 为必需码；`409 unsupported_platform`、`503 service_unavailable` 是登记在案的扩展码，不改变必需码语义。

### 桌面侧：两条通道都归插件

- 「AstrBot 状态」插件清单只声明 `loopback-channel`，不再声明 `probe-service-status`；六类格子全部走通道，不保留探活旧路径。
- 插件注册两条通道：`mode=listen` 的 `8765` 接推送，`mode=request` 的 `http://127.0.0.1:8766` 用于拉取兜底与平台启停。
- 两条通道都声明 `credentialRef` 与 `headerName = X-Status-Token`；宿主从 Windows Credential Manager 读密钥，入站自动校验、出站自动注入，插件拿不到明文。
- 平台启停走 `request_loopback` 的 `POST /v1/actions`，不新增写能力声明。

### 插件内共享

- `snapshot.schema.json` 的解析、字段兜底、缓存全在插件内；模块级单例缓存，同一插件的多个类型实例共用一份快照、一条通道、一个定时器。
- 轮询与订阅按 `instanceId` 去重：通知到达时由持有缓存的模块读一次 `read_loopback_channel`，各类型实例从缓存取，不按类型各开一条读取通道。
- 过期判定分两层，都归插件：
  - **接收判定**：`receivedAt - collectedAt` 超过 30 秒的推送不进入缓存（数据到得太晚，当作坏数据）；
  - **显示判定**：最近一次有效快照的 `collectedAt` 超过 60 秒即判过期，过期时不显示旧时长。
- 宿主侧缓冲只按条数与时间清理（见 ADR-0038），不承担任何业务过期判定。

## 边界

- 宿主只提供通道能力，不认识 AstrBot：不做 dashboard REST 客户端、不校验快照 schema、不做 `system` scope 的 JWT 方案、不做页内直连、不做多实例聚合、不做历史库与告警推送渠道。多实例聚合与历史库若以后要做，属于插件侧与新一轮契约的事。
- 成本金额：AstrBot 只返回 token 数，`cost.total` 在未配置单价表时为 `null`，不写死单价。
- 端口占用、凭据缺失、对端 `401` 的失败呈现按 ADR-0038 的结构化错误交插件处理，宿主不代劳。
- 推送通道绑定 8765 失败（端口被占用）时插件只保留 8766 拉取兜底并给出可见提示，不等同于数据过期。

## 格子侧的已确认设计

按功能拆格子（ADR-0037 的类型机制）与各类型的呈现已在设计稿中确认，细则见 [AstrBot 状态格子](../design/AstrBot状态格子.md)。与本决策相关的约束：

- 六个类型（状态、就绪时长、平台连接、资源占用、消息量、运行日志）共用同一份插件内快照缓存，轮询按实例去重；不按类型各开一条读取通道。
- 运行日志以时间轴呈现，节点来自 `logs.tail`；`collectedAt` 超过 60 秒判过期，过期时不显示旧时长。
- 版本字段（`instance.astrbotVersion`、`service.versions`）不进入任何格子类型，只在插件设置二级页展示。
- 接线完成前不得把平台连接、资源占用、消息量、运行日志实现为占位格子。

## 待验证与待完成

- 平台真实断连时的 `error` 状态与 `lastError` 只有单元测试覆盖，未做真机注入。
- 端口占用目前靠人工注入验证（监听侧必须绑定失败而不是抢占端口），尚未加自动化回归用例。
- 契约 v2 与 `service.probe` 已在采集侧落地；通道能力、插件自解析与六类格子的渲染只有单元测试与契约型假宿主覆盖，尚未在真实桌面上验收。
- 采集侧的四档判定只由自身自检覆盖，未在真实的 AstrBot 实例上跑出过 `warn` 与 `off` 两档。
- v1 与 v2 混合期间的版本分流只有文档口径与插件侧解析测试，未在真实对端上验证。
