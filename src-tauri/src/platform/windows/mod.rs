mod desktop_host;
mod file_system;
mod shell;

pub use desktop_host::{
    NativeCommandHandler, NativeDesktopHost, ShellDesktopState, apply_host_hit_regions,
    create_primary_desktop_host, destroy_primary_desktop_host, discover_shell_desktop,
    load_primary_desktop_host, probe_host_pixels, redraw_shell_desktop, set_desktop_items_visible,
    set_host_visible, suspend_host_hit_regions,
};

pub use file_system::replace_file_atomic;

pub use shell::{
    enumerate_desktop_items, initialize_shell_apartment, uninitialize_shell_apartment,
};
