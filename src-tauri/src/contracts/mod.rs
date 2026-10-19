mod application;
mod component;
mod desktop_host;
mod desktop_item;
pub use desktop_item::ShellDragPreparation;
pub mod settings;
mod workspace;
pub use workspace::FolderPortalInstance;
pub use workspace::{TileAppearance, TileAppearanceChange, TileTitleMode};

pub use application::{APPLICATION_SNAPSHOT_CHANGED_EVENT, ApplicationError, ApplicationSnapshot, ApplicationStatus, parse_application_status};
pub use component::{COMPONENT_HOST_EVENT, COMPONENT_RUNTIME_SNAPSHOT_CHANGED_EVENT, ComponentCapability, ComponentCapabilityDeclaration, ComponentCapabilityResult, ComponentHostEvent, ComponentInstanceSnapshot, ComponentLoadStatus, ComponentManifest, ComponentRegistrationFailure, ComponentRegistrationSnapshot, ComponentRuntimeError, ComponentRuntimeSnapshot};
pub use desktop_host::{DESKTOP_HOST_SNAPSHOT_CHANGED_EVENT, DesktopHostContextSnapshot, DesktopHostError, DesktopHostMonitor, DesktopHostReadiness, DesktopHostSnapshot, FrontendHostReadiness, HitRegion, ReadinessGate, TakeoverPhase};
pub use desktop_item::{DESKTOP_ITEM_SNAPSHOT_CHANGED_EVENT, DesktopItem, DesktopItemAvailability, DesktopItemError, DesktopItemSnapshot, ShellOperationItemResult, ShellOperationKind, ShellOperationResult};
pub use workspace::{FenceLayoutMode, FenceSnapshot, FenceSortMode, TILE_MIN_HEIGHT, TILE_MIN_WIDTH, UnclassifiedItemSnapshot, WORKSPACE_BACKUP_SCHEMA_VERSION, WORKSPACE_GRID_BOTTOM_MARGIN, WORKSPACE_GRID_COLUMN_WIDTH, WORKSPACE_GRID_LEFT, WORKSPACE_GRID_ROW_HEIGHT, WORKSPACE_GRID_TOP, WORKSPACE_SCHEMA_VERSION, WORKSPACE_SNAPSHOT_CHANGED_EVENT, WorkspaceBackupSummary, WorkspaceCommand, WorkspaceContextMenuTarget, WorkspaceError, WorkspaceFenceScroll, WorkspaceFocusTarget, WorkspaceGrid, WorkspaceHistoryKind, WorkspaceHistoryResult, WorkspaceInteractionCommand, WorkspaceInteractionSnapshot, WorkspaceItemPlacement, WorkspaceMonitorGrid, WorkspacePoint, WorkspaceRect, WorkspaceRestoreResult, WorkspaceSelectionMode, WorkspaceSnapshot};
