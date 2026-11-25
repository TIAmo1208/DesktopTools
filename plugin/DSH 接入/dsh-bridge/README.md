# dsh-desktop-bridge

DSH Host 只读桥插件。它在 `127.0.0.1` 上暴露只读快照服务，供 DesktopTools 的 Rust 适配层读取 **余额、会话、job、待决摘要**；不认领审批、不代批、不写 DSH 状态。

- 不注册审批应答者，不参与任何裁决；`approval/asked` 只用于把待决项标进快照。
- 不调用任何 DSH 写方法（命令执行、会话追加、job 控制等）。
- 不读 DSH 状态文件、数据库或浏览器 Cookie。
- 服务只绑 `127.0.0.1`；配对码只出现在 DSH 同源配对页，**不写日志、不在 47832 暴露**。

## 接口

| 方法 | 路径 | 说明 |
|---|---|---|
| `GET` | `/health` | `{ ok, protocolVersion, bridgeId, bridgeInstanceId, since }`，不需要凭据 |
| `POST` | `/pair` | `Content-Type: application/json`，body `{ "code": "XXXXX-XXXXX-XXXXX-XXXXX" }`；成功 `{ ok, token }`；码错/过期/已用 `401`；非 JSON `415` |
| `GET` | `/snapshot` | 头 `Authorization: Bearer <token>`；返回快照；无/错 token `401` |
| 其它方法 | 上述路径 | `405`（带 `Allow`） |
| 其它路径 | — | `404` |

边界约束：

- 请求 `Host` 必须精确等于 `127.0.0.1:<实际端口>`，否则 `421`（防止把 47832 当成别的服务）。
- 只接受本机来源；不返回重定向、不设置 Cookie。

快照字段（协议版本 1）：

```json
{
  "protocolVersion": 1,
  "bridgeId": "…",
  "bridgeInstanceId": "…",
  "revision": 42,
  "fetchedAt": "2026-11-19T09:30:00.000Z",
  "stale": false,
  "balance": { "status": "ready|logged-out|unauthorized|error", "wallets": [{ "currency": "CNY", "amount": "12.34" }] },
  "sessions": [{ "id": "…", "title": "…", "status": "running|idle|waiting", "pending": "approval|question|plan|none" }],
  "jobs": [{ "id": "…", "label": "…", "status": "running|done|failed" }],
  "pending": [{ "id": "…", "kind": "approval|question|plan", "sessionId": "…", "since": "…" }],
  "taskStats": {
    "runCount": 1,
    "unreadCount": 2,
    "completedUnreadCount": 1,
    "unionCount": 2,
    "completeness": "complete|partial",
    "sampledAt": "2026-11-19T09:30:00.000Z"
  }
}
```

- `bridgeId` 是稳定桥身份，首次启动生成后写入 `ctx.credentials`（`dsh-desktop-bridge/identity`），跨重启不变；DesktopTools 应把已存 token 绑定到它。
- `bridgeInstanceId` 每次启动随机；用于作废旧实例快照与在途请求。
- `revision` 覆盖全部展示事实：余额结果变化、待决增删、会话/job 变化都会推进；`fetchedAt` 只是本次采样时间，不承担版本语义。
- `stale` 在会话/job 聚合失败，或余额因平台错误/超时而陈旧时为 `true`；`balance.status` 为 `error` 时不要把余额当 0。
- `balance.status`：未登录 → `logged-out`；凭据存在但平台拒绝/过期 → `unauthorized`；服务缺失、平台失败或超时 → `error`。
- `sessions.pending` 与 `pending[]` 的 `question`/`plan` 由工具事件推断；`approval` 来自 `approval/asked`/`approval/decided` 审计事件；只展示，不提供批准入口。
- 生产合同与 Rust 对齐：`sessions ≤ 64`、`jobs ≤ 32`、`pending ≤ 32`、`wallets ≤ 16`，标题/名称按字符截断到 256，id 超 128 字符或含控制字符的条目会被丢弃；活待决优先保留。
- 会话/job 缓存有界（会话保留 256、job 保留 128，超出回收已结束对象）；job 枚举只扫活跃/最近 16 个会话。
- 部分失败如实表达：会话/job 读取失败时 `stale=true`，不把“不可知”伪装成“没有任务”；余额失败为 `balance.status=error`。
- 本轮不实现 AgentTeams 任务板（M5），只保留会话与 job 状态。
- `taskStats` 可选：任务单位是**可见会话**（最多 64 条）。
  - `runCount`：`status=running` 或含 running job 的会话数（按会话去重，一个会话多个 job 只计一次）。
  - `unreadCount`：自该会话最近一次**直接用户消息**（`user/message` 且 `source.kind=user`）以来发生过 `turn/end`、`job/settled` 或待决到达（`approval/asked`、`ask_user_question`、`exit_plan_mode`）的会话数；这是 bridge-defined 未读代理，不是 DSH 的已读状态。`forked`/`interrupted` 的 `turn/end` 不算完成。
  - `completedUnreadCount`：上述会话中最近一次完成（`turn/end` 或 `job/settled`）晚于 cutoff 的会话数，是 `unreadCount` 的子集。
  - `unionCount`：running 与 unread 的会话并集，恒有 `completedUnread ≤ unread ≤ union ≤ run + unread`。
  - job 归属按**全部可见会话**（≤ 64）逐会话查询，不只看前若干个；某会话多个 running job 只计一次。
  - 三个水位（cutoff / 未读 / 完成）都取 max：乱序或迟到的旧事件不会把水位往回拨，`completedUnread` 恒为 `unread` 的子集。
  - 水位只在会话「既不在事件缓存、也不在 `sessions.list`」时清理，且只以**成功读取的未截断源 ID**为权威：展示截断（≤64）与 `sessions.list` 抛错都不删水位。
  - `running` 集合按「最终 job 状态唯一 map」重建（`job/registered` 之后会话读到 done 不会把会话加回 running），也不受展示截断（jobs ≤32）影响：计数用完整源，展示才截断。
  - `completeness`：会话来源正常且未陈旧为 `complete`；会话/job 读取失败等陈旧情况为 `partial`，消费方应忽略计数并降级。
  - 没有任何会话来源时**省略** `taskStats`，不用全 0 冒充“没有任务”；`sampledAt` 不参与 digest，不会每轮推进 `revision`。

## 受信入口（DSH 设置页）

插件包带 `lib/client.js`（`dsh.client` 声明），在 DSH 设置页注册一个「DSH 桌面桥」分区：

- 显示桥状态（已配对 / 未配对）与 `bridgeId` 前 8 位；
- 「DSH 桌面桥」设置分区**内嵌**显示配对码、剩余时间、轮换与撤销按钮，不再跳转整页；配对页 `/dsh-bridge/pairing` 保留为同源兜底。
- 状态来自同源只读接口 `/dsh-bridge/status`（owner 认证、`no-store`、不返回配对码）。
- 内嵌面板使用 `/dsh-bridge/pairing.json`：`GET` 返回 `{ paired, bridgeId, code, expiresAt, csrf }`（已配对时 `code` 为 `null`），`POST` 用同一 `csrf` 执行 `rotate` / `revoke`；三条同源路由都要求 owner 身份，并要求带 `Origin` 的请求与请求 `Host` 同源。

如果当前 profile 没有加载客户端插件（例如无 Web 组合），仍可在 DSH 窗口直接打开 `/dsh-bridge/pairing`；这是降级路径，不是主入口。

## 配对页（同源兜底，需要 DSH 窗口身份）

主入口是上面的设置分区（走 `/dsh-bridge/pairing.json` 取码）；本页只在客户端分区不可用时兜底。`/dsh-bridge/status` 刻意不返回配对码，取码只走设置分区的 `pairing.json` 或本兜底页。

配对码只在 DSH Web 服务的同源页面展示：

```text
/dsh-bridge/pairing
```

在 DSH 窗口中打开该路径（同一 Web GUI 源；页面需要 DSH 的浏览器会话 Cookie）。处理器先要求 owner 已认证，未认证返回 `401`，组合缺少 Connection 服务时返回 `503`。页面提供：

- 当前配对码与到期时间（未配对时）；
- 「轮换配对码」按钮（已配对时返回 `409`，需先撤销）；
- 「撤销已配对」按钮：清除 token 记录，旧 Bearer 立即 `401`。

页面不返回 JSON、不提供“取当前码”的 API，也不设置 Cookie。日志只记「配对就绪/已过期」和不带码的页面地址：

```text
[dsh-desktop-bridge] pairing ready; open /dsh-bridge/pairing in the DSH window (code expires in 600s)
[dsh-desktop-bridge] pairing code expired; open /dsh-bridge/pairing to get a new one
```

## 配对与凭据

1. 插件启动时生成一次性配对码（96 bit，形如 `ABCDE-FGHIJ-KLMNO-PQRST`），默认 10 分钟有效；码的 SHA-256 哈希写入 `ctx.credentials`（键 `dsh-desktop-bridge/pairing-code`）。
2. `POST /pair` 走单次串行队列：读码 → 校验 → 消费码 → 签发 token → 写 token 哈希。并发请求最多一个成功；消费/签发失败返回 `500`，不会静默回退到内存存储，也不会先返回 `200`。
3. 成功后 token 只返回一次；服务端只保存其 SHA-256 哈希（键 `dsh-desktop-bridge/pairing-token`）。之后 `bridgeId` 可让 DesktopTools 识别同一个桥。
4. 撤销：配对页「撤销已配对」，或删除 `dsh-desktop-bridge/pairing-token` 记录。撤销后旧 Bearer 立即 `401`，并生成新的配对码。
5. 重启插件不会让已配对 token 失效（只有 `bridgeInstanceId` 变，DesktopTools 据此作废旧快照）；桥身份 `bridgeId` 保持不变。

## 配置

`cordis.patch.yml` 已声明默认值：

```yaml
- insert:
    - id: dsh-desktop-bridge
      name: 'dsh-desktop-bridge'
      config:
        port: 47832
        pairingCodeTtlMs: 600000
        rotateCheckMs: 15000
        balanceCacheMs: 5000
        balanceTimeoutMs: 3000
```

- `port`：默认 `47832`；**端口被占用时插件以明确错误失败**，不会静默换端口。`0` 表示系统分配（仅建议自测使用）。
- `pairingCodeTtlMs`：配对码有效期（毫秒），默认 10 分钟。
- `rotateCheckMs`：过期检查间隔（毫秒），默认 15 秒。
- `balanceCacheMs`：余额结果缓存时长，默认 5 秒。
- `balanceTimeoutMs`：平台余额调用超时，默认 3 秒；超时只让余额报 `error`/`stale`，不阻塞整份快照。

配置是 profile patch 层内容，请通过 DSH 插件设置/配置入口修改，不要在这里写任何密钥。

## 安装（说明，未在本任务中执行）

1. 把 `dsh-bridge/` 作为本地插件包加入目标 profile：在 DSH 插件页选择本地目录安装，或对目标 profile 执行本机 `dsh plugin add <路径>`（不同 DSH 版本命令名可能不同，以当前版本 `dsh plugin --help` 为准）。
2. 重新加载插件/重启 DSH；Host 日志出现 `listening on 127.0.0.1:47832` 与配对页地址（不含码）。
3. 打开 DSH 设置页，进入「DSH 桌面桥」分区，配对码直接显示在分区里（含剩余时间、换码与撤销按钮）；分区不可用时才在同窗口打开 `/dsh-bridge/pairing` 兜底。
4. 复制当前一次性配对码到 DesktopTools，取得 token 后由 Rust 侧存 Windows Credential Manager，再轮询 `/snapshot`。
5. 确认 `/health` 可访问、`/snapshot` 未带 token 返回 401；不要在此步骤中触发任何真实审批。

本仓库不自动安装该插件，也不修改用户 `.dsh` profile。

## 生命周期

- 插件 `apply` 启动服务、注册同源配对页并订阅 `session/event`、`agent/status`；disposer 先注销订阅与页面路由，再关闭 HTTP 服务并清理配对码轮换定时器。
- 宿主重启后 `bridgeInstanceId` 变化、`bridgeId` 不变；旧实例的快照全部作废（DesktopTools 按 `bridgeInstanceId` 丢弃缓存）。
- 服务不在宿主退出后残留：`server.close()` 同时断开空闲连接。

## 自测

```bash
node dsh-bridge/自测.mjs
node --check dsh-bridge/lib/index.js
node --check dsh-bridge/自测.mjs
```

自测使用假 ctx 桩（凭据、会话、job、余额、事件、配对页 webServer、client loader），覆盖：health/pair/snapshot/401/405/404、并发同一码最多一次 200、过期 401、撤销后旧 Bearer 401、凭据层故障 500（无假成功）、错误 Host 421、非 JSON 415、稳定 bridgeId、revision 覆盖余额/待决/job、余额缓存/在途合并/慢平台超时、配对页与状态接口 owner 认证/轮换/撤销/无 Cookie、生产合同条数与长度截断、部分失败 `stale`、job 扫描有界、client 设置分区注册。不接触真实 DSH profile、不注册任何 answerer。

## 已知限制

- 配对页与设置分区依赖 DSH Web 服务的 `webServer` 与 `connection` 服务；无头/无 Web 组合下页面与分区不可用，也不会退回日志展示配对码（宁可不可配对，不泄露码）。
- 快照不包含 `bonusWallets`（spec 只定义 `wallets`）；如需要，先改 spec 再改实现。
- `pending` 的 `question`/`plan` 是工具事件推断，不是 DSH 权威待决注册表；`approval` 以审计事件为准。
- 协议版本固定为 `1`；字段变更必须升 `protocolVersion`，DesktopTools 按不支持则收起入口处理。
- 本轮不实现 AgentTeams 任务板（ADR 由维护方同步）。
