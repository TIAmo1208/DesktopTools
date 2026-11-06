---
status: accepted
---

# AstrBot 状态采集链路

## 背景

桌面「AstrBot 状态」格子现在只有一行字：控制台在不在、核心加载完成多久。用户需要平台连接状态、断连原因、CPU/内存、消息总量与小时趋势、实时日志、Token 用量。

这些数据有三条结构性阻断：

1. AstrBot dashboard REST 的 `system` scope 不在 `ALL_OPEN_API_SCOPES` 里，WebUI 创建 API Key 时没有该选项，所以 `/api/v1/stats`、`/logs/live` 等端点用任何 Key 都是 403。
2. 仓库里 AstrBot 没有 `CORSMiddleware`，响应没有 `Access-Control-Allow-*`；Desktop Tools 插件页 CSP 又是 `connect-src 'self'`，页内 `fetch` 与 `EventSource` 一律发不出去。
3. 插件槽一次只能声明一个宿主能力（ADR-0029），已有 `probe-service-status` 无法扩展成多字段聚合。

## 决策

不走 dashboard API，改为**在 AstrBot 进程内采集**：

### 数据产生侧

- 提供一个 AstrBot Star 插件 `astrbot_plugin_desktop_status`，随 Desktop Tools 安装包放在 `plugin/AstrBot 状态/astrbot_plugin_desktop_status/`，由用户手动复制到 AstrBot 数据目录的 `data/plugins/`。
- 采集周期默认 5 秒；每轮采集服务、平台、资源、消息、日志、Token 用量，逐段 try/except，单项失败只把该项置 `null`，绝不把异常抛回 AstrBot 主循环。
- 插件只读免鉴权端点、`platform_manager`、`psutil` 与本地数据库；不读任何既有凭据，不调用 `/api/v1/system-config`。

### 数据契约

- 快照结构固定为契约 v1，字段集合与 `null` 语义以 `plugin/AstrBot 状态/astrbot_plugin_desktop_status/snapshot.schema.json` 为准：时间戳一律 Unix 秒，拿不到就是 `null`，不用 `0` 或空数组冒充。
- 接收端以 `receivedAt` 与 `collectedAt` 现算新鲜度，超过 30 秒判过期；旧快照仍可读，但不得当成在线状态展示。

### 传输

- 推送：插件按周期 `POST http://127.0.0.1:8765/v1/snapshot`。
- 拉取与反向动作：插件在 `127.0.0.1:8766` 暴露 `GET /v1/snapshot` 与 `POST /v1/actions`（启停平台）。启停会写回 `cmd_config.json` 再 `platform_manager.reload()`，重启后仍生效。
- 双向共用 `X-Status-Token`（插件配置里自动生成的随机本机密钥，与 AstrBot 凭据无关），比较用 `secrets.compare_digest`，不写日志；两个端口都只绑 `127.0.0.1`。
- 错误码：`401`、`400`、`405`、`404` 为必需码；`409 unsupported_platform`、`503 service_unavailable` 是登记在案的扩展码，不改变必需码语义。

## 边界

- Desktop Tools 侧**尚未**新增宿主能力读取这份数据：格子仍只显示在线/就绪时长，`probe-service-status` 不变。接线需要按 ADR-0029 的 6 处影响面新增或替换能力，届时插件槽的单能力约束决定它是替换 `probe-service-status` 而不是并存。
- 接收端实现不在本决策范围内落库：本轮验证用的独立守护进程是临时手段，正式实现随宿主接线决定（宿主进程内接收，或随包发布独立守护进程）。
- 不做：dashboard REST 客户端、`system` scope 的 JWT 方案、页内直连、多实例聚合、历史库与告警推送渠道。
- 成本金额：AstrBot 只返回 token 数，`cost.total` 在未配置单价表时为 `null`，不写死单价。

## 待验证与待完成

- 平台真实断连时的 `error` 状态与 `lastError` 只有单元测试覆盖，未做真机注入。
- 端口占用目前靠人工注入验证（守护进程必须绑定失败而不是抢占端口），尚未加自动化回归用例。
- 宿主接线与格子 UI（表格、趋势图）未做。
