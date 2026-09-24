use super::*;
use windows::Win32::System::Ole::DROPEFFECT_COPY;

/// 验证未知、过期、跨屏及错误坐标的渲染确认不能授权投放。
#[test]
fn preview_requires_live_session_and_matching_view()
{
    let monitor = crate::contracts::DesktopHostMonitor { id: "preview-screen".into(), name: "preview-screen".into(), primary: false, x: -1000, y: 0, width: 1000, height: 800, work_x: -1000, work_y: 0, work_width: 1000, work_height: 800, scale_factor: 2.0, dpi: 192 }; // 负坐标与混合 DPI。
    let point = POINTL { x: -970, y: 60 }; // 对应 DIP (15, 30)。
    let preview = DropPreview { session: "preview-test".into(), sequence: 2, geometry_revision: 1, revision: 7, monitor_id: monitor.id.clone(), pointer: crate::contracts::WorkspacePoint { left: 15.0, top: 30.0 }, regions: vec![crate::contracts::HitRegion { id: "background".into(), kind: "workspace-background".into(), left: 0.0, top: 0.0, width: 500.0, height: 400.0, priority: 1, target_path: None }], fence_scrolls: Vec::new() }; // 已呈现的合法几何。
    publish_preview(preview.clone()).unwrap();
    assert!(confirmed_preview(&preview.session, 2, 7, &monitor, &point).is_none());
    set_preview_session(&preview.session, true);
    publish_preview(preview.clone()).unwrap();
    assert!(confirmed_preview(&preview.session, 2, 7, &monitor, &point).is_some());
    assert!(confirmed_preview(&preview.session, 1, 7, &monitor, &point).is_none());
    assert!(confirmed_preview(&preview.session, 3, 7, &monitor, &point).is_some());
    assert!(confirmed_preview(&preview.session, 2, 8, &monitor, &point).is_none());
    assert!(confirmed_preview(&preview.session, 2, 7, &monitor, &POINTL { x: -968, y: 60 }).is_some());
    PREVIEW_CHECKS.with(|checks| assert_eq!(checks.borrow().get(&preview.session).unwrap(), &[3, 0, 1, 1, 0, 0, 0])); // 旧序号与实时坐标均不再单独拒绝。
    let mut older = preview.clone(); // 迟到帧不能覆盖已确认的新序号。
    older.sequence = 1;
    publish_preview(older).unwrap();
    assert!(confirmed_preview(&preview.session, 2, 7, &monitor, &point).is_some());
    let mut invalid = preview.clone(); // 有效序号也不能授权越界几何。
    invalid.regions[0].left = -1.0;
    publish_preview(invalid).unwrap();
    assert!(confirmed_preview(&preview.session, 2, 7, &monitor, &point).is_none());
    set_preview_session(&preview.session, false);
    publish_preview(preview.clone()).unwrap();
    assert!(confirmed_preview(&preview.session, 2, 7, &monitor, &point).is_none());
}

/// 真实 COM 回调验证无 Enter 的释放被拒绝，重复 Leave 不创建虚假会话。
#[test]
fn inactive_target_rejects_drop_and_ignores_leave()
{
    let calls = Rc::new(RefCell::new(0)); // 记录业务处理调用次数。
    let recorded = Rc::clone(&calls); // 回调持有的计数器。
    let target = create_drop_target(
        None,
        Rc::new(move |_| {
            *recorded.borrow_mut() += 1;
            Ok(DROPEFFECT_COPY)
        }),
    ); // 没有启动会话的真实 COM 对象。
    let mut effect = DROPEFFECT_COPY; // 模拟源允许复制。
    unsafe {
        target.DragOver(MODIFIERKEYS_FLAGS(0), POINTL::default(), &mut effect).unwrap();
        assert_eq!(effect, DROPEFFECT_NONE);
        target.DragLeave().unwrap();
        target.DragLeave().unwrap();
        target.Drop(None, MODIFIERKEYS_FLAGS(0), POINTL::default(), &mut effect).unwrap();
    }
    assert_eq!(*calls.borrow(), 0);
}

/// 真实数据对象验证效果协商、释放失败与一次性清理。
#[test]
fn failed_drop_cleans_active_session_once()
{
    use windows::Win32::Foundation::E_FAIL;
    use windows::Win32::System::Ole::{DROPEFFECT_MOVE, OleInitialize, OleUninitialize};
    use windows::Win32::UI::Shell::{BHID_DataObject, IShellItem, SHCreateItemFromParsingName};
    unsafe { OleInitialize(None) }.unwrap();
    {
        let item: IShellItem = unsafe { SHCreateItemFromParsingName(&windows::core::HSTRING::from(std::env::temp_dir().to_string_lossy().as_ref()), None) }.unwrap(); // 只读外部目录的数据提供方。
        let data: IDataObject = unsafe { item.BindToHandler(None, &BHID_DataObject) }.unwrap(); // 真实 Shell 数据对象。
        assert!(filesystem_drop(&data));
        let pointer_offset = crate::contracts::WorkspacePoint { left: 12.0, top: 24.0 }; // 指针偏移同样跟随可信源会话。
        let session = SourceDragGuard::register(&data, &["source".to_string()], pointer_offset, false).unwrap(); // 真实源登记同时验证跨 STA 身份和内部反馈。
        assert!(SourceDragGuard::register(&data, &["second".to_string()], pointer_offset, false).is_err());
        let other_data: IDataObject = unsafe { item.BindToHandler(None, &BHID_DataObject) }.unwrap(); // 相同来源内容不能代替已登记 COM 身份。
        assert!(source_drag_session(&other_data).is_none());
        let agile = windows::core::AgileReference::new(&data).unwrap(); // 真实跨 apartment 代理。
        let (sender, receiver) = std::sync::mpsc::channel(); // 使用与正式宿主相同的消息泵等待。
        let worker = std::thread::spawn(move || {
            unsafe { OleInitialize(None) }.unwrap();
            {
                let proxy = agile.resolve().unwrap(); // 只在目标 STA 使用解析出的接口。
                assert!(filesystem_drop(&proxy));
                let session = source_drag_session(&proxy).expect("合法跨 STA 代理应匹配源身份"); // 必须在当前 apartment 比较 IUnknown。
                assert_eq!(session.item_ids, ["source"]);
                assert_eq!(session.pointer_offset, pointer_offset);
                session.layout_committed.store(true, std::sync::atomic::Ordering::Release);
            }
            drop(agile); // 完成通知前释放所有代理，主线程仍持续泵消息。
            unsafe { OleUninitialize() };
            sender.send(Err("封送已验证".to_string())).unwrap();
        }); // 不执行任何文件传输。
        assert_eq!(wait_transfer(receiver).unwrap_err(), "封送已验证");
        worker.join().unwrap();
        assert!(session.layout_committed());
        assert!(source_drag_session(&data).is_some());
        drop(session);
        assert!(source_drag_session(&data).is_none());
        let phases = Rc::new(RefCell::new(Vec::new())); // 验证每阶段只通知一次。
        let recorded = Rc::clone(&phases); // 回调共享的检查记录。
        let target = create_drop_target(
            None,
            Rc::new(move |phase| match phase
            {
                DropPhase::Enter(data, keys, point, allowed) =>
                {
                    assert!(!windows::core::Interface::as_raw(data).is_null());
                    assert_eq!(keys.0, 0);
                    assert_eq!(point.x, -100);
                    assert_eq!(allowed, DROPEFFECT_MOVE);
                    recorded.borrow_mut().push("enter");
                    Ok(DROPEFFECT_COPY)
                }
                DropPhase::Over(keys, point, allowed) =>
                {
                    assert_eq!(keys.0, 0);
                    assert_eq!(point.x, -100);
                    recorded.borrow_mut().push("over");
                    Ok(allowed)
                }
                DropPhase::Drop(data, keys, point, allowed) =>
                {
                    assert!(!windows::core::Interface::as_raw(data).is_null());
                    assert_eq!(keys.0, 0);
                    assert_eq!(point.x, -100);
                    assert_eq!(allowed, DROPEFFECT_MOVE);
                    recorded.borrow_mut().push("drop");
                    Err(E_FAIL.into())
                }
                DropPhase::Leave =>
                {
                    recorded.borrow_mut().push("leave");
                    Ok(DROPEFFECT_NONE)
                }
            }),
        ); // 业务失败模拟，不执行磁盘操作。
        let point = POINTL { x: -100, y: 0 }; // 保留负屏幕坐标。
        let mut effect = DROPEFFECT_MOVE; // 源不允许复制。
        unsafe {
            target.DragEnter(&data, MODIFIERKEYS_FLAGS(0), point, &mut effect).unwrap();
            assert_eq!(effect, DROPEFFECT_NONE);
            effect = DROPEFFECT_MOVE;
            target.DragOver(MODIFIERKEYS_FLAGS(0), point, &mut effect).unwrap();
            assert_eq!(effect, DROPEFFECT_MOVE);
            assert!(target.Drop(&data, MODIFIERKEYS_FLAGS(0), point, &mut effect).is_err());
            assert_eq!(effect, DROPEFFECT_NONE);
            target.DragLeave().unwrap();
        }
        assert_eq!(*phases.borrow(), ["enter", "over", "drop", "leave"]);
    }
    unsafe { OleUninitialize() };
}
