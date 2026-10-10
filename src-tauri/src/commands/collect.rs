use crate::app::settings::SettingsState;
use crate::collect::{self, CollectItem};
use std::path::Path;
use tauri::{AppHandle, Emitter, Manager, State};

/// 只在当前会话维护文件与窗口的关联；不同文件分配独立窗口。
#[derive(Default)]
struct EditorWindows
{
    labels: std::collections::BTreeMap<std::path::PathBuf, String>,                            // 规范文件路径到窗口标识。
    next_id: u64,                                                                              // 单调分配，重命名后旧路径再次创建也不会复用活跃窗口标识。
    baselines: std::collections::BTreeMap<String, Option<std::collections::BTreeSet<String>>>, // 窗口打开时的附件集合；读取失败禁止清理。
}

impl EditorWindows
{
    /// 文件改名后保留原窗口，旧文件名可独立创建新条目。
    fn rename(&mut self, old: &Path, new: std::path::PathBuf) -> Option<String>
    {
        let label = self.labels.remove(old)?; // 已打开文件的窗口标识。
        self.labels.insert(new, label.clone());
        Some(label)
    }

    /// 同一文件复用标识，不同文件分配新的独立窗口标识。
    fn label(&mut self, path: std::path::PathBuf) -> String
    {
        self.labels
            .entry(path)
            .or_insert_with(|| {
                self.next_id += 1;
                format!("collect-editor-{}", self.next_id)
            })
            .clone()
    }
}

static EDITORS: std::sync::LazyLock<std::sync::Mutex<EditorWindows>> = std::sync::LazyLock::new(Default::default); // 打开和应用内重命名串行维护关联。

/// 使用规范绝对路径统一 Windows 文件名大小写和相对路径表示。
fn editor_path(root: &Path, name: &str) -> Result<std::path::PathBuf, String>
{
    let path = collect::safe_file(root, name).and_then(|path| path.canonicalize()).map_err(|error| format!("收集项不可用：{error}"))?; // 必须是已存在的收集项。
    if !path.is_file()
    {
        return Err("收集项不是文件".into());
    }
    Ok(std::path::PathBuf::from(path.to_string_lossy().to_lowercase()))
}

fn config(state: &State<'_, SettingsState>) -> Result<std::path::PathBuf, String> { crate::app::settings::resolve_config_directory(&state.preferences()?.config_directory) }
fn changed(app: &AppHandle) { let _ = app.emit("collect-items-changed", ()); }

/// 读取当前收集项引用的本地图片，显示数据不写回 Markdown。
#[tauri::command]
pub async fn read_collect_image(app: AppHandle, name: String, reference: String, pixels: Option<u32>) -> Result<String, String> { tauri::async_runtime::spawn_blocking(move || execute_read_collect_image(&app, &name, &reference, pixels)).await.map_err(|error| error.to_string())? }

/// 两个命令通道共用身份校验；列表缩略图复用已有 STA 和有界缓存。
pub(crate) fn execute_read_collect_image(app: &AppHandle, name: &str, reference: &str, pixels: Option<u32>) -> Result<String, String>
{
    let config = config(&app.state::<SettingsState>())?; // 使用当前配置，不接受前端指定任意缩略图路径。
    if let Some(pixels) = pixels
    {
        if !(1..=768).contains(&pixels)
        {
            return Err("图片预览尺寸必须在 1 到 768 像素之间".into());
        }
        let item = collect::get_item(&config, name).map_err(|error| error.to_string())?; // 独立图片身份仍由收集业务验证。
        if item.kind != collect::CollectKind::Image
        {
            return Err("缩略图只用于独立图片收集项".into());
        }
        let path = item.path.ok_or("图片路径不可用")?; // 不使用调用方的 reference 绕过归属验证。
        let response = app.state::<crate::shell::DesktopItemState>().images.request(path, pixels)?; // 合并相同尺寸读取，不增加常驻线程。
        return response.recv().map_err(|error| error.to_string())?.map(|image| image.data_url.clone()).ok_or_else(|| "系统无法生成此图片的缩略图".into());
    }
    collect::read_image(&config, name, reference).map_err(|error| format!("读取收集项图片失败：{error}"))
}

/// 只把明确的网页和邮件链接交给系统默认程序，拒绝脚本和应用内部协议。
#[tauri::command]
pub fn open_collect_link(url: String) -> Result<(), String>
{
    let parsed = tauri::Url::parse(&url).map_err(|_| "链接地址无效")?; // 使用 URL 解析器校验协议。
    if !matches!(parsed.scheme(), "http" | "https" | "mailto") || url.chars().any(char::is_control)
    {
        return Err("不支持此链接协议".into());
    }
    crate::platform::windows::settings::open_path(Path::new(parsed.as_str()))
}
#[tauri::command]
pub async fn open_collect_item(app: AppHandle, name: String) -> Result<(), String> { tauri::async_runtime::spawn_blocking(move || execute_open_collect_item(&app, name)).await.map_err(|error| format!("打开收集项任务失败：{error}"))? }

/// 两个宿主共用打开入口，失败后重新发布文件事实，保留失效映射。
pub(crate) fn execute_open_collect_item(app: &AppHandle, name: String) -> Result<(), String>
{
    let result = (|| {
        let config = config(&app.state::<SettingsState>())?; // 当前配置目录。
        let item = collect::get_item(&config, &name).map_err(|error| format!("读取收集项失败，条目={name}，原因={error}"))?; // 每次打开重新检查文件事实。
        if item.kind == collect::CollectKind::Text
        {
            return execute_open_collect_editor(app.clone(), name.clone());
        }
        let path = item.path.ok_or_else(|| format!("收集项路径未配置，条目={name}"))?; // 映射目标不能由前端替换。
        crate::platform::windows::settings::open_path(Path::new(&path))
    })(); // 包括读取失败和检查后删除的竞态。
    if let Err(error) = &result
    {
        changed(app);
        crate::log_error!("收集箱：打开失败，条目={name}，原因={error}");
    }
    result
}

#[tauri::command]
pub async fn open_collect_editor(app: AppHandle, name: String) -> Result<(), String> { tauri::async_runtime::spawn_blocking(move || execute_open_collect_editor(app, name)).await.map_err(|error| format!("编辑窗任务失败：{error}"))? }

/// 检查和追加共用窗口注册锁，避免检查后打开编辑窗覆盖草稿。
pub(crate) fn with_collect_editor_state<T>(app: &AppHandle, root: &Path, name: Option<&str>, receive: impl FnOnce(bool) -> Result<T, String>) -> Result<T, String>
{
    let mut editors = EDITORS.lock().map_err(|_| "编辑窗状态不可用")?; // 与窗口创建、重命名串行。
    editors.labels.retain(|_, label| app.get_webview_window(label).is_some());
    let editing = name.filter(|name| name.ends_with(".md")).map(|name| editor_path(root, name).map(|path| editors.labels.contains_key(&path))).transpose()?.unwrap_or(false); // 非文本目标不查询文本窗口。
    let result = receive(editing); // 写入完成前保持注册锁。
    drop(editors);
    result
}

pub(crate) fn execute_open_collect_editor(app: AppHandle, name: String) -> Result<(), String>
{
    let settings = app.state::<SettingsState>(); // 当前配置目录。
    let root = collect::directory(&config(&settings)?); // 工作区共享收集箱。
    let mut editors = EDITORS.lock().map_err(|_| "编辑窗状态不可用")?; // 串行检查和创建，避免重复打开。
    let path = editor_path(&root, &name)?; // 当前文件身份。
    editors.labels.retain(|_, label| app.get_webview_window(label).is_some());
    let label = editors.label(path.clone()); // 文件独立的窗口标识。
    editors.baselines.retain(|label, _| app.get_webview_window(label).is_some());
    if let Some(window) = app.get_webview_window(&label)
    {
        window.unminimize().and_then(|_| window.show()).and_then(|_| window.set_focus()).map_err(|e| e.to_string())?;
        return Ok(());
    }
    let result = tauri::WebviewWindowBuilder::new(&app, &label, tauri::WebviewUrl::App(format!("index.html?collect-editor=1&name={}", percent_encode_query(&name)).into())).title(format!("编辑收集项 — {name}")).inner_size(720.0, 520.0).min_inner_size(420.0, 320.0).build(); // 按需创建，不限制其他文件的窗口。
    if result.is_ok()
    {
        let baseline = collect::cleanup::baseline(&config(&settings)?, &name).map_err(|error| crate::log_warn!("收集箱：无法记录编辑附件基线，路径={}，原因={error}", path.display())).ok();
        editors.baselines.insert(label, baseline);
    }
    if result.is_err()
    {
        editors.labels.remove(&path);
    }
    result.map(|_| ()).map_err(|e| format!("打开收集项编辑窗失败：{e}"))
}

/// 关闭编辑窗时保守清理受管附件，后台失败不撤销成功保存的正文。
#[tauri::command]
pub async fn cleanup_collect_resources(app: AppHandle, window: tauri::WebviewWindow, name: String, discard: bool) -> Result<(), String>
{
    let label = window.label().to_string(); // 调用窗口身份由 Tauri 注入，不接受前端指定。
    tauri::async_runtime::spawn_blocking(move || {
        let mut document = name.clone(); // 配置解析成功后记录完整路径。
        let result = (|| {
            let config = config(&app.state::<SettingsState>())?; // 关闭时使用当前配置。
            document = collect::directory(&config).join(&name).to_string_lossy().into_owned();
            let path = editor_path(&collect::directory(&config), &name)?;
            let mut editors = EDITORS.lock().map_err(|_| "编辑窗状态不可用")?; // 串行保护改名、打开和清理。
            editors.labels.retain(|_, label| app.get_webview_window(label).is_some());
            if editors.labels.get(&path) != Some(&label)
            {
                return Err("清理请求不属于当前编辑窗".to_string());
            }
            if editors.labels.len() != 1
            {
                return Err("存在其他编辑窗，无法确认未保存的跨文档引用，保留附件".to_string());
            }
            let baseline = editors.baselines.get(&label).and_then(Option::as_ref).ok_or("编辑附件基线不完整，保留附件")?;
            collect::cleanup::cleanup(&config, &name, discard.then_some(baseline), |proof| {
                let result = app.state::<crate::shell::DesktopItemState>().recycle_collect_resource(proof.clone()).map_err(|error| error.message)?; // 复用现有 Shell STA 回收及 Pre 回调身份校验。
                if result.aborted || result.operation_error.is_some() || result.permanent || result.items.len() != 1 || !result.items[0].succeeded
                {
                    return Err(format!("系统回收未完成：{}，错误码={:?}", result.message, result.operation_error.or_else(|| result.items.first().map(|item| item.result_code))));
                }
                Ok(())
            })
        })();
        if let Err(error) = result
        {
            crate::log_warn!("收集箱：附件清理跳过或失败，正文保存不受影响，文档路径={document}，原因={error}");
        }
    })
    .await
    .map_err(|error| format!("附件清理任务失败：{error}"))
}

/// 将文件名安全编码为编辑窗查询参数，避免 `&` 等字符改变条目身份。
fn percent_encode_query(value: &str) -> String
{
    value
        .bytes()
        .flat_map(|byte| {
            if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte)
            {
                vec![byte as char]
            }
            else
            {
                format!("%{byte:02X}").chars().collect()
            }
        })
        .collect()
}
#[tauri::command]
pub fn get_collect_items(settings: State<'_, SettingsState>) -> Result<Vec<CollectItem>, String> { collect::list(Path::new(&config(&settings)?)).map_err(|e| format!("读取收集箱失败：{e}")) }
#[tauri::command]
pub fn prepare_collect_export(settings: State<'_, SettingsState>, names: Vec<String>) -> Result<collect::PreparedExport, String> { collect::prepare_export(Path::new(&config(&settings)?), &names) }
#[tauri::command]
pub fn receive_collect_items(app: AppHandle, settings: State<'_, SettingsState>, target_name: Option<String>, names: Vec<String>) -> Result<collect::ReceiveResult, String>
{
    let config = config(&settings)?; // 查询与写入使用同一配置快照。
    let result = with_collect_editor_state(&app, &collect::directory(&config), target_name.as_deref(), |editing| collect::receive_items(&config, target_name.as_deref(), &names, editing))?; // 编辑窗存在即逐项拒绝追加。
    changed(&app);
    Ok(result)
}
#[tauri::command]
pub fn create_collect_item(app: AppHandle, settings: State<'_, SettingsState>, content: String) -> Result<CollectItem, String>
{
    let item = collect::create(Path::new(&config(&settings)?), &content).map_err(|e| format!("创建收集项失败：{e}"))?;
    changed(&app);
    Ok(item)
}
#[tauri::command]
pub fn save_collect_item(app: AppHandle, settings: State<'_, SettingsState>, name: String, content: String) -> Result<(), String>
{
    collect::save(Path::new(&config(&settings)?), &name, &content).map_err(|e| format!("保存收集项失败：{e}"))?;
    changed(&app);
    Ok(())
}
#[tauri::command]
pub fn import_collect_resource(settings: State<'_, SettingsState>, name: String, source: String, mode: String) -> Result<String, String> { collect::import_resource(Path::new(&config(&settings)?), &name, Path::new(&source), &mode).map_err(|e| format!("收入图片失败：{e}")) }
#[tauri::command]
pub fn import_collect_rendered_resource(settings: State<'_, SettingsState>, name: String, reference: String, mode: String) -> Result<String, String> { collect::import_rendered_resource(Path::new(&config(&settings)?), &name, &reference, &mode).map_err(|e| format!("收入图片失败：{e}")) }
#[tauri::command]
pub fn write_collect_resource(settings: State<'_, SettingsState>, name: String, file_name: String, bytes: Vec<u8>) -> Result<String, String> { collect::write_resource_bytes(Path::new(&config(&settings)?), &name, &file_name, &bytes).map_err(|e| format!("写入图片资源失败：{e}")) }
#[tauri::command]
pub async fn rename_collect_item(app: AppHandle, old_name: String, new_name: String, draft: Option<String>) -> Result<String, String> { tauri::async_runtime::spawn_blocking(move || execute_rename_collect_item(app, old_name, new_name, draft)).await.map_err(|error| format!("重命名任务失败：{error}"))? }

/// 与窗口创建共用后台锁，主线程保持可处理窗口创建和标题更新。
pub(crate) fn execute_rename_collect_item(app: AppHandle, old_name: String, new_name: String, draft: Option<String>) -> Result<String, String>
{
    let settings = app.state::<SettingsState>(); // 后台任务读取当前配置。
    let config = config(&settings)?; // 当前共享配置目录。
    let root = collect::directory(&config); // 文件身份解析范围。
    let mut editors = EDITORS.lock().map_err(|_| "编辑窗状态不可用")?; // 与打开窗口共用互斥边界。
    let old_path = editor_path(&root, &old_name)?; // 改名前的文件身份。
    let renamed_draft = collect::rename(&config, &old_name, &new_name, draft.as_deref()).map_err(|e| format!("重命名收集项失败：{e}"))?; // 草稿仅转换返回，不写入磁盘。
    let new_path = editor_path(&root, &new_name)?; // 成功改名后的文件身份。
    if let Some(label) = editors.rename(&old_path, new_path)
        && let Some(window) = app.get_webview_window(&label)
    {
        let _ = window.set_title(&format!("编辑收集项 — {new_name}"));
    }
    let _ = app.emit("collect-items-changed", serde_json::json!({ "deletedName": old_name })); // 改名使旧身份失效，复用主动删除的缓存清理及暂停透传。
    Ok(renamed_draft)
}
#[tauri::command]
pub fn set_collect_item_pinned(app: AppHandle, settings: State<'_, SettingsState>, name: String, pinned: bool) -> Result<(), String>
{
    collect::set_pinned(Path::new(&config(&settings)?), &name, pinned).map_err(|e| format!("更新置顶失败：{e}"))?;
    changed(&app);
    Ok(())
}
#[tauri::command]
pub fn delete_collect_item(app: AppHandle, settings: State<'_, SettingsState>, name: String) -> Result<(), String>
{
    collect::delete(Path::new(&config(&settings)?), &name).map_err(|e| format!("删除收集项失败：{e}"))?;
    let _ = app.emit("collect-items-changed", serde_json::json!({ "deletedName": name })); // 主动删除清理所有格子的会话缓存。
    Ok(())
}
#[tauri::command]
pub fn restore_collect_item(app: AppHandle, settings: State<'_, SettingsState>) -> Result<Option<CollectItem>, String>
{
    let item = collect::restore_latest(Path::new(&config(&settings)?)).map_err(|e| format!("恢复收集项失败：{e}"))?;
    changed(&app);
    Ok(item)
}

#[cfg(test)]
mod tests
{
    use super::*;

    /// 不允许 Markdown 链接执行脚本、本地程序或访问应用内部协议。
    #[test]
    fn collect_links_reject_unsafe_protocols()
    {
        for url in ["javascript:alert(1)", "data:text/html,test", "file:///C:/test.exe", "desktop-tools://test", "https://example.com/\n"]
        {
            assert!(open_collect_link(url.to_string()).is_err());
        }
    }

    /// 同文件去重、不同文件并行以及改名后旧名称复用均保持窗口隔离。
    #[test]
    fn editor_windows_are_scoped_to_each_file()
    {
        let mut editors = EditorWindows::default(); // 独立会话关联。
        let first = std::path::PathBuf::from("first.md"); // 第一份文件。
        let second = std::path::PathBuf::from("second.md"); // 第二份文件。
        let first_label = editors.label(first.clone()); // 第一扇窗口。
        assert_eq!(editors.label(first.clone()), first_label);
        assert_ne!(editors.label(second.clone()), first_label);
        let renamed = std::path::PathBuf::from("renamed.md"); // 改名后的同一文件。
        assert_eq!(editors.rename(&first, renamed.clone()), Some(first_label.clone()));
        assert_eq!(editors.label(renamed.clone()), first_label);
        assert_ne!(editors.label(first), first_label);
        editors.labels.remove(&renamed);
        assert_ne!(editors.label(renamed), first_label);
    }

    /// 文件名经校验后按真实路径去重，路径越界和缺失文件不能建立窗口。
    #[test]
    fn editor_paths_validate_files_and_windows_case()
    {
        let root = std::env::temp_dir().join(format!("desktop-tools-editor-{}", std::process::id())); // 测试专用目录。
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("Note.md"), "content").unwrap();
        assert_eq!(editor_path(&root, "Note.md").unwrap(), editor_path(&root, "note.md").unwrap());
        assert!(editor_path(&root, "../Note.md").is_err());
        assert!(editor_path(&root, "missing.md").is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
