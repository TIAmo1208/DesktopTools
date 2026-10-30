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
    /// 前端分派入口。插件固定为 `plugin:slot`，不拿来当脚本路径。
    pub entry: String,
    /// 插件根内的脚本相对路径，形如 `<文件夹>/<入口脚本>`。官方清单为空。
    #[serde(default)]
    pub script: String,
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
}

/// 将组件运行时错误写入标准错误传播链。
impl std::fmt::Display for ComponentRuntimeError
{
    /// 输出错误的用户可见说明。
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { formatter.write_str(&self.message) }
}

/// 允许组件运行时错误进入标准 Rust 错误传播链。
impl std::error::Error for ComponentRuntimeError {}
