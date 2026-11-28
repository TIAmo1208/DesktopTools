// 启动 Desktop Tools 正式应用。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main()
{
    // 诊断入口：只跑原生宿主演示，不进入 Tauri 启动路径；不加该参数时行为完全不变。
    if let Some(mode) = std::env::args_os().skip(1).find(|arg| arg == "--native-surface-demo" || arg == "--native-surface-workspace" || arg == "--native-surface-content" || arg == "--native-content-overflow-example" || arg == "--native-surface-input" || arg == "--native-surface-input-live" || arg == "--native-surface-input-fixture" || arg == "--native-surface-menu" || arg == "--native-surface-island" || arg == "--native-surface-music" || arg == "--native-surface-portal")
    {
        // 门户诊断：自包含数据 → 外壳/面包屑/网格 + 一次更新一次绘制，不建窗。
        if mode == "--native-surface-portal"
        {
            let json = match desktop_tools_lib::render::run_portal().map(|report| serde_json::to_string_pretty(&report))
            {
                Ok(Ok(json)) => json,
                Ok(Err(error)) => { eprintln!("门户诊断报告序列化失败：{error}"); std::process::exit(1) }
                Err(error) => { eprintln!("门户诊断失败：{error}"); std::process::exit(1) }
            };
            if let Some(path) = std::env::var_os("DESKTOP_NATIVE_SURFACE_REPORT")
            {
                if let Err(error) = std::fs::write(&path, &json)
                {
                    eprintln!("门户诊断报告写入失败（{}）：{error}", std::path::Path::new(&path).display());
                    std::process::exit(1);
                }
            }
            println!("{json}");
            std::process::exit(0);
        }
        // 音乐格诊断：自包含数据 → 布局 + 绘制列表 + 一次更新一次绘制，不建窗。
        if mode == "--native-surface-music"
        {
            let json = match desktop_tools_lib::render::run_music().map(|report| serde_json::to_string_pretty(&report))
            {
                Ok(Ok(json)) => json,
                Ok(Err(error)) => { eprintln!("音乐格诊断报告序列化失败：{error}"); std::process::exit(1) }
                Err(error) => { eprintln!("音乐格诊断失败：{error}"); std::process::exit(1) }
            };
            if let Some(path) = std::env::var_os("DESKTOP_NATIVE_SURFACE_REPORT")
            {
                if let Err(error) = std::fs::write(&path, &json)
                {
                    eprintln!("音乐格诊断报告写入失败（{}）：{error}", std::path::Path::new(&path).display());
                    std::process::exit(1);
                }
            }
            println!("{json}");
            std::process::exit(0);
        }
        // 浮岛诊断：三态几何 + 区域裁剪穿透 + 悬停消抖，不接管桌面。
        if mode == "--native-surface-island"
        {
            let json = match desktop_tools_lib::render::run_island().map(|report| serde_json::to_string_pretty(&report))
            {
                Ok(Ok(json)) => json,
                Ok(Err(error)) => { eprintln!("原生浮岛诊断报告序列化失败：{error}"); std::process::exit(1) }
                Err(error) => { eprintln!("原生浮岛诊断失败：{error}"); std::process::exit(1) }
            };
            if let Some(path) = std::env::var_os("DESKTOP_NATIVE_SURFACE_REPORT")
            {
                if let Err(error) = std::fs::write(&path, &json)
                {
                    eprintln!("原生浮岛诊断报告写入失败（{}）：{error}", std::path::Path::new(&path).display());
                    std::process::exit(1);
                }
            }
            println!("{json}");
            std::process::exit(0);
        }
        // 菜单诊断：自包含场景里打开菜单并走键盘/鼠标/失活路径，报告结构与输入诊断同构。
        if mode == "--native-surface-menu"
        {
            let json = match desktop_tools_lib::render::run_menu().map(|report| serde_json::to_string_pretty(&report))
            {
                Ok(Ok(json)) => json,
                Ok(Err(error)) => { eprintln!("原生菜单诊断报告序列化失败：{error}"); std::process::exit(1) }
                Err(error) => { eprintln!("原生菜单诊断失败：{error}"); std::process::exit(1) }
            };
            if let Some(path) = std::env::var_os("DESKTOP_NATIVE_SURFACE_REPORT")
            {
                if let Err(error) = std::fs::write(&path, &json)
                {
                    eprintln!("原生菜单诊断报告写入失败（{}）：{error}", std::path::Path::new(&path).display());
                    std::process::exit(1);
                }
            }
            println!("{json}");
            std::process::exit(0);
        }
        // 命中与焦点诊断的报告结构与其他诊断不同，统一转成 JSON 后再走同一段落盘/退出逻辑。
        if mode == "--native-surface-input" || mode == "--native-surface-input-live" || mode == "--native-surface-input-fixture"
        {
            let result = if mode != "--native-surface-input"
            {
                desktop_tools_lib::render::run_live(mode == "--native-surface-input-fixture").map(|r| serde_json::to_string_pretty(&r))
            }
            else
            {
                desktop_tools_lib::render::run_input().map(|r| serde_json::to_string_pretty(&r))
            };
            let json = match result
            {
                Ok(Ok(json)) => json,
                Ok(Err(error)) =>
                {
                    eprintln!("原生输入诊断报告序列化失败：{error}");
                    std::process::exit(1)
                }
                Err(error) =>
                {
                    eprintln!("原生输入诊断失败：{error}");
                    std::process::exit(1)
                }
            };
            if let Some(path) = std::env::var_os("DESKTOP_NATIVE_SURFACE_REPORT")
            {
                if let Err(error) = std::fs::write(&path, &json)
                {
                    eprintln!("原生输入诊断报告写入失败（{}）：{error}", std::path::Path::new(&path).display());
                    std::process::exit(1);
                }
            }
            println!("{json}");
            std::process::exit(0);
        }
        let report = if mode == "--native-surface-content" || mode == "--native-content-overflow-example"
        {
            desktop_tools_lib::render::run_content(mode == "--native-content-overflow-example")
        }
        else if mode == "--native-surface-workspace"
        {
            desktop_tools_lib::render::run_workspace()
        }
        else
        {
            desktop_tools_lib::render::run_demo(std::time::Duration::from_secs(5))
        };
        match report
        {
            Ok(report) =>
            {
                let json = serde_json::to_string_pretty(&report).unwrap_or_else(|error| {
                    eprintln!("原生宿主报告序列化失败：{error}");
                    std::process::exit(1)
                });
                // 需要落盘时由调用方给路径，避免诊断输出污染正式产物目录。
                if let Some(path) = std::env::var_os("DESKTOP_NATIVE_SURFACE_REPORT")
                {
                    if let Err(error) = std::fs::write(&path, &json)
                    {
                        eprintln!("原生宿主报告写入失败（{}）：{error}", std::path::Path::new(&path).display());
                        std::process::exit(1);
                    }
                }
                println!("{json}");
                std::process::exit(0);
            }
            Err(error) =>
            {
                eprintln!("原生宿主演示失败：HRESULT=0x{:08X} {error}", error.code().0 as u32);
                std::process::exit(1);
            }
        }
    }
    desktop_tools_lib::run();
}
