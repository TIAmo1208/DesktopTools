// 启动 Desktop Tools 正式应用。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() { desktop_tools_lib::run(); }
