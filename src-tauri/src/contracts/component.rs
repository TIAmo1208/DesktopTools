use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::workspace::WorkspaceRect;

/// 组件运行时快照变化事件名称。
pub const COMPONENT_RUNTIME_SNAPSHOT_CHANGED_EVENT: &str = "component-runtime-snapshot-changed";

/// 组件接收宿主事件的事件名称。
pub const COMPONENT_HOST_EVENT: &str = "component-host-event";

/// 插件设置保存成功后的通知；载荷只含 pluginId，读者重新读取命令。
pub const PLUGIN_SETTINGS_CHANGED_EVENT: &str = "plugin-settings-changed";

/// 官方组件可以声明并由宿主授权的能力。
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ComponentCapability
{
    /// 读取宿主提供的本地 Unix 时间。
    ReadHostTime,
    /// 读取系统当前媒体会话。
    ReadCurrentMedia,
    /// 探活宿主固定的本机服务地址（当前只有 AstrBot）。
    ProbeServiceStatus,
    /// 注册本机 loopback 通道，只收发原始 HTTP。
    LoopbackChannel,
}

/// 插件呈现角色，未声明时保持格子行为。
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ComponentRole
{
    #[default]
    Tile,
    Theme,
}

/// 主题经偏好写入的外观覆盖，缺省项不覆盖用户值。
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ThemeAppearance
{
    pub accent_color: Option<String>,         // 界面强调色。
    pub desktop_accent_color: Option<String>, // 桌面强调色。
    pub corner_radius: Option<u32>,           // 共用圆角。
    pub font_family: Option<String>,          // 系统字体或本主题自带字体的族名。
    pub font_size: Option<u32>,               // 正文字号。
}

/// 主题声明的启动覆盖层时长；缺省用内置时长，0 表示不播。
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ThemeBoot
{
    pub duration_ms: Option<u32>, // 覆盖层从出现到消失的总时长，上限 10 秒。
}

/// 主题声明的原生消息配色。整套只有一套色值，不随深浅模式变化；缺省字段回落到当前模式的内置色。
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ThemeNotification
{
    pub surface: Option<String>,      // 消息表面。
    pub border: Option<String>,       // 消息边框。
    pub text: Option<String>,         // 正文。
    pub muted: Option<String>,        // 次要正文。
    pub subtle: Option<String>,       // 后层消息与悬停表面。
    pub success: Option<String>,      // 成功状态图标。
    pub warning: Option<String>,      // 警告状态图标；未声明时沿用次要正文色。
    pub error: Option<String>,        // 错误状态图标。
    pub weight: Option<String>,       // 正文字重：normal、medium、semibold 或 bold。
    pub corner_style: Option<String>, // 原生消息轮廓：round 或 bevel。
    pub corner_size: Option<u32>,     // bevel 的 45° 切角尺寸，1–24 DIP。
}

/// 主题自带的字体文件，由宿主注册到文档字体集。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThemeFont
{
    /// 用户可见的字体族名，也是偏好里保存的值。
    pub family: String,
    /// 插件根内的字体文件相对路径，形如 `<文件夹>/fonts/x.woff2`。
    pub file: String,
    /// 字重，100–900。
    pub weight: u16,
    /// 字形，`normal` 或 `italic`。
    pub style: String,
}

/// 类型级尺寸，单位 DIP；只用于创建默认值与实例有效下限。
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TileSize
{
    /// 宽度。
    pub width: u32,
    /// 高度。
    pub height: u32,
}

/// 插件清单声明的一种格子类型，键写进实例 state 的 tileType。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentTileType
{
    /// 作者代码里的稳定标识，也是 state 里保存的值。
    pub key: String,
    /// 创建菜单里显示的名称。
    pub name: String,
    /// 该类型创建时的默认外框；未声明时沿用公共默认尺寸。
    #[serde(default)]
    pub default_size: Option<TileSize>,
    /// 该类型实例的有效下限；未声明时沿用公共最小尺寸。
    #[serde(default)]
    pub minimum_size: Option<TileSize>,
}

/// 声明了 [tiles] 的插件把数量上限按实例 tileType 分桶；桶键是类型键，缺类型落到默认类型。
pub const DEFAULT_TILE_TYPE: &str = "default";

/// 读取实例 state 里保存的格子类型。state 不是对象或字段不是字符串时返回空串。
pub fn component_tile_type(state: &serde_json::Value) -> &str { state.get("tileType").and_then(serde_json::Value::as_str).unwrap_or("") }

/// 返回该插件用于数量计数的桶键。没有声明类型时整份清单共用一个桶，旧插件与旧工作区行为不变。
pub fn tile_type_bucket<'a>(tile_types: &'a [ComponentTileType], state: &'a serde_json::Value) -> &'a str
{
    if tile_types.is_empty()
    {
        return "";
    }
    let tile_type = component_tile_type(state);
    if tile_type.is_empty()
    {
        tile_types.first().map_or(DEFAULT_TILE_TYPE, |default| default.key.as_str()) // 旧实例按第一节（默认类型）计。
    }
    else
    {
        tile_type
    }
}

/// 一个官方组件的版本化清单。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentManifest
{
    /// 组件包稳定身份。
    pub id: String,
    /// 用户可见组件名称。
    pub name: String,
    /// 插件清单版本；官方组件与测试构造缺省为空。
    #[serde(default)]
    pub version: String,
    /// 前端分派入口。插件固定为 `plugin:slot`，不拿来当脚本路径。
    pub entry: String,
    /// 插件根内的脚本相对路径，形如 `<文件夹>/<入口脚本>`。官方清单为空。
    #[serde(default)]
    pub script: String,
    /// 插件角色，官方及未声明角色的插件为格子。
    #[serde(default)]
    pub role: ComponentRole,
    /// 插件根内的主题样式表相对路径，非主题为空。
    #[serde(default)]
    pub stylesheet: String,
    /// 仅用于主题样式表 URL 缓存失效；由清单版本与文件元数据组成。
    #[serde(default)]
    pub stylesheet_cache_key: String,
    /// 主题外观覆盖，非主题为空。
    #[serde(default)]
    pub theme_appearance: Option<ThemeAppearance>,
    /// 主题声明的原生消息配色，非主题或未声明时为空。
    #[serde(default)]
    pub theme_notification: Option<ThemeNotification>,
    /// 主题声明的启动覆盖层时长，非主题或未声明时为空。
    #[serde(default)]
    pub theme_boot: Option<ThemeBoot>,
    /// 主题自带的字体，非主题为空。
    #[serde(default)]
    pub theme_fonts: Vec<ThemeFont>,
    /// 组件明确请求的全部宿主能力。
    pub required_capabilities: Vec<ComponentCapability>,
    /// 用户是否可从工作区入口创建该组件的新实例。JSON 缺省为 false。
    #[serde(default)]
    pub creatable: bool,
    /// 用户是否可移除该组件的实例。JSON 缺省为 false；插件 ini 缺省为 true。
    #[serde(default)]
    pub removable: bool,
    /// 桌面格子数量上限。0 表示不限。
    #[serde(default)]
    pub max_tiles: u32,
    /// 插件声明的格子类型，按清单顺序排列，第一项为默认类型。空表示只有一个隐式类型。
    #[serde(default)]
    pub tile_types: Vec<ComponentTileType>,
    /// 插件仓库地址。空字符串表示未声明。只保留 http 或 https。
    #[serde(default)]
    pub repository: String,
    /// 插件简介。空字符串表示未写。换行在 ini 里写成 \n。
    #[serde(default)]
    pub description: String,
    /// 插件作者。空字符串表示未写。
    #[serde(default)]
    pub author: String,
    /// 组件持久状态 Schema 版本。
    pub persistence_version: u32,
}

/// 官方组件清单对创建与移除能力的声明结果。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComponentCapabilityDeclaration
{
    /// 已注册清单的组件包身份。
    pub component_id: String,
    /// 用户是否可创建该组件的新实例。
    pub creatable: bool,
    /// 用户是否可移除该组件的实例。
    pub removable: bool,
    /// 该组件允许的桌面格子数量上限。0 表示不限。
    pub max_tiles: u32,
    /// 该组件声明并已通过校验的格子类型。空表示按组件身份整体计数。
    pub tile_types: Vec<ComponentTileType>,
}

/// 工作区持久化的一个组件实例。
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentInstanceSnapshot
{
    /// 工作区内部稳定实例标识。
    pub instance_id: String,
    /// 已注册清单的组件包身份。
    pub component_id: String,
    /// 实例持久归属的显示器稳定身份。
    #[serde(default)]
    pub monitor_id: String,
    /// 持久显示器断开时使用的临时投射显示器。
    #[serde(default)]
    pub projection_monitor_id: Option<String>,
    /// 临时投射期间使用的可见边界。
    #[serde(default)]
    pub projection_bounds: Option<WorkspaceRect>,
    /// 组件表面的持久位置和尺寸。
    pub bounds: WorkspaceRect,
    /// 当前实例状态对应的组件持久化版本。
    pub state_version: u32,
    /// 组件拥有的 JSON 持久状态。
    pub state: Value,
}

/// 一个组件实例当前的隔离加载状态。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ComponentLoadStatus
{
    /// 清单、能力和状态版本均已通过校验。
    Ready,
    /// 当前实例加载失败但不影响其他实例。
    Failed,
}

/// Component Runtime 发布给前端的一个实例注册结果。
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentRegistrationSnapshot
{
    /// 工作区组件实例稳定身份。
    pub instance_id: String,
    /// 实例引用的组件包身份。
    pub component_id: String,
    /// 校验成功后使用的前端入口。
    pub entry: Option<String>,
    /// 当前实例实际获得的宿主能力。
    pub authorized_capabilities: Vec<ComponentCapability>,
    /// 当前实例加载状态。
    pub status: ComponentLoadStatus,
    /// 加载失败时的用户可见中文原因。
    pub error: Option<String>,
}

/// 一个未能进入官方组件注册表的清单失败结果。
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentRegistrationFailure
{
    /// 能够解析时保留的组件身份。
    pub component_id: String,
    /// 用户可见中文失败原因。
    pub message: String,
}

/// Component Runtime 的不可变注册表和实例快照。
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentRuntimeSnapshot
{
    /// 注册表或实例集合变化时递增的修订号。
    pub revision: u64,
    /// 当前成功注册的全部官方组件清单。
    pub manifests: Vec<ComponentManifest>,
    /// 统一工作区的实例注册结果。
    pub instances: Vec<ComponentRegistrationSnapshot>,
    /// 不影响其他组件运行的清单加载失败集合。
    pub failures: Vec<ComponentRegistrationFailure>,
}

/// 一条 loopback 通道的方向。
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LoopbackChannelMode
{
    /// 宿主在 127.0.0.1:<port> 监听，接住对端推送。
    Listen,
    /// 宿主向 http://127.0.0.1:<port> 发请求。
    Request,
}

/// `register_loopback_channel` 的入参；端点字段按 mode 二选一。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RegisterLoopbackChannel
{
    /// 注册通道的插件实例。
    pub instance_id: String,
    /// 插件声明的通道标识，实例内唯一。
    pub channel_id: String,
    /// 通道方向。
    pub mode: LoopbackChannelMode,
    /// listen 通道的端口，1–65535。
    #[serde(default)]
    pub port: Option<u16>,
    /// request 通道的基址，只接受本机 http 地址。
    #[serde(default)]
    pub base_url: Option<String>,
    /// Windows Credential Manager 记录名；未声明时不做注入与校验。
    #[serde(default)]
    pub credential_ref: Option<String>,
    /// 注入与校验凭据使用的头名，由插件声明。
    #[serde(default)]
    pub header_name: Option<String>,
}

/// 注册成功后返回的通道句柄，不进入工作区快照也不落盘。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoopbackChannelHandle
{
    /// 插件声明的通道标识。
    pub channel_id: String,
    /// 通道方向。
    pub mode: LoopbackChannelMode,
    /// listen 通道实际绑定的端口；request 通道为空。
    pub port: Option<u16>,
    /// request 通道的目标基址；listen 通道为空。
    pub base_url: Option<String>,
    /// 注册时刻的 Unix 秒。
    pub registered_at: i64,
}

/// 原始 HTTP 头；保留重复项与顺序，宿主不做合并。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoopbackHeader
{
    /// 头名，按请求原样保留。
    pub name: String,
    /// 头值，按请求原样保留。
    pub value: String,
}

/// 入站缓冲里的一条原始请求；宿主不解析 body。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoopbackRequest
{
    /// 宿主递增的缓冲序号，从 1 起。
    pub sequence: u64,
    /// 宿主接住请求的 Unix 秒。
    pub received_at: i64,
    /// 请求方法。
    pub method: String,
    /// 请求路径，含查询串。
    pub path: String,
    /// 原始请求头。
    pub headers: Vec<LoopbackHeader>,
    /// 原始请求体，按 UTF-8 有损解码。
    pub body: String,
}

/// `request_loopback` 返回的原始响应。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoopbackResponse
{
    /// 对端状态码；非 2xx 也照常返回。
    pub status: u16,
    /// 原始响应头。
    pub headers: Vec<LoopbackHeader>,
    /// 原始响应体，按 UTF-8 有损解码。
    pub body: String,
}

/// 宿主定时发布给已授权组件的事件。
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentHostEvent
{
    /// 当前进程内递增的宿主事件序号。
    pub sequence: u64,
    /// 宿主事件稳定类型。
    pub kind: String,
    /// 宿主提供的当前 Unix 毫秒时间。
    pub unix_millis: i64,
}

/// 一次受能力检查保护的组件宿主调用结果。
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentCapabilityResult
{
    /// 完成调用的组件实例身份。
    pub instance_id: String,
    /// 本次实际授权并执行的能力。
    pub capability: ComponentCapability,
    /// 能力返回的结构化 JSON 载荷。
    pub payload: Value,
}

/// Component Runtime command 返回的结构化错误。
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentRuntimeError
{
    /// 可稳定检索的错误码。
    pub code: String,
    /// 用户可见的中文错误说明。
    pub message: String,
}

/// 创建组件清单、实例和能力边界错误。
impl ComponentRuntimeError
{
    /// 创建组件清单校验错误。
    pub fn invalid_manifest(message: impl Into<String>) -> Self { Self { code: "invalid-component-manifest".to_string(), message: message.into() } }

    /// 创建组件实例校验错误。
    pub fn invalid_instance(message: impl Into<String>) -> Self { Self { code: "invalid-component-instance".to_string(), message: message.into() } }

    /// 创建未声明或未授权能力错误。
    pub fn capability_denied(capability: ComponentCapability) -> Self { Self { code: "component-capability-denied".to_string(), message: format!("组件能力未声明或未授权: {capability:?}") } }

    /// 创建组件运行时状态锁错误。
    pub fn state_unavailable() -> Self { Self { code: "component-runtime-state-unavailable".to_string(), message: "组件运行时状态暂时不可用".to_string() } }

    /// loopback 通道端口已被占用或绑定失败；宿主不抢占、不改用其它端口。
    pub fn loopback_port_in_use(port: u16) -> Self { Self { code: "loopback-port-in-use".to_string(), message: format!("本机端口 {port} 已被占用，通道注册失败") } }

    /// loopback 通道地址或端口不符合硬限制。
    pub fn loopback_address_invalid(message: impl Into<String>) -> Self { Self { code: "loopback-address-invalid".to_string(), message: message.into() } }

    /// 实例已达到通道数量上限。
    pub fn loopback_channel_limit(limit: usize) -> Self { Self { code: "loopback-channel-limit".to_string(), message: format!("每个实例最多注册 {limit} 条 loopback 通道") } }

    /// 凭据声明无效或凭据存储里没有对应记录。
    pub fn loopback_credential_unavailable(message: impl Into<String>) -> Self { Self { code: "loopback-credential-unavailable".to_string(), message: message.into() } }

    /// 指定实例下没有这条通道，或通道已被替换与释放。
    pub fn loopback_channel_not_found() -> Self { Self { code: "loopback-channel-not-found".to_string(), message: "loopback 通道不存在或已释放".to_string() } }

    /// 出站请求在传输层失败，未拿到对端响应。
    pub fn loopback_request_failed(message: impl Into<String>) -> Self { Self { code: "loopback-request-failed".to_string(), message: message.into() } }

    /// 凭据目标或密钥内容不符合受控规则；错误信息不包含密钥明文。
    pub fn loopback_credential_invalid(message: impl Into<String>) -> Self { Self { code: "loopback-credential-invalid".to_string(), message: message.into() } }

    /// 写入或清除 Windows 凭据存储失败；错误信息不包含密钥明文。
    pub fn loopback_credential_store_failed(message: impl Into<String>) -> Self { Self { code: "loopback-credential-store-failed".to_string(), message: message.into() } }
}

/// 将组件运行时错误写入标准错误传播链。
impl std::fmt::Display for ComponentRuntimeError
{
    /// 输出错误的用户可见说明。
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { formatter.write_str(&self.message) }
}

/// 允许组件运行时错误进入标准 Rust 错误传播链。
impl std::error::Error for ComponentRuntimeError {}
