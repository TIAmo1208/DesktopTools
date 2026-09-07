use serde::{Deserialize, Serialize};

/// 桌面接管启动门槛。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReadinessGate {
    /// 独立 guardian 已完成握手。
    Guardian,
    /// React 页面已完成双帧呈现。
    Page,
    /// 当前系统图像已完成加载。
    SystemImages,
    /// 宿主窗口已检测到真实渲染像素。
    Pixels,
    /// 最小项目输入已回传 Rust。
    Input,
    /// 前端已发布有效交互命中图。
    HitMap,
}

/// 正式透明桌面宿主的接管阶段。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TakeoverPhase {
    /// 宿主和门槛正在准备。
    Starting,
    /// 全部门槛已通过并等待提交。
    Ready,
    /// 正在切换 Explorer 与正式宿主可见性。
    Committing,
    /// 桌面项目接管已经提交。
    Committed,
    /// 接管操作失败。
    Failed,
    /// Explorer 状态已经恢复。
    Restored,
}

/// 一台正式宿主使用的显示器身份、物理边界、工作区与 DPI。
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopHostMonitor {
    /// Windows 显示设备提供的稳定身份。
    pub id: String,
    /// Windows 显示设备的用户可见名称。
    pub name: String,
    /// 当前显示器是否为主显示器。
    pub primary: bool,
    /// 完整显示器物理横坐标。
    pub x: i32,
    /// 完整显示器物理纵坐标。
    pub y: i32,
    /// 完整显示器物理像素宽度。
    pub width: u32,
    /// 完整显示器物理像素高度。
    pub height: u32,
    /// 工作区物理横坐标。
    pub work_x: i32,
    /// 工作区物理纵坐标。
    pub work_y: i32,
    /// 工作区物理像素宽度。
    pub work_width: u32,
    /// 工作区物理像素高度。
    pub work_height: u32,
    /// CSS 像素到物理像素的比例。
    pub scale_factor: f64,
    /// 当前显示器有效 DPI。
    pub dpi: u32,
}

/// 一台显示器宿主的独立上下文快照。
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopHostContextSnapshot {
    /// 当前宿主对应的显示器状态。
    pub monitor: DesktopHostMonitor,
    /// 当前宿主独立的六项启动门槛。
    pub readiness: DesktopHostReadiness,
}

/// 正式桌面接管的六项门槛状态。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopHostReadiness {
    /// guardian 就绪状态。
    pub guardian: bool,
    /// 页面双帧就绪状态。
    pub page: bool,
    /// 系统图像加载就绪状态。
    pub system_images: bool,
    /// 真实像素探针就绪状态。
    pub pixels: bool,
    /// 最小输入探针就绪状态。
    pub input: bool,
    /// 交互命中图就绪状态。
    pub hit_map: bool,
}

/// 发布给 React 的正式宿主不可变快照。
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopHostSnapshot {
    /// 当前桌面接管阶段。
    pub phase: TakeoverPhase,
    /// Explorer 桌面项目接管是否已经提交。
    pub committed: bool,
    /// 当前每台显示器对应的独立宿主上下文。
    pub hosts: Vec<DesktopHostContextSnapshot>,
}

/// React 发布的一个语义交互命中区域。
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HitRegion {
    /// 前端语义对象稳定标识。
    pub id: String,
    /// 前端语义对象类型。
    pub kind: String,
    /// CSS 像素横坐标。
    pub left: f64,
    /// CSS 像素纵坐标。
    pub top: f64,
    /// CSS 像素宽度。
    pub width: f64,
    /// CSS 像素高度。
    pub height: f64,
    /// 重叠区域输入优先级。
    pub priority: i32,
}

/// React 双帧完成后发布的宿主准备证据。
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontendHostReadiness {
    /// 当前页面所属的显示器稳定身份。
    pub monitor_id: String,
    /// 当前页面视口宽度。
    pub viewport_width: f64,
    /// 当前页面视口高度。
    pub viewport_height: f64,
    /// 当前页面设备像素比例。
    pub device_pixel_ratio: f64,
    /// 页面当前系统图像数量。
    pub total_system_images: usize,
    /// 页面成功加载的系统图像数量。
    pub loaded_system_images: usize,
    /// 当前完整语义命中图。
    pub hit_regions: Vec<HitRegion>,
}

/// 正式宿主 command 返回的结构化错误。
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopHostError {
    /// 可稳定检索的错误码。
    pub code: String,
    /// 用户可见的中文错误说明。
    pub message: String,
}

/// 创建正式宿主 command 使用的稳定错误。
impl DesktopHostError {
    /// 创建宿主状态暂时不可用错误。
    pub fn state_unavailable() -> Self {
        Self {
            code: "desktop-host-state-unavailable".to_string(),
            message: "桌面宿主状态暂时不可用".to_string(),
        }
    }

    /// 创建宿主输入参数错误。
    pub fn invalid_readiness(message: impl Into<String>) -> Self {
        Self {
            code: "invalid-desktop-host-readiness".to_string(),
            message: message.into(),
        }
    }

    /// 创建 Windows 宿主操作错误。
    pub fn platform_failed(error: &impl std::fmt::Display) -> Self {
        Self {
            code: "desktop-host-platform-failed".to_string(),
            message: format!("桌面宿主操作失败: {error}"),
        }
    }
}

/// 正式宿主快照变化事件名称。
pub const DESKTOP_HOST_SNAPSHOT_CHANGED_EVENT: &str = "desktop-host-snapshot-changed";
