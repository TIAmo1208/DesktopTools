use super::*;

/// 使用隔离的临时注册表验证通知范围和目标键删除、重建，不修改系统配置。
#[test]
fn config_watch_ignores_unrelated_values_and_tracks_key_lifecycle()
{
    use windows::Win32::Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT};
    use windows::Win32::System::Registry::{KEY_ALL_ACCESS, REG_DWORD, REG_OPTION_VOLATILE, RegCreateKeyExW, RegDeleteKeyW, RegDeleteTreeW, RegSetKeyValueW};
    use windows::Win32::System::Threading::WaitForSingleObject;

    // 创建测试键并交由调用方关闭，监听器单独持有只读句柄。
    let create_key = |root, path: &str| {
        let mut key = HKEY::default(); // 临时创建句柄。
        unsafe {
            RegCreateKeyExW(root, &HSTRING::from(path), None, PCWSTR::null(), REG_OPTION_VOLATILE, KEY_ALL_ACCESS, None, &mut key, None).ok().unwrap();
        }
        key
    };
    /// 即使断言失败，也清除本测试独占的注册表子树。
    struct TestRegistry
    {
        key: HKEY,    // 独占测试根句柄。
        path: String, // 当前用户下的唯一测试路径。
    }
    impl Drop for TestRegistry
    {
        /// 释放测试子树和根键，不触及实际 Explorer 配置。
        fn drop(&mut self)
        {
            unsafe {
                let _ = RegDeleteTreeW(self.key, PCWSTR::null());
                let _ = RegCloseKey(self.key);
                let _ = RegDeleteKeyW(HKEY_CURRENT_USER, &HSTRING::from(&self.path));
            }
        }
    }
    let path = format!(r"Software\DesktopToolsWatchTest-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()); // 并行测试独占路径。
    let registry = TestRegistry { key: create_key(HKEY_CURRENT_USER, &path), path }; // 自动清理测试根。
    unsafe { RegCloseKey(create_key(registry.key, "Unrelated")).ok().unwrap() };
    let mut watch = SourceConfigWatch { root: registry.key, path: r"Unrelated\Target\Leaf", key: HKEY::default(), event: unsafe { CreateEventW(None, false, false, PCWSTR::null()) }.unwrap() }; // 初始目标缺失，先监听最近已存在祖先的直接子键结构。
    watch.rearm().unwrap();
    // 测试根和最近祖先的值更新都不能唤醒监听器。
    for path in ["", "Unrelated"]
    {
        unsafe {
            RegSetKeyValueW(registry.key, &HSTRING::from(path), &HSTRING::from("Value"), REG_DWORD.0, Some(1_u32.to_le_bytes().as_ptr().cast()), 4).ok().unwrap();
            assert_eq!(WaitForSingleObject(watch.event, 50), WAIT_TIMEOUT);
        }
    }
    // 分段创建目标路径，验证监听逐步从祖先收窄到叶子。
    for path in [r"Unrelated\Target", r"Unrelated\Target\Leaf"]
    {
        unsafe {
            RegCloseKey(create_key(registry.key, path)).ok().unwrap();
            assert_eq!(WaitForSingleObject(watch.event, 1000), WAIT_OBJECT_0);
        }
        watch.rearm().unwrap();
    }
    unsafe {
        RegCloseKey(create_key(registry.key, r"Unrelated\Target\Leaf\Child")).ok().unwrap();
        assert_eq!(WaitForSingleObject(watch.event, 50), WAIT_TIMEOUT);
        RegSetKeyValueW(registry.key, &HSTRING::from(r"Unrelated\Target\Leaf\Child"), &HSTRING::from("Value"), REG_DWORD.0, Some(1_u32.to_le_bytes().as_ptr().cast()), 4).ok().unwrap();
        assert_eq!(WaitForSingleObject(watch.event, 50), WAIT_TIMEOUT);
        RegSetKeyValueW(registry.key, &HSTRING::from(r"Unrelated\Target\Leaf"), &HSTRING::from("Value"), REG_DWORD.0, Some(1_u32.to_le_bytes().as_ptr().cast()), 4).ok().unwrap();
        assert_eq!(WaitForSingleObject(watch.event, 1000), WAIT_OBJECT_0);
    }
    watch.rearm().unwrap();
    // 删除目标后回退监听父键，重建后恢复目标键值通知。
    unsafe {
        RegDeleteTreeW(registry.key, &HSTRING::from(r"Unrelated\Target\Leaf")).ok().unwrap();
        assert_eq!(WaitForSingleObject(watch.event, 1000), WAIT_OBJECT_0);
    }
    watch.rearm().unwrap();
    unsafe {
        RegCloseKey(create_key(registry.key, r"Unrelated\Target\Leaf")).ok().unwrap();
        assert_eq!(WaitForSingleObject(watch.event, 1000), WAIT_OBJECT_0);
    }
    watch.rearm().unwrap();
    unsafe {
        RegSetKeyValueW(registry.key, &HSTRING::from(r"Unrelated\Target\Leaf"), &HSTRING::from("Value"), REG_DWORD.0, Some(0_u32.to_le_bytes().as_ptr().cast()), 4).ok().unwrap();
        assert_eq!(WaitForSingleObject(watch.event, 1000), WAIT_OBJECT_0);
    }
}

/// 验证完整三层优先级、普通图标缺省隐藏和错误不伪装为缺省。
#[test]
fn explicit_precedence_and_missing_values()
{
    for user in [None, Some(false), Some(true)]
    {
        for policy in [None, Some(false), Some(true)]
        {
            for system in [None, Some(false), Some(true)]
            {
                let layers = [user, policy, system]; // 所有显式值和缺省组合。
                assert_eq!(resolve_visible(&FOLDERID_ComputerFolder, |index| Ok(layers[index])).unwrap(), !user.or(policy).or(system).unwrap_or(true));
            }
        }
    }
    assert!(resolve_visible(&FOLDERID_ComputerFolder, |_| Err("拒绝访问".into())).is_err());
    assert!(resolve_visible(&FOLDERID_ComputerFolder, |index| if index == 0 { Ok(Some(false)) } else { panic!("不读取低优先级配置") }).unwrap());
}

/// 回收站仅在全部缺省时显示，任意有效层的显式隐藏及读取错误仍生效。
#[test]
fn recycle_bin_default_does_not_override_explicit_configuration()
{
    for (folder, _) in SYSTEM_ICONS
    {
        assert_eq!(resolve_visible(&folder, |_| Ok(None)).unwrap(), folder == FOLDERID_RecycleBinFolder);
    }
    for user in [None, Some(false), Some(true)]
    {
        for policy in [None, Some(false), Some(true)]
        {
            for system in [None, Some(false), Some(true)]
            {
                let layers = [user, policy, system]; // 回收站的全部显式配置与缺省组合。
                assert_eq!(resolve_visible(&FOLDERID_RecycleBinFolder, |index| Ok(layers[index])).unwrap(), !user.or(policy).or(system).unwrap_or(false));
            }
        }
    }
    for failed_layer in 0..3
    {
        assert!(resolve_visible(&FOLDERID_RecycleBinFolder, |index| { if index == failed_layer { Err("配置读取失败".into()) } else { Ok(None) } }).is_err());
    }
}
