/// 真实 OLE 循环遗漏松键时能自行退出，已经开始 Drop 的期间不被检查线程取消。
#[test]
#[ignore = "需要交互式 Windows 桌面，短暂建立真实 OLE 会话但不执行文件投放"]
fn lost_source_release_exits_ole_without_cancelling_started_drop()
{
    use super::*;
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
    /// 模拟始终丢失松键的源，绝不返回 Drop。
    #[implement(IDropSource)]
    struct LostReleaseSource;
    impl IDropSource_Impl for LostReleaseSource_Impl
    {
        fn QueryContinueDrag(&self, escape: BOOL, _: MODIFIERKEYS_FLAGS) -> HRESULT { if escape.as_bool() { DRAGDROP_S_CANCEL } else { S_OK } }
        fn GiveFeedback(&self, _: DROPEFFECT) -> HRESULT { DRAGDROP_S_USEDEFAULTCURSORS }
    }
    initialize_shell_apartment().unwrap();
    assert!(unsafe { GetAsyncKeyState(VK_LBUTTON.0 as i32) } >= 0, "测试开始时必须松开鼠标");
    let path = std::env::temp_dir().join(format!("desktop-tools-release-{}.txt", std::process::id())); // 不触碰用户文件。
    std::fs::write(&path, "release probe").unwrap();
    let item: IShellItem = unsafe { windows::Win32::UI::Shell::SHCreateItemFromParsingName(&windows::core::HSTRING::from(path.to_string_lossy().as_ref()), None) }.unwrap(); // 临时文件真实 Shell 身份。
    let data: IDataObject = unsafe { item.BindToHandler(None, &windows::Win32::UI::Shell::BHID_DataObject) }.unwrap(); // 原生数据对象留在本 STA。
    let source: IDropSource = LostReleaseSource.into(); // 不允许写文件的模拟源。
    let source_thread = unsafe { windows::Win32::System::Threading::GetCurrentThreadId() }; // 本次 OLE 所属线程。
    let drop_started = Arc::new(AtomicBool::new(true)); // 先模拟目标正在执行，检查线程必须等待。
    let (stop, worker) = watch_source_drag_release(source_thread, Arc::clone(&drop_started)).unwrap(); // 与产品相同的释放检查。
    let begin = std::time::Instant::now(); // 检测不得过早取消，也不能永久不返回。
    let allow_cancel = thread::spawn(move || {
        thread::sleep(std::time::Duration::from_millis(200));
        drop_started.store(false, Ordering::Release);
    }); // 随后模拟尚未交付的失联源。
    let (stop_fallback, receiver) = mpsc::channel::<()>(); // 失败时独立解除系统拖放，防止测试本身挂死。
    let fallback = thread::spawn(move || {
        if receiver.recv_timeout(std::time::Duration::from_secs(2)).is_err()
        {
            let _ = unsafe { windows::Win32::UI::WindowsAndMessaging::PostThreadMessageW(source_thread, windows::Win32::UI::WindowsAndMessaging::WM_KEYDOWN, WPARAM(27), LPARAM(0)) };
        }
    }); // 原生源循环不保证处理 WM_TIMER，兜底同样不依赖它。
    let mut effect = DROPEFFECT_NONE; // 本例没有投放结果。
    let status = unsafe { DoDragDrop(&data, &source, DROPEFFECT_COPY, &mut effect) };
    let elapsed = begin.elapsed(); // 正式检查应在允许取消后很快结束。
    let _ = stop.send(());
    let _ = stop_fallback.send(());
    worker.join().unwrap();
    allow_cancel.join().unwrap();
    fallback.join().unwrap();
    drop(source);
    drop(data);
    drop(item);
    uninitialize_shell_apartment();
    std::fs::remove_file(path).unwrap();
    assert_eq!(status, DRAGDROP_S_CANCEL);
    assert!(elapsed >= std::time::Duration::from_millis(180) && elapsed < std::time::Duration::from_secs(1), "释放检查未遵守投放阶段或没有及时退出：{elapsed:?}");
}

/// 文件执行边界必须在任何磁盘操作前拒绝链接、无效果和组合效果。
#[test]
fn desktop_transfer_rejects_unsupported_effects()
{
    super::initialize_shell_apartment().unwrap();
    {
        let owner = super::ShellMenuOwner::create().unwrap(); // 当前测试的 STA owner。
        let second_owner = super::ShellMenuOwner::create().unwrap(); // 同进程重复创建应复用已注册窗口类。
        assert_ne!(owner.hwnd, second_owner.hwnd);
        let item = super::resolve_shell_item(&std::env::temp_dir().to_string_lossy()).unwrap(); // 外部文件夹只用于构造数据对象，不执行复制。
        let data: super::IDataObject = unsafe { item.BindToHandler(None, &windows::Win32::UI::Shell::BHID_DataObject) }.unwrap(); // 通过项目自己的 Shell handler 提供外部拖放数据。
        for effect in [super::DROPEFFECT_NONE, super::DROPEFFECT_LINK, super::DROPEFFECT_COPY | super::DROPEFFECT_MOVE]
        {
            assert!(super::transfer_desktop_data(&owner, &data, effect, None).unwrap_err().contains("只接受复制或移动"));
        }
    }
    super::uninitialize_shell_apartment();
}

/// 门户拖放使用文件系统数据对象，并以已校验路径直接执行跨目录传输。
#[test]
fn portal_drag_uses_filesystem_items_without_cross_apartment_data_object()
{
    super::initialize_shell_apartment().unwrap();
    let root = std::env::temp_dir().join(format!("desktop-tools-portal-drag-{}", std::process::id()));
    let source = root.join("source");
    let destination = root.join("destination");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::create_dir_all(&destination).unwrap();
    let file = source.join("probe.txt");
    std::fs::write(&file, "portal drag").unwrap();
    let references = vec![file.to_string_lossy().into_owned()];
    {
        let data = super::drag_data_object(windows::Win32::Foundation::HWND::default(), &references, true).unwrap();
        assert_eq!(crate::platform::windows::drop_target::filesystem_drop_items(&data).unwrap(), references);
        let owner = super::ShellMenuOwner::create().unwrap();
        let extended_destination = format!(r"\\?\{}", destination.to_string_lossy()); // 覆盖门户 canonicalize 产生的目标格式。
        let result = super::transfer_paths_to_directory(&owner, super::DROPEFFECT_COPY, Some(&extended_destination), &references).unwrap();
        assert!(result.items.iter().any(|item| item.succeeded));
        assert!(destination.join("probe.txt").exists());
    }
    super::uninitialize_shell_apartment();
    std::fs::remove_dir_all(root).unwrap();
}

/// 同名覆盖成功但 Shell 未返回新对象时，沿用操作前已确认的目标身份。
#[test]
fn overwrite_without_new_item_keeps_existing_target_reference()
{
    super::initialize_shell_apartment().unwrap();
    let root = std::env::temp_dir().join(format!("desktop-tools-overwrite-{}", std::process::id())); // 当前测试的隔离目录。
    let source_dir = root.join("source"); // 覆盖来源目录。
    let destination_dir = root.join("destination"); // 已有同名目标目录。
    std::fs::create_dir_all(&source_dir).unwrap();
    std::fs::create_dir_all(&destination_dir).unwrap();
    let source_path = source_dir.join("probe.txt"); // 本次复制来源。
    let target_path = destination_dir.join("probe.txt"); // 覆盖前已存在的目标。
    std::fs::write(&source_path, "new").unwrap();
    std::fs::write(&target_path, "old").unwrap();
    {
        let source = super::resolve_shell_item(&source_path.to_string_lossy()).unwrap();
        let destination = super::resolve_shell_item(&destination_dir.to_string_lossy()).unwrap();
        let expected = super::resolve_shell_item(&target_path.to_string_lossy()).and_then(|item| super::shell_item_reference(&item)).unwrap(); // 操作前的准确目标引用。
        let (sink, results) = super::create_operation_sink();
        unsafe {
            sink.PreCopyItem(windows::Win32::UI::Shell::TSF_OVERWRITE_EXIST.0 as u32, &source, &destination, super::PCWSTR::null()).unwrap();
            sink.PostCopyItem(windows::Win32::UI::Shell::TSF_OVERWRITE_EXIST.0 as u32, &source, &destination, super::PCWSTR::null(), super::S_OK, None).unwrap();
        }
        let actual = results.lock().unwrap();
        assert_eq!(actual.len(), 1);
        assert_eq!(actual[0].result_reference.as_deref(), Some(expected.as_str()));
        assert!(actual[0].compensation_blocked.is_some());
    }
    super::uninitialize_shell_apartment();
    std::fs::remove_dir_all(root).unwrap();
}

/// 优化移动的 NONE 不意味着取消，外部逻辑反馈也不能覆盖明确取消。
#[test]
fn optimized_move_is_not_reported_as_cancelled()
{
    assert_eq!(super::completed_drag_effect(super::DRAGDROP_S_DROP, super::DROPEFFECT_NONE, Some(super::DROPEFFECT_MOVE)), (false, super::DROPEFFECT_MOVE));
    assert_eq!(super::completed_drag_effect(super::DRAGDROP_S_DROP, super::DROPEFFECT_NONE, None), (false, super::DROPEFFECT_NONE));
    assert_eq!(super::completed_drag_effect(super::DRAGDROP_S_CANCEL, super::DROPEFFECT_MOVE, Some(super::DROPEFFECT_MOVE)), (true, super::DROPEFFECT_NONE));
}

/// 验证嵌套或交错回调按对象身份关联来源，不依赖先进先出顺序。
#[test]
fn interleaved_file_callbacks_keep_source_identity()
{
    super::initialize_shell_apartment().unwrap();
    {
        let first = super::resolve_shell_item(&std::env::temp_dir().to_string_lossy()).unwrap(); // 已存在的临时目录身份。
        let second = super::resolve_shell_item(&std::env::current_dir().unwrap().to_string_lossy()).unwrap(); // 不同的工作目录身份。
        let first_reference = super::shell_item_reference(&first).unwrap(); // 预期第一项来源。
        let second_reference = super::shell_item_reference(&second).unwrap(); // 预期第二项来源。
        let (sink, results) = super::create_operation_sink(); // 真实 COM 进度回调入口。
        unsafe {
            sink.PreCopyItem(0, &first, None, super::PCWSTR::null()).unwrap();
            sink.PreCopyItem(0, &second, None, super::PCWSTR::null()).unwrap();
            sink.PostCopyItem(0, &second, None, super::PCWSTR::null(), super::S_OK, &second).unwrap();
            sink.PostCopyItem(0, &first, None, super::PCWSTR::null(), super::S_OK, &first).unwrap();
            for status in [windows::Win32::UI::Shell::COPYENGINE_S_USER_IGNORED, windows::Win32::UI::Shell::COPYENGINE_S_PENDING, windows::Win32::UI::Shell::COPYENGINE_S_MERGE]
            {
                sink.PostCopyItem(0, &first, None, super::PCWSTR::null(), status, &first).unwrap();
            }
        }
        let results = results.lock().unwrap(); // 按回调完成顺序采集的实际结果。
        assert_eq!(results[0].source_reference, second_reference);
        assert_eq!(results[1].source_reference, first_reference);
        assert!(results[..2].iter().all(|item| item.succeeded));
        assert!(results[2..].iter().all(|item| !item.succeeded));
    }
    super::uninitialize_shell_apartment();
}

/// 验证整体错误与取消状态查询失败都不会抹去已完成的逐项结果。
#[test]
fn file_operation_failure_preserves_completed_items()
{
    let item = super::ShellOperationItemResult { proof: None, compensation_blocked: None, source_reference: "source".to_string(), result_reference: Some("destination".to_string()), succeeded: true, result_code: 0, source_parent_reference: None, source_name: None, result_parent_reference: None, result_name: None }; // 整体错误发生前已经完成的文件。
    let failed = super::completed_file_operation(super::ShellOperationKind::Paste, true, false, None, Err(windows::core::Error::from_hresult(super::HRESULT(0x80004005_u32 as i32))), Ok(super::BOOL(0)), vec![item.clone()]); // 执行失败仍保留成功项目。
    assert_eq!(failed.items, vec![item.clone()]);
    assert_eq!(failed.operation_error, Some(0x80004005_u32 as i32));
    assert!(!failed.aborted);
    let failed_query = super::completed_file_operation(super::ShellOperationKind::Paste, true, false, None, Ok(()), Err(windows::core::Error::from_hresult(super::HRESULT(0x80070005_u32 as i32))), vec![item]); // 查询失败也不能伪装成取消或丢弃文件结果。
    assert_eq!(failed_query.operation_error, Some(0x80070005_u32 as i32));
    assert_eq!(failed_query.items.len(), 1);
    assert!(!failed_query.aborted);
}

/// 普通失败不能压制混合布局，明确取消即使查询返回 false 仍阻止已有项目归属提交。
#[test]
fn cancellation_is_not_inferred_from_ordinary_partial_failure()
{
    let failed = super::completed_file_operation(super::ShellOperationKind::Paste, false, false, None, Err(windows::core::Error::from_hresult(windows::Win32::Foundation::E_ACCESSDENIED)), Ok(super::BOOL(1)), Vec::new());
    assert!(!failed.aborted);
    let cancelled = super::completed_file_operation(super::ShellOperationKind::Paste, false, false, None, Err(windows::core::Error::from_hresult(windows::Win32::UI::Shell::COPYENGINE_E_USER_CANCELLED)), Ok(super::BOOL(0)), Vec::new());
    assert!(cancelled.aborted);
}

/// 补偿源变化时不执行文件动作；目标同名冲突在 Pre 回调拒绝而不是覆盖。
#[test]
fn native_compensation_preserves_changed_source_and_conflicting_destination()
{
    super::initialize_shell_apartment().unwrap();
    let root = std::env::temp_dir().join(format!("desktop-tools-compensation-{}-{}", std::process::id(), crate::tools::time::next_timeline_timestamp()));
    std::fs::create_dir_all(root.join("target")).unwrap();
    let source = root.join("item.txt");
    let destination = root.join("target").join("item.txt");
    std::fs::write(&source, b"before").unwrap();
    std::fs::write(&destination, b"keep target").unwrap();
    let proof = super::shell_file_proof(source.to_str().unwrap()).unwrap();
    {
        let (sink, _) = super::create_operation_sink_with_proof(Some(proof.clone()));
        let item = super::resolve_shell_item(source.to_str().unwrap()).unwrap();
        let folder = super::resolve_shell_item(root.join("target").to_str().unwrap()).unwrap();
        assert!(unsafe { sink.PreMoveItem(0, &item, &folder, super::PCWSTR::null()) }.is_err());
    }
    std::fs::write(&source, b"after!").unwrap();
    {
        let owner = super::ShellMenuOwner::create().unwrap();
        let result = super::compensate_desktop_items(&owner, super::ShellOperationKind::Paste, vec![super::ShellCompensationAction::Move { reference: source.to_string_lossy().into_owned(), destination_parent_reference: root.join("target").to_string_lossy().into_owned(), new_name: "item.txt".into() }], vec![proof]).unwrap();
        assert!(!result.items[0].succeeded);
        assert!(result.items[0].proof.is_none());
        assert_eq!(std::fs::read(&source).unwrap(), b"after!");
        assert_eq!(std::fs::read(&destination).unwrap(), b"keep target");
    }
    std::fs::remove_dir_all(root).unwrap();
    super::uninitialize_shell_apartment();
}

/// 显式原生验收：背景菜单必须由 Explorer 进程创建，测试结束取消菜单。
#[test]
#[ignore = "需要交互式 Windows 桌面，会短暂弹出真实系统菜单"]
fn background_menu_is_owned_by_explorer()
{
    use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, GetWindowThreadProcessId, WM_CANCELMODE};
    super::initialize_shell_apartment().unwrap();
    let desktop = super::super::desktop_host::discover_shell_desktop().unwrap(); // 当前 Explorer 桌面。
    let view = super::HWND(desktop.def_view as *mut std::ffi::c_void); // 真实桌面视图。
    let mut explorer_pid = 0; // 预期菜单所属进程。
    unsafe { GetWindowThreadProcessId(view, Some(&mut explorer_pid)) };
    let mut owner = super::ShellMenuOwner::create().unwrap(); // 用于调用正式共享入口。
    assert_eq!(super::show_native_context_menu(&mut owner, super::NativeMenuTarget::Background, 20, 20, &[]).unwrap(), None);
    let started = std::time::Instant::now(); // 等待异步消息投递和菜单显示。
    let mut observed = false; // 不以成功投递代替菜单实际出现。
    while started.elapsed() < std::time::Duration::from_secs(3)
    {
        let popup = unsafe { FindWindowW(super::w!("#32768"), super::PCWSTR::null()) }.unwrap_or_default(); // Windows 原生菜单窗口类。
        let mut process_id = 0; // 实际弹出窗口所属进程。
        if !popup.is_invalid()
        {
            unsafe { GetWindowThreadProcessId(popup, Some(&mut process_id)) };
        }
        if process_id == explorer_pid
        {
            observed = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    let _ = unsafe { super::PostMessageW(Some(view), WM_CANCELMODE, super::WPARAM(0), super::LPARAM(0)) };
    drop(owner);
    super::uninitialize_shell_apartment();
    assert!(observed, "已投递请求，但没有观察到 Explorer 所属的菜单窗口");
}

/// 覆盖负坐标副屏、纵向副屏与协议边界，避免右键菜单落到错误屏幕。
#[test]
fn explorer_menu_coordinates_preserve_signed_monitors()
{
    for (x, y) in [(-1920, 40), (2560, -1080), (0, 0), (32767, -32768)]
    {
        let packed = super::desktop_menu_coordinates(x, y).unwrap().0 as u32; // 实际发送的消息参数。
        assert_eq!(i32::from(packed as u16 as i16), x);
        assert_eq!(i32::from((packed >> 16) as u16 as i16), y);
    }
    assert!(super::desktop_menu_coordinates(32768, 0).is_err());
}

/// 显式运行的真实菜单循环检查，会短暂显示并自动取消八次菜单。
#[test]
#[ignore = "需要交互式 Windows 桌面，会临时取得前台焦点"]
fn native_menu_reopens_after_cancel()
{
    use windows::Win32::UI::WindowsAndMessaging::{EndMenu, KillTimer, SetTimer};
    unsafe extern "system" fn cancel_menu(_: super::HWND, _: u32, _: usize, _: u32)
    {
        let _ = unsafe { EndMenu() }; // 定时退出当前菜单，模拟取消后的再次打开。
    }
    super::initialize_shell_apartment().unwrap();
    let mut owner = super::ShellMenuOwner::create().unwrap(); // 本 STA 的真实菜单 owner。
    for attempt in 0..8
    {
        let timer = unsafe { SetTimer(Some(owner.hwnd), 1, 150, Some(cancel_menu)) }; // 每轮自动取消。
        assert_ne!(timer, 0);
        let started = std::time::Instant::now(); // 立即返回意味着菜单没有保持打开。
        let result = super::show_native_context_menu(&mut owner, super::NativeMenuTarget::ProductOnly, 20, 20, &[super::NativeMenuCommand { id: 0x7000, label: "菜单循环检查".into(), enabled: true }]);
        unsafe { KillTimer(Some(owner.hwnd), timer) }.unwrap();
        assert_eq!(result.unwrap(), None);
        assert!(started.elapsed().as_millis() >= 100, "第 {attempt} 次菜单提前关闭");
    }
    drop(owner);
    super::uninitialize_shell_apartment();
}

/// 桌面原生设置动词必须出现在生产菜单中，不能退化为普通目录菜单。
#[test]
fn local_background_menu_preserves_desktop_verbs()
{
    use windows::Win32::UI::Shell::GCS_VERBW;
    use windows::Win32::UI::WindowsAndMessaging::{GetMenuItemCount, GetMenuItemID, GetMenuStringW, MF_BYPOSITION};
    super::initialize_shell_apartment().unwrap();
    let (folder, primary, menu) = super::folder_background_menu(super::HWND::default(), std::env::temp_dir().to_str().unwrap()).unwrap(); // 使用生产菜单获取路径。
    let popup = unsafe { super::CreatePopupMenu() }.unwrap(); // 仅枚举，不显示。
    unsafe { primary.QueryContextMenu(popup, 0, 1, 0x4fff, super::CMF_NORMAL).ok() }.unwrap();
    unsafe { menu.QueryContextMenu(popup, GetMenuItemCount(Some(popup)) as u32, 0x5000, 0x6fff, super::CMF_NORMAL).ok() }.unwrap();
    let mut labels = Vec::new(); // 当前系统实际提供的标签。
    let mut verbs = Vec::new(); // 与显示语言无关的原生动词。
    for index in 0..unsafe { GetMenuItemCount(Some(popup)) }
    {
        let mut buffer = [0u16; 256]; // 标签缓冲区。
        let length = unsafe { GetMenuStringW(popup, index as u32, Some(&mut buffer), MF_BYPOSITION) }; // 读取顶层标签。
        labels.push(String::from_utf16_lossy(&buffer[..length.max(0) as usize]));
        let id = unsafe { GetMenuItemID(popup, index) }; // 分隔线及子菜单没有普通命令编号。
        if id != u32::MAX && id > 0
        {
            let (source, first) = if id >= 0x5000 { (&menu, 0x5000) } else { (&primary, 1) }; // 与生产菜单一致的编号空间。
            let mut verb = [0u16; 256]; // GCS_VERBW 返回 UTF-16。
            if unsafe { source.GetCommandString((id - first) as usize, GCS_VERBW, None, windows::core::PSTR(verb.as_mut_ptr().cast()), verb.len() as u32) }.is_ok()
            {
                verbs.push(String::from_utf16_lossy(&verb[..verb.iter().position(|value| *value == 0).unwrap_or(verb.len())]));
            }
        }
    }
    eprintln!("生产桌面菜单: {labels:?}，动词: {verbs:?}");
    assert!(verbs.iter().any(|verb| verb.eq_ignore_ascii_case("display")), "缺少显示设置: {verbs:?}");
    assert!(verbs.iter().any(|verb| verb.eq_ignore_ascii_case("personalize")), "缺少个性化: {verbs:?}");
    unsafe { super::DestroyMenu(popup) }.unwrap();
    drop(menu);
    drop(primary);
    drop(folder);
    super::uninitialize_shell_apartment();
}

/// 实际展开 Shell 新建菜单并创建 TXT，覆盖获取和执行两个阶段。
#[test]
fn local_background_menu_creates_text_document()
{
    use windows::Win32::UI::WindowsAndMessaging::{GetMenuItemCount, GetMenuItemID, GetMenuStringW, GetSubMenu, MF_BYPOSITION};
    super::initialize_shell_apartment().unwrap();
    let directory = std::env::temp_dir().join(format!("desktop-tools-menu-{}", crate::tools::time::next_timeline_timestamp())); // 独占中文目录，用来验证 Unicode 工作目录。
    let directory = directory.join("菜单测试"); // 不接触用户桌面。
    std::fs::create_dir_all(&directory).unwrap();
    let (_folder, menu, desktop_menu) = super::folder_background_menu(super::HWND::default(), directory.to_str().unwrap()).unwrap(); // 与产品相同的获取路径。
    let popup = unsafe { super::CreatePopupMenu() }.unwrap(); // 不显示的真实 HMENU。
    unsafe { menu.QueryContextMenu(popup, 0, 1, 0x6fff, super::CMF_NORMAL).ok() }.unwrap();
    let handler = windows::core::Interface::cast::<super::IContextMenu2>(&menu).unwrap(); // Shell 动态子菜单消息处理器。
    let mut selected = None; // TXT 新建项的真实编号。
    let mut labels = Vec::new(); // 失败时报告实际枚举项。
    for index in 0..unsafe { GetMenuItemCount(Some(popup)) }
    {
        let submenu = unsafe { GetSubMenu(popup, index) }; // 延迟生成的子菜单。
        if submenu.is_invalid()
        {
            continue;
        }
        let _ = unsafe { handler.HandleMenuMsg(super::WM_INITMENUPOPUP, super::WPARAM(submenu.0 as usize), super::LPARAM(index as isize)) };
        for child in 0..unsafe { GetMenuItemCount(Some(submenu)) }
        {
            let mut buffer = [0u16; 256]; // 菜单标签缓冲区。
            let length = unsafe { GetMenuStringW(submenu, child as u32, Some(&mut buffer), MF_BYPOSITION) }; // 本地 Shell 的标签。
            let label = String::from_utf16_lossy(&buffer[..length.max(0) as usize]); // 可检查文本。
            if label.contains("文本文档") || label.contains("Text Document")
            {
                selected = Some(unsafe { GetMenuItemID(submenu, child) });
            }
            labels.push(label);
        }
    }
    let selected = selected.unwrap_or_else(|| panic!("TXT 模板不在实际菜单中: {labels:?}")); // 未提供模板不能记为通过。
    super::invoke_menu_command(&menu, super::HWND::default(), selected - 1, directory.to_str()).unwrap();
    let created = std::fs::read_dir(&directory).unwrap().filter_map(Result::ok).any(|entry| entry.path().extension().is_some_and(|extension| extension == "txt")); // 验证实际文件，不以 HRESULT 成功代替。
    unsafe { super::DestroyMenu(popup) }.unwrap();
    assert!(created, "Shell 返回成功但没有在目标目录创建 TXT");
    drop(handler);
    drop(menu);
    drop(desktop_menu);
    drop(_folder);
    std::fs::remove_dir_all(directory.parent().unwrap()).unwrap();
    super::uninitialize_shell_apartment();
}

/// 在独立临时目录执行原生重命名、旧历史补偿与重做，验证快捷方式类型不丢失。
#[test]
fn native_shortcut_rename_undo_redo_preserves_extension()
{
    super::initialize_shell_apartment().unwrap();
    let directory = std::env::temp_dir().join(format!("desktop-tools-shortcut-{}-{}", std::process::id(), crate::tools::time::next_timeline_timestamp())); // 测试独占目录。
    std::fs::create_dir(&directory).unwrap();
    let original = directory.join("original.lnk"); // 测试文件不在用户桌面。
    std::fs::write(&original, []).unwrap();
    let owner = super::ShellMenuOwner::create().unwrap(); // 本线程原生操作所有者。
    let renamed = super::rename_desktop_item(&owner, original.to_str().unwrap(), "renamed").unwrap(); // 正向名称不含后缀。
    assert_eq!(renamed.items[0].source_name.as_deref(), Some("original.lnk"));
    assert!(directory.join("renamed.lnk").exists());
    let undone = super::compensate_desktop_items(&owner, super::ShellOperationKind::Rename, vec![super::ShellCompensationAction::Rename { reference: directory.join("renamed.lnk").to_string_lossy().into_owned(), new_name: "original".into() }], vec![super::shell_file_proof(directory.join("renamed.lnk").to_str().unwrap()).unwrap()]).unwrap(); // 模拟本会话已经保存隐藏后缀的旧历史。
    assert!(undone.items[0].succeeded, "原生撤销结果：{undone:?}");
    assert!(original.exists());
    super::compensate_desktop_items(&owner, super::ShellOperationKind::Rename, vec![super::ShellCompensationAction::Rename { reference: original.to_string_lossy().into_owned(), new_name: "renamed.lnk".into() }], vec![super::shell_file_proof(original.to_str().unwrap()).unwrap()]).unwrap();
    assert!(directory.join("renamed.lnk").exists());
    drop(owner);
    std::fs::remove_dir_all(directory).unwrap();
    super::uninitialize_shell_apartment();
}
/// 门户传入非桌面路径时仍执行原生重命名与回收，不永久删除。
#[test]
fn native_portal_rename_and_recycle()
{
    super::initialize_shell_apartment().unwrap();
    let directory = std::env::temp_dir().join(format!("desktop-tools-portal-native-{}", crate::tools::time::next_timeline_timestamp())); // 独占夹具目录。
    std::fs::create_dir(&directory).unwrap();
    let source = directory.join("原名.txt"); // 不接触用户数据。
    std::fs::write(&source, "portal-test").unwrap();
    let owner = super::ShellMenuOwner::create().unwrap(); // 与产品相同的原生 owner。
    let renamed = super::rename_desktop_item(&owner, source.to_str().unwrap(), "新名.txt").unwrap();
    assert!(renamed.items.iter().any(|item| item.succeeded));
    let target = directory.join("新名.txt"); // 实际系统改名结果。
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "portal-test");
    let recycled = super::delete_desktop_items(&owner, &[target.to_string_lossy().into_owned()], false).unwrap();
    assert_eq!(recycled.kind, super::ShellOperationKind::Recycle);
    assert!(!recycled.permanent);
    assert!(!recycled.aborted);
    assert!(recycled.items.iter().any(|item| item.succeeded && item.result_reference.is_some()));
    assert!(!target.exists());
    drop(owner);
    std::fs::remove_dir(&directory).unwrap();
    super::uninitialize_shell_apartment();
}

/// 真实门户多选菜单经过现有 owner 消息循环显示并自动取消。
#[test]
#[ignore = "需要交互式桌面，会短暂显示门户原生菜单"]
fn native_portal_menu_opens_and_cancels()
{
    use windows::Win32::UI::WindowsAndMessaging::{EndMenu, KillTimer, SetTimer};
    unsafe extern "system" fn cancel_menu(_: super::HWND, _: u32, _: usize, _: u32) { let _ = unsafe { EndMenu() }; }
    super::initialize_shell_apartment().unwrap();
    let directory = std::env::temp_dir().join(format!("desktop-tools-portal-menu-{}", crate::tools::time::next_timeline_timestamp())); // 独占测试目录。
    std::fs::create_dir(&directory).unwrap();
    let paths = [directory.join("一.txt"), directory.join("二.txt")]; // 真实同目录多选。
    for path in &paths
    {
        std::fs::write(path, "menu-test").unwrap();
    }
    let mut owner = super::ShellMenuOwner::create().unwrap(); // 复用产品菜单 owner。
    let timer = unsafe { SetTimer(Some(owner.hwnd), 1, 500, Some(cancel_menu)) }; // 自动关闭，不调用菜单动作。
    assert_ne!(timer, 0);
    let started = std::time::Instant::now(); // 验证菜单确实进入消息循环。
    let result = super::show_native_context_menu(&mut owner, super::NativeMenuTarget::FolderItems(paths.iter().map(|path| path.to_string_lossy().into_owned()).collect()), 40, 40, &[]);
    unsafe { KillTimer(Some(owner.hwnd), timer) }.unwrap();
    assert_eq!(result.unwrap(), None);
    assert!(started.elapsed().as_millis() >= 100);
    drop(owner);
    for path in paths
    {
        std::fs::remove_file(path).unwrap();
    }
    std::fs::remove_dir(directory).unwrap();
    super::uninitialize_shell_apartment();
}

/// 隐藏快捷方式后缀不应在用户只修改显示名称时丢失。
#[test]
fn shortcut_rename_preserves_hidden_extension()
{
    assert_eq!(super::shortcut_rename_name(Some(r"C:\Desktop\old.lnk"), "新名称"), "新名称.lnk");
    assert_eq!(super::shortcut_rename_name(Some(r"C:\Desktop\old.LNK"), "new.lnk"), "new.lnk");
    assert_eq!(super::shortcut_rename_name(Some(r"C:\Desktop\old.url"), "new"), "new.url");
    assert_eq!(super::shortcut_rename_name(Some(r"C:\Desktop\old.txt"), "new.md"), "new.md");
    assert_eq!(super::shortcut_rename_name(None, "new"), "new");
}
use super::{encoded_pidl_reference, parse_desktop_item_reference};

/// 半透明 HICON 经 PNG 编码后保持原色，避免浏览器再次乘 Alpha 导致边缘变暗。
#[test]
fn icon_png_preserves_translucent_colors()
{
    use base64::Engine;
    use windows::Win32::UI::WindowsAndMessaging::{CreateIcon, DestroyIcon};

    let pixels = [0_u8, 128, 255, 128].repeat(48 * 48); // CreateIcon 接收的直通 Alpha BGRA 橙色图标。
    let mask = [0_u8; 48 * 48 / 8]; // 32 位图标使用 Alpha，不额外裁切像素。
    let icon = unsafe { CreateIcon(None, 48, 48, 1, 32, mask.as_ptr(), pixels.as_ptr()) }.expect("测试图标应创建成功"); // 当前测试独占的原生图标。
    let encoded = unsafe { super::icon_data_url(icon) }; // 经过正式 GDI 绘制与 PNG 编码路径。
    unsafe { DestroyIcon(icon).expect("测试图标应释放") };
    let encoded = encoded.expect("图标应编码成功"); // 待解码验证的 data URL。
    let bytes = base64::engine::general_purpose::STANDARD.decode(encoded.strip_prefix("data:image/png;base64,").unwrap()).unwrap(); // PNG 文件字节。
    let mut reader = png::Decoder::new(std::io::Cursor::new(bytes)).read_info().unwrap(); // 原始 RGBA 解码器。
    let mut rgba = vec![0; reader.output_buffer_size()]; // 解码输出缓冲。
    let info = reader.next_frame(&mut rgba).unwrap(); // 本次解码的像素范围。
    for pixel in rgba[..info.buffer_size()].chunks_exact(4)
    {
        assert_eq!(pixel, [255, 128, 0, 128]);
    }
}

/// 验证 PIDL 稳定引用可解码并保持原始字节。
#[test]
fn encoded_pidl_reference_round_trips()
{
    // 表示桌面根节点的终止 PIDL。
    let bytes = [0_u8, 0];
    // 编码后的 PIDL 稳定引用。
    let reference = encoded_pidl_reference(bytes.as_ptr().cast());
    // 从稳定引用重新解析出的 PIDL。
    let parsed = parse_desktop_item_reference(&reference).expect("PIDL 稳定引用应可重新解析");

    assert_eq!(encoded_pidl_reference(parsed.0), reference);
}
