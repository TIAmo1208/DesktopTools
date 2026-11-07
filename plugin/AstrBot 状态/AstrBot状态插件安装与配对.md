# AstrBot 状态插件安装与配对

这份文档讲怎么把「AstrBot 状态」用起来：从安装 AstrBot 侧的采集插件，到桌面上的六个格子出数据。涉及两个独立程序，缺一不可：

- **AstrBot 采集插件** `astrbot_plugin_desktop_status`：跑在 AstrBot 进程里，负责采数据并按契约 v2 推送；
- **Desktop Tools 的「AstrBot 状态」格子插件**：跑在桌面工作区里，负责接收、解析、缓存并画格子。

两者靠一把**本机共享 token** 配对，不经过任何外部服务。

---

## 1. 安装 AstrBot 采集插件

### 方式一：Web 插件管理上传

1. 打开 AstrBot 的 WebUI →「插件」→「安装插件」。
2. 选择 `astrbot_plugin_desktop_status.zip` 上传：
   - **已安装 Desktop Tools**：zip 随安装包输出在安装目录下，取 `<Desktop Tools 安装目录>/plugin/AstrBot 状态/astrbot_plugin_desktop_status.zip`（例如 `C:\\Program Files\\Desktop Tools\\plugin\\AstrBot 状态\\astrbot_plugin_desktop_status.zip`）。
   - **从仓库开发**：先在仓库根目录运行 `scripts/pack-astrbot-plugin.ps1`，产物在 `plugin/AstrBot 状态/astrbot_plugin_desktop_status.zip`。
3. 上传完成后，插件列表里出现 **astrbot_plugin_desktop_status**，版本号与包内 `metadata.yaml` 一致。

### 方式二：手动复制文件夹

1. 找到 AstrBot 实例的数据目录：
   - Launcher 托管：`<AstrBot 数据根>\instances\<实例 ID>\core\data\plugins\`
   - 直接部署：`<AstrBot 根目录>\data\plugins\`
2. 把整个 `astrbot_plugin_desktop_status` 文件夹复制进去，目录名保持 `astrbot_plugin_desktop_status`。
3. 重启 AstrBot。

### 加载成功的标志

AstrBot 日志出现 `Loading plugin astrbot_plugin_desktop_status`，插件列表里该插件处于启用状态。若日志里没有这一行，检查目录名是否写错、是否多套了一层文件夹。

---

## 2. 配置项

在 AstrBot 的插件配置里，这几项与本安装相关：

| 配置项 | 默认值 | 含义 |
| --- | --- | --- |
| `enabled` | `true` | 总开关。关闭后不采集、不推送、不开反向端口。 |
| `collect_interval_sec` | `5` | 采集周期（秒）。同一周期内既采集快照又推送。 |
| `push_url` | `http://127.0.0.1:8765/v1/snapshot` | 推送目标。端口写 `0` 表示不推送。 |
| `listen_port` | `8766` | 插件反向端口，只监听 `127.0.0.1`。写 `0` 表示不开。 |
| `token` | 空 | 双向共享密钥。留空时插件首次启动自动生成随机值并写回配置；与 AstrBot 的任何既有凭据无关，日志里不会输出。 |

其余项（`log_tail`、`error_keywords`、`pricing`）只影响采集内容，不影响通信，可以保持默认。

### 两条通道与四档探测的关系

桌面侧经宿主的 loopback 通道连两条通道，各管一件事：

- **推送通道**（桌面侧 `mode=listen` 绑 `127.0.0.1:8765`）：AstrBot 按 `collect_interval_sec` 周期 POST 快照，是数据的主路径。
- **拉取通道**（桌面侧 `mode=request` 指向 `http://127.0.0.1:8766`）：调 `GET /v1/snapshot` 做拉取兜底，也用来 `POST /v1/actions` 启停平台。

快照契约 v2 里，`service.probe.level` 有四档，由采集侧判定：`ok` 在线、`off` 离线、`warn` 探测被拒、`unknown` 未知。**这与通道是两回事**——通道通不通决定「有没有数据」，`probe.level` 决定 AstrBot 服务本身在不在。可能出现「数据正常推送但状态格显示探测被拒」的情况，这不是插件坏了。

---

## 3. 取共享 token

1. 打开 AstrBot WebUI 的「插件」→ astrbot_plugin_desktop_status → 配置页。
2. 找到 `token` 项，复制它的值。若为空，保存一次配置或重启 AstrBot，插件会自动生成并写回。
3. 这把 token 只在本机的 AstrBot 与 Desktop Tools 之间使用。

---

## 4. 在 Desktop Tools 里配对

1. 桌面右键 →「添加 …」或设置页，创建至少一个「AstrBot 状态」格子（六个类型任选其一；数据是共享的，建一个即可）。
2. 打开「设置 → 功能格子 → AstrBot 状态」二级页。
3. 在「共享 token」输入框里粘贴上一步复制的值，点「保存」。
4. 看到「已保存到 Windows 凭据管理器，页面不会回显 token」即成功。**保存后立即重连，不需要重启 Desktop Tools，也不需要切换格子。**

配对值存在 Windows 凭据管理器的 `DesktopTools/astrbot-status` 记录里，页面不会回显已保存的值；要换 token 就重新粘贴保存，要断开就点「清除」。

---

## 5. 验证：六类格子各看到什么

| 类型 | 正常情况下看到 |
| --- | --- |
| 状态 | 一行结论：状态点 + 「在线 / 离线 / 探测被拒 / 未知」+ 就绪时长 |
| 就绪时长 | 一行大号时长，如「已就绪 2 小时 13 分」 |
| 平台连接 | 汇总行「平台 3 / 4 在线」+ 逐平台状态与运行时长，有错误时带「错误 ×N」 |
| 资源占用 | CPU 百分比 + 细横条、进程内存 + 细横条、线程数 |
| 消息量 | 「今日 1,284」+ 24 点小时柱状 |
| 运行日志 | 时间轴上的 ERROR / WARNING 节点，最近的在最上 |

### 四种异常状态怎么区分

| 看到什么 | 含义 | 该怎么办 |
| --- | --- | --- |
| 「未配对」+ 配对指引 | 桌面侧读不到共享 token（注册返回 `loopback-credential-unavailable`）。两条通道共用一把 token，进入该状态后不再重复尝试注册。 | 按第 4 节粘贴 token 并保存。保存后会自动重试。 |
| 「token 无效」+ 重新保存提示 | 已配对，但对端明确拒绝了这把 token（`401`）。 | 去 AstrBot 配置页重新复制 `token`，在设置页重新粘贴保存。 |
| 六类格子都显示「暂无数据」/「平台清单暂不可用」，状态格还补一行「采集插件未上报或未安装」 | 已配对，但收不到任何快照。 | AstrBot 没运行、采集插件没启用、或端口没通，见第 6 节。 |
| 状态格压成灰色未知、数值停在最后一次 | 最近一次采集超过 60 秒没更新。格子保留最后内容，不显示过期标记。 | 检查 AstrBot 是否卡住、`enabled` 是否被关掉、端口是否被占用。 |

> 未配对、token 无效、无数据是三种不同状态；数据停止更新时保留最后一次内容、状态格压成未知，格子不会把任何一种画成「离线」。

---

## 6. 排错

### 8765 端口被占用

表现：桌面侧日志出现 `loopback-port-in-use`，推送收不到，但格子最终仍能出数据（靠拉取兜底）。

原因：`127.0.0.1:8765` 已被别的程序占用，宿主不会抢占端口、也不会改用其它端口。此时会自动退回只用 `8766` 拉取，格子会给出可见提示。

处理：关掉占用 8765 的程序，或把 AstrBot 配置里的 `push_url` 改到未占用端口——但**桌面侧固定绑 8765**，改端口后推送就不通，只能继续靠拉取。理想做法是腾出 8765。

### token 不匹配（401）

表现：格子显示「token 无效」。

原因：AstrBot 的 `token` 与 Desktop Tools 里保存的不一致（比如 AstrBot 重装后重新生成了 token）。

处理：重新从 AstrBot 配置页复制，在设置页重新保存。

### AstrBot 未运行（快照未上报）

表现：已配对但没有数据，格子显示「暂无数据」，状态格补「采集插件未上报或未安装」。

排查顺序：

1. AstrBot 进程在不在；
2. WebUI 里采集插件是否启用、`enabled` 是否为 `true`；
3. `collect_interval_sec` 是否被改得太大；
4. 用浏览器打开 `http://127.0.0.1:8766/v1/snapshot` 看是否有响应（需要带上 `X-Status-Token` 头）。

### 收不到推送但拉取正常

说明推送那一路没通（端口占用、`push_url` 被改、或防火墙拦了本机回环）。数据仍会通过 `8766` 拉取出现，只是更新频率低一些。

---

## 7. 安全边界

- 共享 token 只在本机两个程序之间使用，**不要发给任何人、不要贴进聊天或工单**。
- 重装 AstrBot 或换实例后 token 会变，旧 token 在新环境里无效，需要在设置页重新配对。
- AstrBot 侧插件不读取、不记录 AstrBot 的任何既有凭据（`passwordHash`、`jwt_secret`、`ws_reverse_token`、OpenAPI Key），也不调用 `/api/v1/system-config`。
- 两侧都不在日志里输出 token：AstrBot 侧用 `secrets.compare_digest` 比较且不写日志；桌面侧凭据由宿主从 Windows 凭据管理器读取，明文不进缓存、不进插件设置、不进工作区快照。
- 两个端口都只绑 `127.0.0.1`，不监听局域网。

---

## 附：数据来源

| 数据 | 来源 |
| --- | --- |
| 在线四档 / 已就绪时长 / 版本 | 免鉴权 `GET /api/stat/start-time`、`GET /api/stat/versions` |
| 平台连接状态、断连原因、汇总 | `platform_manager.get_all_stats()` |
| CPU / 内存 / 线程数 | `psutil` |
| 消息总量与小时趋势 | `platform_stats` 表 |
| 实时日志（近期，上限 200 条，不落盘） | 挂在 AstrBot 根 logger 上的限速 handler |
| Token 用量 | `provider_stats` 表 |
| 一键启停平台 | `platform_manager.reload()` + 写回 `cmd_config.json` |

采集结果按**快照契约 v2**（`snapshot.schema.json`）推送；拿不到的项一律 `null`，不用 `0` 或空数组冒充。
