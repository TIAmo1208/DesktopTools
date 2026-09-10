use std::ffi::c_void;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

use windows::Win32::System::Com::{CLSCTX_LOCAL_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize, IServiceProvider};
use windows::Win32::UI::Shell::Common::IObjectArray;
use windows::core::{GUID, HRESULT, IUnknown, Interface};

/// Microsoft 官方 TypeAgent 当前使用的 Immersive Shell COM 类。
const IMMERSIVE_SHELL: GUID = GUID::from_u128(0xc2f03a33_21f5_47fa_b4bb_156362a2f239);

/// Microsoft TypeAgent 使用的 VirtualDesktopManagerInternal 服务 CLSID。
const VIRTUAL_DESKTOP_MANAGER_INTERNAL_SERVICE: GUID = GUID::from_u128(0xc5e0cdca_7b6e_41b2_9fc4_d93975cc467b);

/// Windows 11 24H2 开始使用当前 TypeAgent 接口 IID。
const WINDOWS_11_24H2_BUILD: u32 = 26100;

windows_core::imp::define_interface!(IVirtualDesktopWindows10, IVirtualDesktop_Vtbl, 0xff72ffdd_be7e_43fc_9c03_ad81681e88e4);
windows_core::imp::interface_hierarchy!(IVirtualDesktopWindows10, IUnknown);

windows_core::imp::define_interface!(IVirtualDesktopWindows11, IVirtualDesktop_Vtbl, 0x3f07f4be_b107_441a_af0f_39d82529072c);
windows_core::imp::interface_hierarchy!(IVirtualDesktopWindows11, IUnknown);

/// Windows 私有虚拟桌面对象只暴露当前实现需要的稳定身份读取。
impl IVirtualDesktopWindows10
{
    /// 返回当前虚拟桌面的稳定 GUID。
    unsafe fn get_id(&self) -> windows::core::Result<GUID>
    {
        unsafe {
            let mut result = GUID::zeroed(); // COM 写入的虚拟桌面稳定身份。
            (Interface::vtable(self).GetId)(Interface::as_raw(self), &mut result).map(|| result)
        }
    }
}

/// Windows 11 私有虚拟桌面对象只暴露当前实现需要的稳定身份读取。
impl IVirtualDesktopWindows11
{
    /// 返回当前虚拟桌面的稳定 GUID。
    unsafe fn get_id(&self) -> windows::core::Result<GUID>
    {
        unsafe {
            let mut result = GUID::zeroed(); // COM 写入的虚拟桌面稳定身份。
            (Interface::vtable(self).GetId)(Interface::as_raw(self), &mut result).map(|| result)
        }
    }
}

/// Windows 私有虚拟桌面对象的已确认 vtable 前缀。
#[repr(C)]
#[allow(non_snake_case)]
pub struct IVirtualDesktop_Vtbl
{
    /// IUnknown 基础接口。
    base__: windows_core::IUnknown_Vtbl,
    /// 当前桌面是否包含指定应用视图。
    IsViewVisible: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut i32) -> HRESULT,
    /// 返回稳定虚拟桌面 GUID。
    GetId: unsafe extern "system" fn(*mut c_void, *mut GUID) -> HRESULT,
}

/// 两个受支持 Windows 版本共享到 SwitchDesktop 的私有 vtable 前缀。
#[repr(C)]
#[allow(non_snake_case)]
pub struct IVirtualDesktopManagerCommon_Vtbl
{
    /// IUnknown 基础接口。
    base__: windows_core::IUnknown_Vtbl,
    /// 返回虚拟桌面数量。
    GetCount: unsafe extern "system" fn(*mut c_void, *mut u32) -> HRESULT,
    /// 将应用视图移动到桌面。
    MoveViewToDesktop: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> HRESULT,
    /// 判断应用视图是否可以跨桌面移动。
    CanViewMoveDesktops: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut i32) -> HRESULT,
    /// 返回当前虚拟桌面。
    GetCurrentDesktop: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    /// 返回完整虚拟桌面对象数组。
    GetDesktops: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    /// 返回相邻虚拟桌面。
    GetAdjacentDesktop: unsafe extern "system" fn(*mut c_void, *mut c_void, i32, *mut *mut c_void) -> HRESULT,
    /// 切换当前虚拟桌面。
    SwitchDesktop: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
}

windows_core::imp::define_interface!(IVirtualDesktopManagerWindows10, IVirtualDesktopManagerWindows10_Vtbl, 0xf31574d6_b682_4cdc_bd56_1827860abec6);
windows_core::imp::interface_hierarchy!(IVirtualDesktopManagerWindows10, IUnknown);

/// Windows 10 私有虚拟桌面创建与删除调用。
impl IVirtualDesktopManagerWindows10
{
    /// 返回当前虚拟桌面。
    unsafe fn current_desktop(&self) -> windows::core::Result<IVirtualDesktopWindows10>
    {
        unsafe {
            let mut result = std::ptr::null_mut(); // COM 返回的当前虚拟桌面接口。
            (Interface::vtable(self).base__.GetCurrentDesktop)(Interface::as_raw(self), &mut result).and_then(|| windows_core::Type::from_abi(result))
        }
    }

    /// 返回当前完整虚拟桌面对象集合。
    unsafe fn desktops(&self) -> windows::core::Result<IObjectArray>
    {
        unsafe {
            let mut result = std::ptr::null_mut(); // COM 返回的虚拟桌面对象数组。
            (Interface::vtable(self).base__.GetDesktops)(Interface::as_raw(self), &mut result).and_then(|| windows_core::Type::from_abi(result))
        }
    }

    /// 创建新虚拟桌面。
    unsafe fn create_desktop(&self) -> windows::core::Result<IVirtualDesktopWindows10>
    {
        unsafe {
            let mut result = std::ptr::null_mut(); // COM 返回的新虚拟桌面接口。
            (Interface::vtable(self).CreateDesktopW)(Interface::as_raw(self), &mut result).and_then(|| windows_core::Type::from_abi(result))
        }
    }

    /// 删除目标虚拟桌面并指定回退桌面。
    unsafe fn remove_desktop(&self, desktop: &IVirtualDesktopWindows10, fallback: &IVirtualDesktopWindows10) -> windows::core::Result<()> { unsafe { (Interface::vtable(self).RemoveDesktop)(Interface::as_raw(self), Interface::as_raw(desktop), Interface::as_raw(fallback)).ok() } }
}

/// Windows 10 22H2 私有接口的创建与删除 vtable 后缀。
#[repr(C)]
#[allow(non_snake_case)]
pub struct IVirtualDesktopManagerWindows10_Vtbl
{
    /// Windows 10 与 Windows 11 共享的查询前缀。
    base__: IVirtualDesktopManagerCommon_Vtbl,
    /// 创建新虚拟桌面。
    CreateDesktopW: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    /// 删除目标虚拟桌面。
    RemoveDesktop: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> HRESULT,
    /// 按稳定身份查找虚拟桌面。
    FindDesktop: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
}

windows_core::imp::define_interface!(IVirtualDesktopManagerWindows11, IVirtualDesktopManagerWindows11_Vtbl, 0x53f5ca0b_158f_4124_900c_057158060b27);
windows_core::imp::interface_hierarchy!(IVirtualDesktopManagerWindows11, IUnknown);

/// Windows 11 24H2 及以后私有虚拟桌面创建与删除调用。
impl IVirtualDesktopManagerWindows11
{
    /// 返回当前虚拟桌面。
    unsafe fn current_desktop(&self) -> windows::core::Result<IVirtualDesktopWindows11>
    {
        unsafe {
            let mut result = std::ptr::null_mut(); // COM 返回的当前虚拟桌面接口。
            (Interface::vtable(self).base__.GetCurrentDesktop)(Interface::as_raw(self), &mut result).and_then(|| windows_core::Type::from_abi(result))
        }
    }

    /// 返回当前完整虚拟桌面对象集合。
    unsafe fn desktops(&self) -> windows::core::Result<IObjectArray>
    {
        unsafe {
            let mut result = std::ptr::null_mut(); // COM 返回的虚拟桌面对象数组。
            (Interface::vtable(self).base__.GetDesktops)(Interface::as_raw(self), &mut result).and_then(|| windows_core::Type::from_abi(result))
        }
    }

    /// 创建新虚拟桌面。
    unsafe fn create_desktop(&self) -> windows::core::Result<IVirtualDesktopWindows11>
    {
        unsafe {
            let mut result = std::ptr::null_mut(); // COM 返回的新虚拟桌面接口。
            (Interface::vtable(self).CreateDesktop)(Interface::as_raw(self), &mut result).and_then(|| windows_core::Type::from_abi(result))
        }
    }

    /// 删除目标虚拟桌面并指定回退桌面。
    unsafe fn remove_desktop(&self, desktop: &IVirtualDesktopWindows11, fallback: &IVirtualDesktopWindows11) -> windows::core::Result<()> { unsafe { (Interface::vtable(self).RemoveDesktop)(Interface::as_raw(self), Interface::as_raw(desktop), Interface::as_raw(fallback)).ok() } }
}

/// Windows 11 24H2 起的私有虚拟桌面管理器后缀。
#[repr(C)]
#[allow(non_snake_case)]
pub struct IVirtualDesktopManagerWindows11_Vtbl
{
    /// Windows 10 与 Windows 11 共享的查询前缀。
    base__: IVirtualDesktopManagerCommon_Vtbl,
    /// 切换桌面并迁移前台视图。
    SwitchDesktopAndMoveForegroundView: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    /// 创建新虚拟桌面。
    CreateDesktop: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    /// 移动虚拟桌面顺序。
    MoveDesktop: unsafe extern "system" fn(*mut c_void, *mut c_void, i32) -> HRESULT,
    /// 删除目标虚拟桌面。
    RemoveDesktop: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> HRESULT,
}

/// Windows 虚拟桌面当前不可变状态。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VirtualDesktopEnvironment
{
    /// 当前 Windows build number。
    pub windows_build: u32,
    /// 当前桌面稳定身份。
    pub current_id: String,
    /// 按 Windows 顺序排列的完整桌面身份。
    pub desktop_ids: Vec<String>,
}

/// 虚拟桌面生命周期变化事实。
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VirtualDesktopChange
{
    /// 新桌面已经出现。
    Created
    {
        desktop_id: String
    },
    /// 现有桌面已经删除。
    Removed
    {
        desktop_id: String
    },
    /// Windows 当前桌面已经切换。
    Switched
    {
        previous_id: String, current_id: String
    },
}

/// 隔离 Windows 10/11 私有虚拟桌面 ABI 与轮询通知。
#[derive(Clone, Copy)]
pub struct WindowsVirtualDesktopAdapter
{
    /// 当前系统 build number。
    windows_build: u32,
}

/// 仅包含 RtlGetVersion 需要的 Windows 版本结构。
#[repr(C)]
struct RtlOsVersionInfo
{
    /// 结构体字节数。
    size: u32,
    /// Windows 主版本号。
    major: u32,
    /// Windows 次版本号。
    minor: u32,
    /// Windows build number。
    build: u32,
    /// Windows 平台标识。
    platform: u32,
    /// Windows CSD 文本。
    csd: [u16; 128],
}

#[link(name = "ntdll")]
unsafe extern "system" {
    /// 返回不受应用清单版本虚拟化影响的真实 Windows 版本。
    fn RtlGetVersion(version: *mut RtlOsVersionInfo) -> i32;
}

/// 配对释放由当前调用初始化的 COM apartment。
struct ComApartment
{
    /// 当前调用是否需要执行 CoUninitialize。
    initialized: bool,
}

/// 释放当前线程由适配器初始化的 COM apartment。
impl Drop for ComApartment
{
    /// 释放适配器持有的原生资源。
    fn drop(&mut self)
    {
        if self.initialized
        {
            unsafe { CoUninitialize() };
        }
    }
}

/// 提供 Windows 10/11 虚拟桌面枚举、创建、删除和变化通知。
impl WindowsVirtualDesktopAdapter
{
    /// 按真实 Windows build 选择经过隔离的私有 ABI。
    pub fn new() -> Result<Self, String>
    {
        let windows_build = windows_build()?; // 当前运行系统的真实 build number。
        crate::log_info!("虚拟桌面：版本适配器已选择，Windows build={}，ABI={}", windows_build, if uses_windows_11_24h2_abi(windows_build) { "windows-11-24h2+" } else { "windows-10" });
        Ok(Self { windows_build })
    }

    /// 返回当前完整桌面集合与活动身份。
    pub fn environment(&self) -> Result<VirtualDesktopEnvironment, String>
    {
        if uses_windows_11_24h2_abi(self.windows_build)
        {
            return self.with_shell(|shell| unsafe {
                let manager: IVirtualDesktopManagerWindows11 = shell.QueryService(&VIRTUAL_DESKTOP_MANAGER_INTERNAL_SERVICE)?; // Windows 11 24H2+ 私有管理器。
                let current_id = desktop_id_windows11(&manager.current_desktop()?)?; // 当前虚拟桌面稳定身份。
                let desktops = manager.desktops()?; // 当前完整虚拟桌面对象数组。
                let count = desktops.GetCount()?; // 当前 Windows 桌面数量。
                let mut desktop_ids = Vec::with_capacity(count as usize); // 按 Windows 顺序收集的稳定身份。
                for index in 0..count
                {
                    desktop_ids.push(desktop_id_windows11(&desktops.GetAt::<IVirtualDesktopWindows11>(index)?)?);
                }
                Ok(VirtualDesktopEnvironment { windows_build: self.windows_build, current_id, desktop_ids })
            });
        }

        self.with_shell(|shell| unsafe {
            let manager: IVirtualDesktopManagerWindows10 = shell.QueryService(&VIRTUAL_DESKTOP_MANAGER_INTERNAL_SERVICE)?; // Windows 10 22H2 私有管理器。
            let current_id = desktop_id_windows10(&manager.current_desktop()?)?; // 当前虚拟桌面稳定身份。
            let desktops = manager.desktops()?; // 当前完整虚拟桌面对象数组。
            let count = desktops.GetCount()?; // 当前 Windows 桌面数量。
            let mut desktop_ids = Vec::with_capacity(count as usize); // 按 Windows 顺序收集的稳定身份。
            for index in 0..count
            {
                desktop_ids.push(desktop_id_windows10(&desktops.GetAt::<IVirtualDesktopWindows10>(index)?)?);
            }
            Ok(VirtualDesktopEnvironment { windows_build: self.windows_build, current_id, desktop_ids })
        })
    }

    /// 创建新虚拟桌面并返回 Windows 分配的稳定身份。
    pub fn create(&self) -> Result<String, String>
    {
        if uses_windows_11_24h2_abi(self.windows_build)
        {
            return self.with_shell(|shell| unsafe {
                let manager: IVirtualDesktopManagerWindows11 = shell.QueryService(&VIRTUAL_DESKTOP_MANAGER_INTERNAL_SERVICE)?; // Windows 11 24H2+ 私有管理器。
                desktop_id_windows11(&manager.create_desktop()?)
            });
        }

        self.with_shell(|shell| unsafe {
            let manager: IVirtualDesktopManagerWindows10 = shell.QueryService(&VIRTUAL_DESKTOP_MANAGER_INTERNAL_SERVICE)?; // Windows 10 22H2 私有管理器。
            desktop_id_windows10(&manager.create_desktop()?)
        })
    }

    /// 删除目标虚拟桌面并把窗口迁移到指定回退桌面。
    pub fn remove(&self, target_id: &str, fallback_id: &str) -> Result<(), String>
    {
        if uses_windows_11_24h2_abi(self.windows_build)
        {
            return self.with_shell(|shell| unsafe {
                let manager: IVirtualDesktopManagerWindows11 = shell.QueryService(&VIRTUAL_DESKTOP_MANAGER_INTERNAL_SERVICE)?; // Windows 11 24H2+ 私有管理器。
                let desktops = manager.desktops()?; // 当前完整桌面对象数组。
                let count = desktops.GetCount()?; // 当前虚拟桌面数量。
                let mut target = None; // 等待删除的桌面对象。
                let mut fallback = None; // 删除后承接窗口的回退桌面对象。
                for index in 0..count
                {
                    let desktop = desktops.GetAt::<IVirtualDesktopWindows11>(index)?; // 当前枚举桌面对象。
                    let id = desktop_id_windows11(&desktop)?; // 当前桌面稳定身份。
                    if id == target_id
                    {
                        target = Some(desktop.clone());
                    }
                    if id == fallback_id
                    {
                        fallback = Some(desktop);
                    }
                }
                let target = target.ok_or_else(invalid_argument)?; // 已校验的删除目标。
                let fallback = fallback.ok_or_else(invalid_argument)?; // 已校验的回退目标。
                manager.remove_desktop(&target, &fallback)
            });
        }

        self.with_shell(|shell| unsafe {
            let manager: IVirtualDesktopManagerWindows10 = shell.QueryService(&VIRTUAL_DESKTOP_MANAGER_INTERNAL_SERVICE)?; // Windows 10 22H2 私有管理器。
            let desktops = manager.desktops()?; // 当前完整桌面对象数组。
            let count = desktops.GetCount()?; // 当前虚拟桌面数量。
            let mut target = None; // 等待删除的桌面对象。
            let mut fallback = None; // 删除后承接窗口的回退桌面对象。
            for index in 0..count
            {
                let desktop = desktops.GetAt::<IVirtualDesktopWindows10>(index)?; // 当前枚举桌面对象。
                let id = desktop_id_windows10(&desktop)?; // 当前桌面稳定身份。
                if id == target_id
                {
                    target = Some(desktop.clone());
                }
                if id == fallback_id
                {
                    fallback = Some(desktop);
                }
            }
            let target = target.ok_or_else(invalid_argument)?; // 已校验的删除目标。
            let fallback = fallback.ok_or_else(invalid_argument)?; // 已校验的回退目标。
            manager.remove_desktop(&target, &fallback)
        })
    }

    /// 启动创建、删除和切换变化轮询并返回唯一接收端。
    pub fn watch(self) -> Receiver<Result<Vec<VirtualDesktopChange>, String>>
    {
        let (sender, receiver) = mpsc::channel(); // 适配器轮询线程和应用协调器之间的变化通道。
        thread::spawn(move || {
            let mut previous = match self.environment()
            {
                Ok(environment) => environment,
                Err(error) =>
                {
                    let _ = sender.send(Err(error));
                    return;
                }
            }; // 当前变化比较基线。
            loop
            {
                thread::sleep(Duration::from_millis(250));
                let current = match self.environment()
                {
                    Ok(environment) => environment,
                    Err(error) =>
                    {
                        let _ = sender.send(Err(error));
                        return;
                    }
                }; // 当前轮询读取的虚拟桌面环境。
                let changes = diff_environments(&previous, &current); // 当前轮询产生的稳定变化事实。
                previous = current;
                if !changes.is_empty() && sender.send(Ok(changes)).is_err()
                {
                    return;
                }
            }
        });
        receiver
    }

    /// 在当前线程初始化 COM 并创建 Immersive Shell 服务提供器。
    fn with_shell<T>(&self, action: impl FnOnce(&IServiceProvider) -> windows::core::Result<T>) -> Result<T, String>
    {
        let initialized = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok(); // 当前调用是否拥有 COM 初始化配对责任。
        let _apartment = ComApartment { initialized }; // 当前作用域的 COM 生命周期。
        let shell: IServiceProvider = unsafe { CoCreateInstance(&IMMERSIVE_SHELL, None, CLSCTX_LOCAL_SERVER).map_err(|error| format!("创建 ImmersiveShell 失败: {error}"))? }; // Explorer Immersive Shell 服务提供器。
        action(&shell).map_err(|error| format!("调用虚拟桌面适配器失败: {error}"))
    }
}

/// 返回当前系统不受清单影响的真实 build number。
fn windows_build() -> Result<u32, String>
{
    let mut version = RtlOsVersionInfo { size: std::mem::size_of::<RtlOsVersionInfo>() as u32, major: 0, minor: 0, build: 0, platform: 0, csd: [0; 128] }; // 等待 RtlGetVersion 写入的版本结构。
    let status = unsafe { RtlGetVersion(&mut version) }; // NTSTATUS 版本查询结果。
    if status < 0 || version.build == 0
    {
        return Err(format!("读取 Windows build 失败: NTSTATUS={status:#x}"));
    }
    Ok(version.build)
}

/// 判断当前 build 是否使用 Windows 11 24H2+ 私有 ABI。
fn uses_windows_11_24h2_abi(windows_build: u32) -> bool { windows_build >= WINDOWS_11_24H2_BUILD }

/// 返回参数无效 COM 错误。
fn invalid_argument() -> windows::core::Error { windows::core::Error::from_hresult(HRESULT(0x80070057u32 as i32)) }

/// 将 Windows 10 桌面 GUID 转换为持久化稳定身份。
fn desktop_id_windows10(desktop: &IVirtualDesktopWindows10) -> windows::core::Result<String>
{
    let id = unsafe { desktop.get_id()? }; // Windows 返回的虚拟桌面 GUID。
    Ok(format!("{:032x}", id.to_u128()))
}

/// 将 Windows 11 桌面 GUID 转换为持久化稳定身份。
fn desktop_id_windows11(desktop: &IVirtualDesktopWindows11) -> windows::core::Result<String>
{
    let id = unsafe { desktop.get_id()? }; // Windows 返回的虚拟桌面 GUID。
    Ok(format!("{:032x}", id.to_u128()))
}

/// 比较两次虚拟桌面环境并生成创建、删除和切换事实。
fn diff_environments(previous: &VirtualDesktopEnvironment, current: &VirtualDesktopEnvironment) -> Vec<VirtualDesktopChange>
{
    let mut changes = current.desktop_ids.iter().filter(|desktop_id| !previous.desktop_ids.contains(desktop_id)).map(|desktop_id| VirtualDesktopChange::Created { desktop_id: desktop_id.clone() }).collect::<Vec<_>>(); // 当前新出现的桌面事实。
    changes.extend(previous.desktop_ids.iter().filter(|desktop_id| !current.desktop_ids.contains(desktop_id)).map(|desktop_id| VirtualDesktopChange::Removed { desktop_id: desktop_id.clone() }));
    if previous.current_id != current.current_id
    {
        changes.push(VirtualDesktopChange::Switched { previous_id: previous.current_id.clone(), current_id: current.current_id.clone() });
    }
    changes
}

#[cfg(test)]
mod tests
{
    use super::{VirtualDesktopChange, VirtualDesktopEnvironment, WINDOWS_11_24H2_BUILD, diff_environments, uses_windows_11_24h2_abi};

    /// 验证目标系统选择各自经过隔离的私有 ABI。
    #[test]
    fn windows_build_selects_the_matching_private_abi()
    {
        assert_eq!(WINDOWS_11_24H2_BUILD, 26100);
        assert!(!uses_windows_11_24h2_abi(19045));
        assert!(uses_windows_11_24h2_abi(26100));
        assert!(uses_windows_11_24h2_abi(26200));
    }

    /// 验证轮询环境差异按创建、删除、切换顺序发布事实。
    #[test]
    fn environment_diff_emits_virtual_desktop_lifecycle()
    {
        let previous = VirtualDesktopEnvironment { windows_build: 19045, current_id: "a".to_string(), desktop_ids: vec!["a".to_string(), "b".to_string()] }; // 变化前虚拟桌面环境。
        let current = VirtualDesktopEnvironment { windows_build: 19045, current_id: "c".to_string(), desktop_ids: vec!["a".to_string(), "c".to_string()] }; // 创建 c、删除 b 并切换到 c 后的环境。

        assert_eq!(diff_environments(&previous, &current), [VirtualDesktopChange::Created { desktop_id: "c".to_string() }, VirtualDesktopChange::Removed { desktop_id: "b".to_string() }, VirtualDesktopChange::Switched { previous_id: "a".to_string(), current_id: "c".to_string() }]);
    }
}
