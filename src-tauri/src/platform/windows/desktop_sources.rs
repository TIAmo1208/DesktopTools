use windows::Win32::Foundation::{CloseHandle, ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, HANDLE};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_NOTIFY, KEY_WOW64_64KEY,
    REG_NOTIFY_CHANGE_LAST_SET, REG_NOTIFY_CHANGE_NAME, RRF_RT_REG_DWORD, RRF_SUBKEY_WOW6464KEY,
    RegCloseKey, RegGetValueW, RegNotifyChangeKeyValue, RegOpenKeyExW,
};
use windows::Win32::System::Threading::CreateEventW;
use windows::Win32::UI::Shell::{
    FOLDERID_ComputerFolder, FOLDERID_ControlPanelFolder, FOLDERID_NetworkFolder,
    FOLDERID_RecycleBinFolder, FOLDERID_UsersFiles,
};
use windows::core::{GUID, HSTRING, PCWSTR};

/// Known Folder 与注册表 Shell CLSID 分开存储，不按显示名称判断来源。
pub(super) const SYSTEM_ICONS: [(GUID, &str); 5] = [
    (
        FOLDERID_ComputerFolder,
        "{20D04FE0-3AEA-1069-A2D8-08002B30309D}",
    ),
    (
        FOLDERID_UsersFiles,
        "{59031A47-3F72-44A7-89C5-5595FE6B30EE}",
    ),
    (
        FOLDERID_NetworkFolder,
        "{F02C1A0D-BE21-4350-88B0-7367FC96EF3C}",
    ),
    (
        FOLDERID_RecycleBinFolder,
        "{645FF040-5081-101B-9F08-00AA002F954E}",
    ),
    (
        FOLDERID_ControlPanelFolder,
        "{5399E694-6CE5-4D6C-8FCE-1D8870FDCBA0}",
    ),
];

// Windows 10/11 的桌面图标显隐兼容配置，不读取 Explorer 临时总隐藏开关。
const DISPLAY_KEY: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\HideDesktopIcons\NewStartPanel";
// 五类图标使用的组策略位置。
const POLICY_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Policies\NonEnum";
// 网络图标还受独立的桌面网络策略约束。
const EXPLORER_POLICY_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Policies\Explorer";

/// 只接受明确 DWORD 开关；缺失与读取错误不能混为一谈。
fn read_hidden(root: HKEY, key: &str, name: &str) -> Result<Option<bool>, String> {
    let mut value = 0_u32; // 注册表返回的隐藏开关。
    let mut size = size_of::<u32>() as u32; // DWORD 缓冲区大小。
    let status = unsafe {
        RegGetValueW(
            root,
            &HSTRING::from(key),
            &HSTRING::from(name),
            RRF_RT_REG_DWORD | RRF_SUBKEY_WOW6464KEY,
            None,
            Some((&mut value as *mut u32).cast()),
            Some(&mut size),
        )
    }; // 只读 64 位 Windows Shell 配置。
    if status == ERROR_FILE_NOT_FOUND || status == ERROR_PATH_NOT_FOUND {
        return Ok(None);
    }
    status
        .ok()
        .map_err(|error| format!("读取桌面图标配置失败，键：{key}，值：{name}，错误：{error}"))?;
    if size != 4 || value > 1 {
        return Err(format!(
            "桌面图标配置必须为 DWORD 0 或 1，键：{key}，值：{name}"
        ));
    }
    Ok(Some(value != 0))
}

/// 按用户确认的优先级延迟读取，低优先级配置不能覆盖显式用户选择。
fn resolve_visible(
    folder: &GUID,
    mut read: impl FnMut(usize) -> Result<Option<bool>, String>,
) -> Result<bool, String> {
    for layer in 0..3 {
        if let Some(hidden) = read(layer)? {
            return Ok(!hidden);
        }
    }
    Ok(*folder == FOLDERID_RecycleBinFolder)
}

/// 用户配置、组策略、系统配置依次回退；全部缺省时仅回收站显示。
pub(super) fn visible_icons() -> Result<[bool; 5], String> {
    let mut visible = [false; 5]; // 一次完整读取后的五类图标状态。
    for (index, (folder, clsid)) in SYSTEM_ICONS.iter().enumerate() {
        visible[index] = resolve_visible(folder, |layer| match layer {
            0 => read_hidden(HKEY_CURRENT_USER, DISPLAY_KEY, clsid),
            1 => {
                // 同一策略先取机器配置，未配置时取用户策略；网络两种限制有一项启用即隐藏。
                let non_enum = match read_hidden(HKEY_LOCAL_MACHINE, POLICY_KEY, clsid)? {
                    Some(value) => Some(value),
                    None => read_hidden(HKEY_CURRENT_USER, POLICY_KEY, clsid)?,
                }; // 五类对象的命名空间策略。
                if index != 2 {
                    return Ok(non_enum);
                }
                let network =
                    match read_hidden(HKEY_LOCAL_MACHINE, EXPLORER_POLICY_KEY, "NoNetHood")? {
                        Some(value) => Some(value),
                        None => read_hidden(HKEY_CURRENT_USER, EXPLORER_POLICY_KEY, "NoNetHood")?,
                    }; // 网络专用策略。
                Ok(match (non_enum, network) {
                    (Some(a), Some(b)) => Some(a || b),
                    (a, b) => a.or(b),
                })
            }
            _ => read_hidden(HKEY_LOCAL_MACHINE, DISPLAY_KEY, clsid),
        })?;
    }
    Ok(visible)
}

/// 持有只读注册表键与异步通知事件，由 Shell 现有消息线程泵送。
pub(super) struct SourceConfigWatch {
    root: HKEY,         // 用户或机器配置根。
    path: &'static str, // 图标显隐或策略值所在的目标键。
    key: HKEY,          // 当前可用的最近祖先键。
    pub event: HANDLE,  // 自动复位的注册表通知事件。
}

impl SourceConfigWatch {
    /// 只监听实际读取的三个配置键，分别覆盖用户配置与机器配置。
    pub fn create_all() -> Result<Vec<Self>, String> {
        let mut watches = Vec::new(); // 创建失败时自动释放已建立的订阅。
        for root in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
            for path in [DISPLAY_KEY, POLICY_KEY, EXPLORER_POLICY_KEY] {
                let event = unsafe { CreateEventW(None, false, false, PCWSTR::null()) }
                    .map_err(|error| format!("创建桌面配置通知事件失败：{error}"))?; // 系统变化通知句柄。
                let mut watch = Self {
                    root,
                    path,
                    key: HKEY::default(),
                    event,
                }; // 尚未绑定注册表键的监听器。
                watch.rearm()?;
                watches.push(watch);
            }
        }
        Ok(watches)
    }

    /// 每次通知后重新打开并注册，覆盖删除、重建和原先不存在的键。
    pub fn rearm(&mut self) -> Result<(), String> {
        if !self.key.is_invalid() {
            let _ = unsafe { RegCloseKey(self.key) };
        }
        self.key = HKEY::default();
        let mut path = self.path; // 当前尝试打开的最近祖先。
        loop {
            let status = unsafe {
                RegOpenKeyExW(
                    self.root,
                    &HSTRING::from(path),
                    None,
                    KEY_NOTIFY | KEY_WOW64_64KEY,
                    &mut self.key,
                )
            }; // 不创建、不写入注册表。
            if status != ERROR_FILE_NOT_FOUND && status != ERROR_PATH_NOT_FOUND {
                status
                    .ok()
                    .map_err(|error| format!("监听桌面配置失败，键：{path}，错误：{error}"))?;
                break;
            }
            path = path
                .rsplit_once('\\')
                .map(|(parent, _)| parent)
                .ok_or_else(|| format!("桌面配置监听缺少可用祖先：{}", self.path))?;
        }
        // 目标存在时只接收本键值变化；缺失时只接收最近祖先的直接子键增删。
        let filter = if path == self.path {
            REG_NOTIFY_CHANGE_LAST_SET
        } else {
            REG_NOTIFY_CHANGE_NAME
        }; // 不递归监听 Explorer 子树，仍能逐层发现目标键的创建。
        unsafe { RegNotifyChangeKeyValue(self.key, false, filter, Some(self.event), true) }
            .ok()
            .map_err(|error| format!("注册桌面配置变化通知失败：{error}"))
    }
}

impl Drop for SourceConfigWatch {
    /// 先解除键通知再关闭事件，保持句柄生命周期完整。
    fn drop(&mut self) {
        if !self.key.is_invalid() {
            let _ = unsafe { RegCloseKey(self.key) };
        }
        let _ = unsafe { CloseHandle(self.event) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 使用隔离的临时注册表验证通知范围和目标键删除、重建，不修改系统配置。
    #[test]
    fn config_watch_ignores_unrelated_values_and_tracks_key_lifecycle() {
        use windows::Win32::Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT};
        use windows::Win32::System::Registry::{
            KEY_ALL_ACCESS, REG_DWORD, REG_OPTION_VOLATILE, RegCreateKeyExW, RegDeleteKeyW,
            RegDeleteTreeW, RegSetKeyValueW,
        };
        use windows::Win32::System::Threading::WaitForSingleObject;

        // 创建测试键并交由调用方关闭，监听器单独持有只读句柄。
        let create_key = |root, path: &str| {
            let mut key = HKEY::default(); // 临时创建句柄。
            unsafe {
                RegCreateKeyExW(
                    root,
                    &HSTRING::from(path),
                    None,
                    PCWSTR::null(),
                    REG_OPTION_VOLATILE,
                    KEY_ALL_ACCESS,
                    None,
                    &mut key,
                    None,
                )
                .ok()
                .unwrap();
            }
            key
        };
        /// 即使断言失败，也清除本测试独占的注册表子树。
        struct TestRegistry {
            key: HKEY,    // 独占测试根句柄。
            path: String, // 当前用户下的唯一测试路径。
        }
        impl Drop for TestRegistry {
            /// 释放测试子树和根键，不触及实际 Explorer 配置。
            fn drop(&mut self) {
                unsafe {
                    let _ = RegDeleteTreeW(self.key, PCWSTR::null());
                    let _ = RegCloseKey(self.key);
                    let _ = RegDeleteKeyW(HKEY_CURRENT_USER, &HSTRING::from(&self.path));
                }
            }
        }
        let path = format!(
            r"Software\DesktopToolsWatchTest-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ); // 并行测试独占路径。
        let registry = TestRegistry {
            key: create_key(HKEY_CURRENT_USER, &path),
            path,
        }; // 自动清理测试根。
        unsafe {
            RegCloseKey(create_key(registry.key, "Unrelated"))
                .ok()
                .unwrap()
        };
        let mut watch = SourceConfigWatch {
            root: registry.key,
            path: r"Unrelated\Target\Leaf",
            key: HKEY::default(),
            event: unsafe { CreateEventW(None, false, false, PCWSTR::null()) }.unwrap(),
        }; // 初始目标缺失，先监听最近已存在祖先的直接子键结构。
        watch.rearm().unwrap();
        // 测试根和最近祖先的值更新都不能唤醒监听器。
        for path in ["", "Unrelated"] {
            unsafe {
                RegSetKeyValueW(
                    registry.key,
                    &HSTRING::from(path),
                    &HSTRING::from("Value"),
                    REG_DWORD.0,
                    Some(1_u32.to_le_bytes().as_ptr().cast()),
                    4,
                )
                .ok()
                .unwrap();
                assert_eq!(WaitForSingleObject(watch.event, 50), WAIT_TIMEOUT);
            }
        }
        // 分段创建目标路径，验证监听逐步从祖先收窄到叶子。
        for path in [r"Unrelated\Target", r"Unrelated\Target\Leaf"] {
            unsafe {
                RegCloseKey(create_key(registry.key, path)).ok().unwrap();
                assert_eq!(WaitForSingleObject(watch.event, 1000), WAIT_OBJECT_0);
            }
            watch.rearm().unwrap();
        }
        unsafe {
            RegCloseKey(create_key(registry.key, r"Unrelated\Target\Leaf\Child"))
                .ok()
                .unwrap();
            assert_eq!(WaitForSingleObject(watch.event, 50), WAIT_TIMEOUT);
            RegSetKeyValueW(
                registry.key,
                &HSTRING::from(r"Unrelated\Target\Leaf\Child"),
                &HSTRING::from("Value"),
                REG_DWORD.0,
                Some(1_u32.to_le_bytes().as_ptr().cast()),
                4,
            )
            .ok()
            .unwrap();
            assert_eq!(WaitForSingleObject(watch.event, 50), WAIT_TIMEOUT);
            RegSetKeyValueW(
                registry.key,
                &HSTRING::from(r"Unrelated\Target\Leaf"),
                &HSTRING::from("Value"),
                REG_DWORD.0,
                Some(1_u32.to_le_bytes().as_ptr().cast()),
                4,
            )
            .ok()
            .unwrap();
            assert_eq!(WaitForSingleObject(watch.event, 1000), WAIT_OBJECT_0);
        }
        watch.rearm().unwrap();
        // 删除目标后回退监听父键，重建后恢复目标键值通知。
        unsafe {
            RegDeleteTreeW(registry.key, &HSTRING::from(r"Unrelated\Target\Leaf"))
                .ok()
                .unwrap();
            assert_eq!(WaitForSingleObject(watch.event, 1000), WAIT_OBJECT_0);
        }
        watch.rearm().unwrap();
        unsafe {
            RegCloseKey(create_key(registry.key, r"Unrelated\Target\Leaf"))
                .ok()
                .unwrap();
            assert_eq!(WaitForSingleObject(watch.event, 1000), WAIT_OBJECT_0);
        }
        watch.rearm().unwrap();
        unsafe {
            RegSetKeyValueW(
                registry.key,
                &HSTRING::from(r"Unrelated\Target\Leaf"),
                &HSTRING::from("Value"),
                REG_DWORD.0,
                Some(0_u32.to_le_bytes().as_ptr().cast()),
                4,
            )
            .ok()
            .unwrap();
            assert_eq!(WaitForSingleObject(watch.event, 1000), WAIT_OBJECT_0);
        }
    }

    /// 验证完整三层优先级、普通图标缺省隐藏和错误不伪装为缺省。
    #[test]
    fn explicit_precedence_and_missing_values() {
        for user in [None, Some(false), Some(true)] {
            for policy in [None, Some(false), Some(true)] {
                for system in [None, Some(false), Some(true)] {
                    let layers = [user, policy, system]; // 所有显式值和缺省组合。
                    assert_eq!(
                        resolve_visible(&FOLDERID_ComputerFolder, |index| Ok(layers[index]))
                            .unwrap(),
                        !user.or(policy).or(system).unwrap_or(true)
                    );
                }
            }
        }
        assert!(resolve_visible(&FOLDERID_ComputerFolder, |_| Err("拒绝访问".into())).is_err());
        assert!(
            resolve_visible(&FOLDERID_ComputerFolder, |index| if index == 0 {
                Ok(Some(false))
            } else {
                panic!("不读取低优先级配置")
            })
            .unwrap()
        );
    }

    /// 回收站仅在全部缺省时显示，任意有效层的显式隐藏及读取错误仍生效。
    #[test]
    fn recycle_bin_default_does_not_override_explicit_configuration() {
        for (folder, _) in SYSTEM_ICONS {
            assert_eq!(
                resolve_visible(&folder, |_| Ok(None)).unwrap(),
                folder == FOLDERID_RecycleBinFolder
            );
        }
        for user in [None, Some(false), Some(true)] {
            for policy in [None, Some(false), Some(true)] {
                for system in [None, Some(false), Some(true)] {
                    let layers = [user, policy, system]; // 回收站的全部显式配置与缺省组合。
                    assert_eq!(
                        resolve_visible(&FOLDERID_RecycleBinFolder, |index| Ok(layers[index]))
                            .unwrap(),
                        !user.or(policy).or(system).unwrap_or(false)
                    );
                }
            }
        }
        for failed_layer in 0..3 {
            assert!(
                resolve_visible(&FOLDERID_RecycleBinFolder, |index| {
                    if index == failed_layer {
                        Err("配置读取失败".into())
                    } else {
                        Ok(None)
                    }
                })
                .is_err()
            );
        }
    }
}
