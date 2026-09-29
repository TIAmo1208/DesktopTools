mod desktop_host;
pub(crate) mod directory_watch;
pub(crate) mod refresh_policy;
pub(crate) use desktop_host::cache_host_hit_regions;
pub(crate) use desktop_host::drag_payload_limit;
pub(crate) use desktop_host::install_host_events;
mod desktop_sources;
pub(crate) mod drop_content;
pub mod drop_target;
pub(crate) mod environment_watch;
pub(crate) mod file_proof;
mod file_system;
pub mod music;
pub mod settings;
mod shell;
pub(crate) use shell::{drag_data_object, transfer_desktop_data, transfer_paths_to_directory};

pub use desktop_host::{NativeCommandHandler, NativeDesktopHost, ShellDesktopState, apply_host_hit_regions, create_desktop_hosts, desktop_item_monitor_assignments, desktop_items_setting_visible, desktop_items_visible, destroy_desktop_hosts, discover_shell_desktop, enumerate_desktop_monitors, load_desktop_hosts, probe_host_pixels, reconcile_desktop_hosts, redraw_shell_desktop, reload_desktop_host, set_desktop_items_visible, set_host_visible, suspend_host_hit_regions};

pub use desktop_host::relay_workspace_drag;
pub(crate) use file_system::move_resource_file;
pub use file_system::replace_file_atomic;

pub use shell::{NativeMenuCommand, NativeMenuTarget, ShellChangeEvent, ShellChangeWatcher, ShellCompensationAction, ShellMenuOwner, compensate_desktop_items, delete_desktop_items, enumerate_desktop_items, initialize_shell_apartment, open_desktop_items, paste_desktop_items, paste_folder_portal_items, rename_desktop_item, set_desktop_item_clipboard, set_desktop_item_clipboard_for_paths, show_native_context_menu, start_desktop_item_drag, start_shell_change_watcher, uninitialize_shell_apartment};
