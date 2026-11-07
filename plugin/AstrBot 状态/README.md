# AstrBot 状态

在桌面上查看本机 AstrBot 的六种信息：服务状态、就绪时长、平台连接、资源占用、消息量与运行日志。一个插件声明六种格子类型，创建时选择一种；一格只放一件事。

## 数据从哪里来

- 插件清单只声明 `loopback-channel` 能力。数据不经过页面直连，也不由宿主解析。
- 启动后按当前实例注册两条通道：
    - `astrbot-push`：`mode=listen`，宿主监听 `127.0.0.1:8765`，接住 AstrBot 采集插件的快照推送。
    - `astrbot-request`：`mode=request`，宿主向 `http://127.0.0.1:8766` 发请求，用于拉取兜底与平台启停。
- 两条通道都声明 `credentialRef = DesktopTools/astrbot-status` 与 `headerName = X-Status-Token`；密钥由宿主从 Windows Credential Manager 读取并注入/校验，插件拿不到明文。
- 插件自解析快照 v2（v1 视为没有 `probe`，按未知处理），在模块级单例里缓存；同一插件的多个类型实例共用一份快照、一条通道与一个定时器，按 `instanceId` 去重。
- 快照解析、字段兜底、过期判定都在本插件内：推送到达比采集时刻晚超过 30 秒不进缓存；最近一次快照的 `collectedAt` 超过 60 秒判为过期。

## 六种格子

| 键 | 内容 | 默认尺寸 | 最小尺寸 |
| --- | --- | --- | --- |
| `status` | 四档结论（在线 / 离线 / 探测被拒 / 未知）与就绪时长 | 168 × 170 | 168 × 114 |
| `uptime` | 一行大号就绪时长 | 168 × 114 | 168 × 114 |
| `platforms` | 平台汇总、逐平台状态、运行时长与错误次数 | 288 × 300 | 288 × 200 |
| `resources` | CPU、进程内存、线程数 | 220 × 210 | 220 × 150 |
| `messages` | 今日合计与 24 点柱状 | 420 × 300 | 420 × 210 |
| `logs` | ERROR / WARNING 时间轴 | 420 × 300 | 420 × 200 |

- 四档语义只看 `service.probe.level`；`service.online` 只是采集侧派生的兼容字段，不用于展示。
- 数据过期时状态点压灰并标「数据过期」，就绪时长显示「—」，不沿用旧时长。
- 版本字段（`instance.astrbotVersion`、`service.versions`）不进入任何格子。

## 交互

- 格子内只读，动作都在右键菜单：打开 AstrBot 控制台、刷新一次、复制状态摘要；平台连接格在有出错平台时追加启停项。
- 启停走 `request_loopback` 的 `POST /v1/actions`，成功后立即重读快照；请求进行中菜单项禁用。

## 配套采集插件

AstrBot 进程内的 `astrbot_plugin_desktop_status` 随仓库放在同目录下，需要手动复制到 AstrBot 数据目录的 `data/plugins/`。它负责采集并推送到 8765、在 8766 提供拉取与启停接口。
