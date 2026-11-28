//! 原生渲染核心：绘制指令、几何换算、Direct2D 绘制与 DirectComposition 宿主。
//!
//! 模块边界：
//! - [`draw`]：所有绘制方共用的最小绘制指令集（不含主题与业务语义）。
//! - [`geometry`]：DIP 与物理像素换算、矩形命中、工作区钳制等纯逻辑（不依赖 Win32 句柄）。
//! - [`painter`]：把指令集落成 Direct2D / DirectWrite 绘制。
//! - [`host`]：每显示器一块透明宿主窗口 + DirectComposition 视觉树 + D2D 表面。
//! - [`input`]：命中测试与焦点的纯逻辑，以及命中/焦点诊断入口。
//!
//! 诊断入口（都不进默认启动路径、跑完自行退出）：
//! `--native-surface-demo`、`--native-surface-workspace`、`--native-surface-content`、
//! `--native-content-overflow-example`、`--native-surface-input`（命中与焦点）。

mod content;
pub mod draw;
pub mod geometry;
pub mod host;
pub mod input;
mod live;
pub use live::run_live;
pub mod island;
pub mod music_tile;
pub mod portal_tile;
pub mod thumbs;
pub mod menu;
pub mod painter;
pub use content::run_content;
pub use input::{InputReport, run_input};
pub use island::run_island;
pub use music_tile::run_music;
pub use portal_tile::run_portal;
pub use menu::run_menu;
mod workspace_scene;
pub use workspace_scene::run_workspace;

pub use draw::{Color, DrawCommand, DrawList, TextAlign};
pub use geometry::{DpiScale, PixelRect, dip_rect_to_pixels};
pub use host::{HostReport, run_demo};

#[cfg(test)]
#[path = "geometry.test.rs"]
mod geometry_tests;

#[cfg(test)]
#[path = "draw.test.rs"]
mod draw_tests;
