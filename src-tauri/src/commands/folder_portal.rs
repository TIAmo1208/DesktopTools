use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::command;

/// 在注册表菜单所在 UI 线程选择目录，取消保持当前门户。
pub fn create_from_desktop_menu() -> Result<Option<String>, String>
{
    crate::log_info!("门户：打开系统目录选择器");
    let selected = crate::platform::windows::settings::choose_directory(0, "选择文件夹门户目录")?; // 无外部窗口作为 owner，避免禁用其他程序。
    if let Some(path) = &selected
    {
        crate::log_info!("门户：目录已选择，等待宿主读取，目录={path}");
    }
    else
    {
        crate::log_info!("门户：已取消目录选择");
    }
    Ok(selected)
}

/// 使用 Windows Shell 打开门户中的真实文件或目录。
#[tauri::command]
pub fn open_folder_portal_path(root_path: String, path: String) -> Result<(), String>
{
    let root = Path::new(&root_path).canonicalize().map_err(|error| format!("无法访问绑定目录：{error}"))?;
    let target = Path::new(&path).canonicalize().map_err(|error| format!("无法访问门户项目：{error}"))?;
    if !target.starts_with(&root)
    {
        return Err("门户项目超出绑定目录范围".to_string());
    }
    crate::platform::windows::settings::open_path(&target)
}

/// 门户删除复用应用的 Shell STA，绝不直接删除文件系统内容。
#[tauri::command]
pub fn delete_folder_portal_items(state: tauri::State<'_, crate::shell::DesktopItemState>, root_path: String, current_path: String, names: Vec<String>) -> Result<crate::contracts::ShellOperationResult, String> { execute_delete_folder_portal_items(&state, root_path, current_path, names) }

/// 将门户当前目录项目写入系统剪贴板。
#[tauri::command]
pub fn set_folder_portal_clipboard(state: tauri::State<'_, crate::shell::DesktopItemState>, root_path: String, current_path: String, names: Vec<String>, cut: bool) -> Result<crate::contracts::ShellOperationResult, String>
{
    let references = checked_item_paths(&root_path, &current_path, &names)?;
    state.set_portal_clipboard(references, cut).map_err(|error| error.message)
}

/// 将系统剪贴板粘贴到门户当前目录。
#[tauri::command]
pub fn paste_folder_portal_items(state: tauri::State<'_, crate::shell::DesktopItemState>, root_path: String, current_path: String) -> Result<crate::contracts::ShellOperationResult, String>
{
    let (_, current) = checked_directory(&root_path, &current_path)?;
    state.paste_portal(shell_display_path(&current)).map_err(|error| error.message)
}

/// 两种命令传输共用完整选择校验与原生操作结果。
pub(crate) fn execute_delete_folder_portal_items(state: &crate::shell::DesktopItemState, root_path: String, current_path: String, names: Vec<String>) -> Result<crate::contracts::ShellOperationResult, String>
{
    let references = checked_item_paths(&root_path, &current_path, &names)?; // 先校验全部目标，再开始任何操作。
    state.recycle_portal_items(references).map_err(|error| error.message)
}

/// 仅允许当前目录的直接子项；保留链接本身的路径，删除时不跟随链接目标。
fn checked_item_paths(root_path: &str, current_path: &str, names: &[String]) -> Result<Vec<String>, String>
{
    let (_, current) = checked_directory(root_path, current_path)?; // 当前目录已验证在根目录内。
    if names.is_empty()
    {
        return Err("门户选择为空".to_string());
    }
    let mut references = Vec::new(); // 全部校验成功后才交付 Shell。
    for name in names
    {
        if name.is_empty() || name == "." || name == ".." || name.contains(['\\', '/', ':', '\0']) || name.ends_with(['.', ' '])
        {
            return Err("门户项目名称无效".to_string());
        }
        let target = current.join(name); // 保留直接子项身份，不 canonicalize 到链接目标。
        std::fs::symlink_metadata(&target).map_err(|error| format!("无法访问门户项目：{error}"))?;
        references.push(shell_display_path(&target));
    }
    if references.iter().collect::<std::collections::HashSet<_>>().len() != references.len()
    {
        return Err("门户选择包含重复项目".to_string());
    }
    Ok(references)
}

/// 将 Rust 的 extended-length 路径转换为 Shell 解析接受的显示路径。
fn shell_display_path(path: &Path) -> String
{
    let value = path.to_string_lossy();
    value.strip_prefix(r"\\?\UNC\").map_or_else(|| value.strip_prefix(r"\\?\").unwrap_or(&value).to_string(), |unc| format!(r"\\{unc}"))
}

/// 门户重命名参数只允许当前目录中的单个项目。
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortalRenameRequest
{
    pub root_path: String,    // 绑定根目录。
    pub current_path: String, // 当前目录。
    pub name: String,         // 已选择的直接子项。
    pub new_name: String,     // 明确提交的新名称。
}

/// Tauri 原位重命名入口。
#[tauri::command]
pub fn rename_folder_portal_item(state: tauri::State<'_, crate::shell::DesktopItemState>, request: PortalRenameRequest) -> Result<crate::contracts::ShellOperationResult, String> { execute_rename_folder_portal_item(&state, request) }

/// 校验名称和冲突后交给现有 Shell STA；失败不覆盖现有文件。
pub(crate) fn execute_rename_folder_portal_item(state: &crate::shell::DesktopItemState, request: PortalRenameRequest) -> Result<crate::contracts::ShellOperationResult, String>
{
    let references = checked_item_paths(&request.root_path, &request.current_path, &[request.name])?; // 先校验源项目。
    let name = request.new_name.trim(); // 和公共命名输入保持一致。
    if name.is_empty() || name == "." || name == ".." || name.contains(['\\', '/', ':', '\0', '<', '>', '"', '|', '?', '*']) || name.ends_with(['.', ' '])
    {
        return Err("新名称无效".to_string());
    }
    let source = Path::new(&references[0]); // 不跟随目录链接进行改名。
    let target = source.with_file_name(name); // 目标限定在同一目录。
    if std::fs::symlink_metadata(&target).is_ok() && (!name.eq_ignore_ascii_case(&source.file_name().unwrap_or_default().to_string_lossy()) || !matches!((target.canonicalize(), source.canonicalize()), (Ok(target), Ok(source)) if target == source))
    {
        return Err("名称已存在，请更换名称后重试".to_string());
    }
    state.rename_portal_item(references[0].clone(), name.to_string()).map_err(|error| error.message)
}

/// 门户原生菜单请求保留目录边界与所属显示器坐标。
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortalMenuRequest
{
    pub root_path: String,    // 绑定根目录。
    pub current_path: String, // 当前目录。
    pub names: Vec<String>,   // 直接子项名称。
    pub monitor_id: String,   // 坐标所属宿主。
    pub client_x: f64,        // DIP 横坐标。
    pub client_y: f64,        // DIP 纵坐标。
}

/// Tauri 菜单入口与独立宿主共用执行路径。
#[tauri::command]
pub fn show_folder_portal_menu(state: tauri::State<'_, crate::shell::DesktopItemState>, runtime: tauri::State<'_, std::sync::Arc<crate::host::DesktopHostRuntime>>, request: PortalMenuRequest) -> Result<(), String> { execute_show_folder_portal_menu(&state, &runtime, request) }

/// 校验完整选择，转换屏幕坐标并在现有 STA 显示系统菜单。
pub(crate) fn execute_show_folder_portal_menu(state: &crate::shell::DesktopItemState, runtime: &crate::host::DesktopHostRuntime, request: PortalMenuRequest) -> Result<(), String>
{
    let references = checked_item_paths(&request.root_path, &request.current_path, &request.names)?; // 仅当前目录已存在的直接子项。
    crate::log_debug!("门户：准备原生菜单，目录={}，项目={}", request.current_path, references.join(" | ")); // 记录失败时的真实 Shell 路径。
    let hosts = runtime.snapshot().map_err(|error| error.message)?; // 菜单坐标取当前宿主事实。
    let monitor = &hosts.hosts.iter().find(|host| host.monitor.id == request.monitor_id).ok_or("门户显示器不存在")?.monitor;
    let x = f64::from(monitor.work_x) + request.client_x * monitor.scale_factor; // Windows 菜单使用物理屏幕坐标。
    let y = f64::from(monitor.work_y) + request.client_y * monitor.scale_factor;
    if !x.is_finite() || !y.is_finite() || x < i32::MIN as f64 || x > i32::MAX as f64 || y < i32::MIN as f64 || y > i32::MAX as f64
    {
        return Err("门户菜单坐标无效".to_string());
    }
    state.show_menu(crate::platform::windows::NativeMenuTarget::FolderItems(references), x.round() as i32, y.round() as i32, Vec::new()).map_err(|error| error.message)?;
    Ok(())
}

/// 文件夹门户中一个真实文件系统项目的只读快照。
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderPortalItem
{
    /// 当前目录中的文件名，不包含父目录。
    pub name: String,
    /// 项目是否为目录。
    pub is_directory: bool,
    /// 文件大小，目录或无法读取时为空。
    pub size: Option<u64>,
    /// 修改时间的 Unix 秒数，无法读取时为空。
    pub modified_unix: Option<u64>,
}

/// 文件夹门户当前目录的只读快照。
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderPortalSnapshot
{
    /// 门户绑定的导航根目录。
    pub root_path: String,
    /// 门户当前浏览目录。
    pub current_path: String,
    /// 当前目录是否可访问。
    pub accessible: bool,
    /// 当前目录项目，按文件夹优先、名称升序排列。
    pub items: Vec<FolderPortalItem>,
    /// 无法读取目录时的用户可见原因。
    pub error: Option<String>,
}

/// 验证门户路径为绝对目录，并阻止导航越出绑定根目录。
fn checked_directory(root_path: &str, current_path: &str) -> Result<(PathBuf, PathBuf), String>
{
    let root = Path::new(root_path).canonicalize().map_err(|error| format!("无法访问绑定目录：{error}"))?;
    if !root.is_dir()
    {
        return Err("绑定路径不是目录".to_string());
    }
    let current = Path::new(current_path).canonicalize().map_err(|error| format!("无法访问当前目录：{error}"))?;
    if !current.is_dir()
    {
        return Err("当前路径不是目录".to_string());
    }
    if !current.starts_with(&root)
    {
        return Err("当前目录超出绑定目录范围".to_string());
    }
    Ok((root, current))
}

/// 枚举当前门户目录，失败时保留根路径和当前路径供前端显示恢复入口。
fn read_snapshot(root_path: &str, current_path: &str) -> FolderPortalSnapshot
{
    let root_path = root_path.to_string();
    let current_path = current_path.to_string();
    let checked = checked_directory(&root_path, &current_path);
    let Ok((root, current)) = checked
    else
    {
        return FolderPortalSnapshot { root_path, current_path, accessible: false, items: Vec::new(), error: Some("无法访问门户目录".to_string()) };
    };
    let entries = match std::fs::read_dir(&current)
    {
        Ok(entries) => entries,
        Err(error) => return FolderPortalSnapshot { root_path, current_path, accessible: false, items: Vec::new(), error: Some(format!("读取目录失败：{error}")) },
    };
    let mut items = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let modified_unix = metadata.modified().ok().and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok()).map(|duration| duration.as_secs());
            Some(FolderPortalItem { name, is_directory: metadata.is_dir(), size: metadata.is_file().then_some(metadata.len()), modified_unix })
        })
        .collect::<Vec<_>>();
    items.sort_by_cached_key(|item| (!item.is_directory, item.name.to_lowercase(), item.name.clone()));
    FolderPortalSnapshot { root_path: root.to_string_lossy().into_owned(), current_path: current.to_string_lossy().into_owned(), accessible: true, items, error: None }
}

/// 获取绑定目录和当前目录的文件夹门户快照。
#[command]
pub fn get_folder_portal_snapshot(root_path: String, current_path: Option<String>) -> Result<FolderPortalSnapshot, String>
{
    if root_path.trim().is_empty()
    {
        return Err("绑定目录不能为空".to_string());
    }
    let current = current_path.filter(|path| !path.trim().is_empty()).unwrap_or_else(|| root_path.clone());
    Ok(read_snapshot(&root_path, &current))
}

#[cfg(test)]
mod tests
{
    use super::{checked_item_paths, read_snapshot};

    /// 验证目录快照按文件夹优先、名称升序排列，并保留 Unicode 名称。
    #[test]
    fn snapshot_sorts_directories_before_files()
    {
        let root = std::env::temp_dir().join(format!("desktop-tools-folder-portal-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir(&root).expect("临时目录创建成功");
        std::fs::create_dir(root.join("资料")).expect("子目录创建成功");
        std::fs::write(root.join("说明.txt"), "内容").expect("文件创建成功");
        let path = root.to_string_lossy();
        let snapshot = read_snapshot(&path, &path);
        assert!(snapshot.accessible);
        assert_eq!(snapshot.items.iter().map(|item| item.name.as_str()).collect::<Vec<_>>(), vec!["资料", "说明.txt"]);
        assert_eq!(checked_item_paths(&path, &path, &["说明.txt".to_string()]).unwrap().len(), 1);
        for invalid in [".", "..", "../资料", "C:\\资料", "说明.txt:stream"]
        {
            assert!(checked_item_paths(&path, &path, &["说明.txt".to_string(), invalid.to_string()]).is_err());
        }
        assert!(root.join("说明.txt").exists()); // 非法选择不产生部分删除。
        let _ = std::fs::remove_dir_all(root);
    }
}
