/// 动作插入导致序号变化后重复注册，旧子键必须消失且全部当前动作保留。
#[test]
fn registration_replaces_stale_actions()
{
    use windows::Win32::System::Registry::{KEY_READ, RegOpenKeyExW};
    let path = format!(r"Software\DesktopToolsMenuTest-{}", std::process::id()); // 本测试独占的非产品键。
    let obsolete = format!(r"{path}\shell\01-edit-layout"); // 新清单中序号已经改变的旧动作。
    let executable = std::path::Path::new(r"C:\Program Files\Desktop Tools\desktop-tools.exe"); // 含空格的注册命令路径。
    super::write_value(&obsolete, "MUIVerb", "编辑布局").unwrap();
    for _ in 0..2
    {
        super::register_actions(&path, executable).unwrap();
    }
    let mut key = super::HKEY::default(); // 测试读取句柄。
    assert_eq!(unsafe { RegOpenKeyExW(super::HKEY_CURRENT_USER, &super::HSTRING::from(obsolete), None, KEY_READ, &mut key) }.0, 2);
    for (index, (action, _, _)) in super::ACTIONS.iter().enumerate()
    {
        let current = format!(r"{path}\shell\{index:02}-{action}\command"); // 当前动作必须存在且有命令。
        unsafe { RegOpenKeyExW(super::HKEY_CURRENT_USER, &super::HSTRING::from(current), None, KEY_READ, &mut key) }.ok().unwrap();
        drop(unsafe { super::Owned::new(key) });
    }
    unsafe { super::RegDeleteTreeW(super::HKEY_CURRENT_USER, &super::HSTRING::from(path)) }.ok().unwrap();
}
/// 安装后从 Shell 桌面视图直接枚举菜单，验证注册项由系统加载而非应用追加。
#[test]
#[ignore = "要求先运行正式程序的 --register-desktop-menu"]
fn registered_desktop_menu_is_loaded_by_shell()
{
    use windows::Win32::System::Ole::{OleInitialize, OleUninitialize};
    use windows::Win32::UI::Shell::{CMF_NORMAL, IContextMenu, IContextMenu3, IShellView, SHGetDesktopFolder, SVGIO_BACKGROUND};
    use windows::Win32::UI::WindowsAndMessaging::{CreatePopupMenu, GetMenuItemCount, GetMenuStringW, GetSubMenu, MF_BYPOSITION, WM_INITMENUPOPUP};
    use windows::core::Interface;
    unsafe { OleInitialize(None) }.unwrap();
    {
        let desktop = unsafe { SHGetDesktopFolder() }.unwrap(); // Windows 桌面命名空间。
        let view: IShellView = unsafe { desktop.CreateViewObject(super::HWND::default()) }.unwrap(); // 本地原生视图，仅检查菜单。
        let menu: IContextMenu = unsafe { view.GetItemObject(SVGIO_BACKGROUND) }.unwrap(); // 由 Shell 自己读取注册表。
        let popup = unsafe { super::Owned::new(CreatePopupMenu().unwrap()) }; // 测试不追加任何菜单项。
        unsafe { menu.QueryContextMenu(*popup, 0, 1, 0x6fff, CMF_NORMAL | windows::Win32::UI::Shell::CMF_SYNCCASCADEMENU).ok() }.unwrap(); // 无 UI 消息泵的测试必须同步加载级联子菜单。
        let handler: IContextMenu3 = menu.cast().unwrap(); // 展开延迟子菜单。
        let _ = unsafe { handler.HandleMenuMsg2(WM_INITMENUPOPUP, super::WPARAM(popup.0 as usize), super::LPARAM(0), None) }; // 先初始化根菜单，符合真实菜单循环的顺序。
        let mut product_labels = Vec::new(); // 系统实际加载的产品动作。
        for index in 0..unsafe { GetMenuItemCount(Some(*popup)) }
        {
            let mut buffer = [0u16; 256]; // 当前顶层标签。
            let length = unsafe { GetMenuStringW(*popup, index as u32, Some(&mut buffer), MF_BYPOSITION) };
            let label = String::from_utf16_lossy(&buffer[..length.max(0) as usize]); // Shell 可能添加访问键。
            eprintln!("原生桌面菜单项: {label}");
            if label.replace('&', "") != "Desktop Tools"
            {
                continue;
            }
            let submenu = unsafe { GetSubMenu(*popup, index) }; // 系统创建的级联菜单。
            assert!(!submenu.is_invalid());
            let initialization = unsafe { handler.HandleMenuMsg2(WM_INITMENUPOPUP, super::WPARAM(submenu.0 as usize), super::LPARAM(index as isize), None) };
            eprintln!("子菜单初始化: {initialization:?}，数量={}", unsafe { GetMenuItemCount(Some(submenu)) });
            let submenu = unsafe { GetSubMenu(*popup, index) }; // Shell 初始化时可能替换占位子菜单句柄。
            for child in 0..unsafe { GetMenuItemCount(Some(submenu)) }
            {
                let length = unsafe { GetMenuStringW(submenu, child as u32, Some(&mut buffer), MF_BYPOSITION) };
                product_labels.push(String::from_utf16_lossy(&buffer[..length.max(0) as usize]).replace('&', ""));
            }
        }
        assert_eq!(product_labels, super::ACTIONS.iter().map(|entry| entry.1.to_string()).collect::<Vec<_>>());
    }
    unsafe { OleUninitialize() };
}

/// 空格、中文路径保持单个可执行文件参数，未知动词不能进入命令行。
#[test]
fn desktop_menu_commands_quote_paths_and_reject_unknown_actions()
{
    let path = std::path::Path::new(r"C:\Program Files\桌面工具\desktop-tools.exe"); // 有空格的 Unicode 安装路径。
    assert_eq!(super::command_line(path, "settings").unwrap(), "\"C:\\Program Files\\桌面工具\\desktop-tools.exe\" --desktop-menu settings");
    assert!(super::command_line(path, "settings & calc").is_err());
}
