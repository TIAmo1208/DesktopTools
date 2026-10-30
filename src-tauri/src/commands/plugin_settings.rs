use tauri::{AppHandle, Emitter, Manager, State};

use crate::app::settings::SettingsState;
use crate::component_runtime::ComponentRuntimeState;

/// 读取一个插件的设置；没有文件或内容损坏时返回空，不改写磁盘。清单缺失不猜键。
#[tauri::command]
pub fn get_plugin_settings(app: AppHandle, state: State<'_, SettingsState>, plugin_id: String) -> Result<Option<serde_json::Value>, String>
{
    let author = plugin_author(&app, &plugin_id)?; // 键里的作者来自当前清单。
    crate::app::settings::read_plugin_settings(&state.directory()?, &author, &plugin_id)
}

fn plugin_author(app: &AppHandle, plugin_id: &str) -> Result<String, String>
{
    let manifests = app.state::<ComponentRuntimeState>().snapshot(&[]).map_err(|error| error.message)?.manifests; // 当前已注册的清单。
    crate::app::settings::plugin_settings_author(&manifests, plugin_id)
}

/// 只打开该插件清单里声明的仓库，不接受调用方传入的地址。
#[tauri::command]
pub fn open_plugin_repository(app: AppHandle, plugin_id: String) -> Result<(), String>
{
    let manifests = app.state::<ComponentRuntimeState>().snapshot(&[]).map_err(|error| error.message)?.manifests; // 当前已注册的清单。
    let repository = manifests.iter().find(|manifest| manifest.id == plugin_id).ok_or_else(|| "插件清单不存在".to_string())?.repository.clone();
    let url = crate::component_runtime::declared_plugin_repository(&repository)?;
    crate::platform::windows::settings::open_path(std::path::Path::new(url))
}

/// 原子保存一个插件的设置，并通知读者重新读取。
#[tauri::command]
pub fn save_plugin_settings(app: AppHandle, state: State<'_, SettingsState>, plugin_id: String, value: serde_json::Value) -> Result<(), String>
{
    let author = plugin_author(&app, &plugin_id)?; // 键里的作者来自当前清单。
    crate::app::settings::write_plugin_settings(&state.directory()?, &author, &plugin_id, &value)?;
    if let Err(error) = app.emit(crate::contracts::PLUGIN_SETTINGS_CHANGED_EVENT, serde_json::json!({ "pluginId": &plugin_id }))
    {
        crate::log_warn!("插件设置：变更通知失败，插件={plugin_id}，错误：{error}");
    }
    Ok(())
}

/// 卸载命令返回实际删除的文件名。清理配置时包含设置文件。
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UninstallPluginResult
{
    pub removed: Vec<String>, // 本次删掉的清单、脚本或设置文件名。
}

/// 设置页创建插件格子前的清单门禁。不满足时不创建。
pub(crate) fn validate_creatable_plugin(manifests: &[crate::contracts::ComponentManifest], plugin_id: &str) -> Result<(), String>
{
    if !plugin_id.starts_with("plugin.") || plugin_id.len() == "plugin.".len()
    {
        crate::log_warn!("插件创建：拒绝非插件标识，插件={plugin_id}");
        return Err("插件标识无效".to_string());
    }
    let Some(manifest) = manifests.iter().find(|manifest| manifest.id == plugin_id)
    else
    {
        crate::log_warn!("插件创建：清单不存在，插件={plugin_id}");
        return Err("插件清单不存在".to_string());
    }; // 启动时冻结的清单，不在这里重新读盘。
    if manifest.entry != "plugin:slot" || !manifest.creatable
    {
        crate::log_warn!("插件创建：清单不允许创建，插件={plugin_id}，入口={}，可创建={}", manifest.entry, manifest.creatable);
        return Err("该插件不支持创建".to_string());
    }
    Ok(())
}

/// 在主显示器上按默认格子尺寸创建一个插件实例。
#[tauri::command]
pub fn create_plugin_component(app: AppHandle, plugin_id: String) -> Result<(), String>
{
    let manifests = app.state::<ComponentRuntimeState>().snapshot(&[]).map_err(|error| error.message)?.manifests; // 当前运行时已经注册的清单。
    validate_creatable_plugin(&manifests, &plugin_id)?;
    let workspace = app.state::<crate::workspace::WorkspaceState>(); // 统一布局事务。
    let existing = workspace.snapshot().map_err(|error| error.message)?.components.into_iter().filter(|instance| instance.component_id == plugin_id).count(); // 所有显示器上的该插件格子。
    let max_tiles = manifests.iter().find(|manifest| manifest.id == plugin_id).map(|manifest| manifest.max_tiles).unwrap_or(0);
    crate::component_runtime::enforce_plugin_tile_limit(max_tiles, existing)?;
    let runtime = app.state::<std::sync::Arc<crate::host::DesktopHostRuntime>>(); // 用来取主显示器。
    let monitor_id = super::primary_monitor_id(&runtime)?; // 与收集箱创建入口同一台主显示器。
    let bounds = workspace.next_component_bounds(&monitor_id, super::DEFAULT_TILE_WIDTH, super::DEFAULT_TILE_HEIGHT).map_err(|error| error.message)?; // 现有避让算法算出的位置。
    super::workspace::execute_apply_workspace_command(&app, &workspace, &app.state::<crate::shell::DesktopItemState>(), &runtime, monitor_id.clone(), crate::contracts::WorkspaceCommand::CreateComponent { component_id: plugin_id.clone(), bounds, state: serde_json::json!({}) }).map_err(|error| error.message)?;
    crate::log_info!("插件：已创建格子，插件={plugin_id}，显示器={monitor_id}");
    Ok(())
}

/// 先校验文件，再移除全部显示器上的实例，最后删文件。工作区失败时文件仍在。
#[tauri::command]
pub fn uninstall_plugin(app: AppHandle, plugin_id: String, remove_settings: bool) -> Result<UninstallPluginResult, String>
{
    let author = if remove_settings { plugin_author(&app, &plugin_id)? } else { String::new() }; // 清理配置必须先拿到清单里的作者。
    let directory = app.state::<SettingsState>().directory()?; // 当前生效的配置目录。
    let pending = crate::app::settings::plan_plugin_removal(app.state::<ComponentRuntimeState>().plugin_directory(), &directory, &plugin_id, remove_settings, &author)?; // 非法路径在改工作区前拒绝。
    let snapshot = app.state::<crate::workspace::WorkspaceState>().remove_uninstalled_plugin(&plugin_id).map_err(|error| error.message)?; // 不走用户移除门禁，也不进撤销历史。
    let mut removed = crate::app::settings::commit_plugin_removal(pending)?; // 实例已清掉后再删文件，失败时可重入补删。
    if remove_settings && crate::app::settings::remove_plugin_settings(&directory, &author, &plugin_id)?
    {
        removed.push("settings-plugins.json".to_string());
    }
    super::workspace::publish_workspace_snapshot(&app, &snapshot).map_err(|error| error.message)?;
    if let Err(error) = publish_plugin_catalog(&app)
    {
        crate::log_warn!("插件清单：卸载后刷新失败，错误：{error}");
    }
    Ok(UninstallPluginResult { removed })
}

/// 选中默认插件目录时返回空串，前端据此把偏好写成跟随默认。
pub(crate) fn plugin_directory_preference(chosen: &str, config_directory: &std::path::Path) -> String
{
    let follows_default = crate::app::settings::resolve_plugin_directory("", config_directory).is_ok_and(|default| same_directory(std::path::Path::new(chosen), &default)); // 空偏好解析出的同级 plugin。
    if follows_default { String::new() } else { chosen.to_string() }
}

/// 比较两个目录，存在时按实际路径，否则按文本忽略大小写。
fn same_directory(left: &std::path::Path, right: &std::path::Path) -> bool
{
    match (std::fs::canonicalize(left), std::fs::canonicalize(right))
    {
        (Ok(left), Ok(right)) => left == right,
        _ => left.to_string_lossy().eq_ignore_ascii_case(&right.to_string_lossy()),
    }
}

/// 在设置窗口选择插件目录。取消不改变偏好。
#[tauri::command]
pub async fn choose_plugin_directory(window: tauri::WebviewWindow) -> Result<Option<String>, String>
{
    if window.label() != "main"
    {
        return Err("目录选择仅适用于设置窗口".to_string());
    }
    let owner = window.hwnd().map_err(|error| error.to_string())?.0 as isize; // 设置窗口原生父句柄。
    let (sender, receiver) = std::sync::mpsc::sync_channel(1); // 主线程弹窗返回通道。
    window
        .run_on_main_thread(move || {
            let _ = sender.send(crate::platform::windows::settings::choose_directory(owner, "选择插件目录"));
        })
        .map_err(|error| error.to_string())?;
    let chosen = tauri::async_runtime::spawn_blocking(move || receiver.recv().map_err(|error| error.to_string())).await.map_err(|error| error.to_string())???; // 取消时没有路径。
    let Some(chosen) = chosen
    else
    {
        return Ok(None);
    };
    Ok(Some(plugin_directory_preference(&chosen, &window.state::<SettingsState>().directory()?)))
}

/// 返回本次启动冻结的插件根，不读取后来改过的偏好。
#[tauri::command]
pub fn resolved_plugin_directory(state: State<'_, ComponentRuntimeState>) -> String { state.plugin_directory().to_string_lossy().into_owned() }

/// 在资源管理器中选中该插件的清单；沿用日志目录同一 ShellExecuteW，不另起进程封装。
#[tauri::command]
pub fn open_plugin_directory(app: AppHandle, plugin_id: String) -> Result<(), String>
{
    let manifest = crate::app::settings::plugin_manifest_file(app.state::<ComponentRuntimeState>().plugin_directory(), &plugin_id)?; // 已确认位于插件根内的普通文件。
    reveal_plugin_manifest(&manifest)
}

/// 还没落盘的安装计划。取消标记和来源一起放着。
struct PluginInstallTask
{
    prepared: crate::app::settings::PreparedPlugin,        // 校验后的来源。
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>, // 解压和复制循环看这个标记。
    running: std::sync::atomic::AtomicBool,                // 同一计划不能同时执行两次。
}

fn install_tasks() -> &'static std::sync::Mutex<std::collections::HashMap<String, PluginInstallTask>>
{
    static TASKS: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, PluginInstallTask>>> = std::sync::OnceLock::new();
    TASKS.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

/// 返回给设置页的计划。冲突只描述版本，不写文件。
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginInstallPlanView
{
    plan_id: String,                             // 之后执行或取消用的标识。
    name: String,                                // 插件名称。
    author: String,                              // 待安装清单里的作者，没有就是空。
    plugin_id: String,                           // 已安装目标的标识；新安装还没生成，留空。
    version: String,                             // 待安装版本。
    target_folder: String,                       // 插件根下的目标文件夹。
    source_name: String,                         // 进度里的来源名。
    conflict: Option<PluginInstallConflictView>, // 目标已存在时才有。
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginInstallConflictView
{
    installed_version: String, // 已装版本。
    incoming_version: String,  // 待装版本。
    downgrade: bool,           // 是否降级。
}

fn remember_install(prepared: crate::app::settings::PreparedPlugin) -> PluginInstallPlanView
{
    let plan_id = format!("install-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|value| value.as_nanos()).unwrap_or(0));
    let plugin_id = prepared.conflict.as_ref().map(|conflict| conflict.installed_id.clone()).unwrap_or_default(); // 新安装的 id 要等执行时才生成。
    let view = PluginInstallPlanView { plan_id: plan_id.clone(), name: prepared.name.clone(), author: prepared.author.clone(), plugin_id, version: prepared.version.clone(), target_folder: prepared.name.clone(), source_name: prepared.source_name.clone(), conflict: prepared.conflict.as_ref().map(|conflict| PluginInstallConflictView { installed_version: conflict.installed_version.clone(), incoming_version: conflict.incoming_version.clone(), downgrade: conflict.downgrade }) };
    install_tasks().lock().unwrap_or_else(|error| error.into_inner()).insert(plan_id, PluginInstallTask { prepared, cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)), running: std::sync::atomic::AtomicBool::new(false) });
    view
}

fn settings_window(window: &tauri::WebviewWindow) -> Result<(), String>
{
    if window.label() != "main"
    {
        return Err("插件安装仅适用于设置窗口".to_string());
    }
    Ok(())
}

fn choose_install_folders(owner: isize) -> Result<Vec<String>, String> { crate::platform::windows::settings::choose_directories(owner, "选择插件文件夹") }

async fn choose_on_main(window: &tauri::WebviewWindow, choose: fn(isize) -> Result<Vec<String>, String>) -> Result<Vec<String>, String>
{
    let owner = window.hwnd().map_err(|error| error.to_string())?.0 as isize; // 设置窗口原生父句柄。
    let (sender, receiver) = std::sync::mpsc::sync_channel(1); // 主线程弹窗返回通道。
    window
        .run_on_main_thread(move || {
            let _ = sender.send(choose(owner));
        })
        .map_err(|error| error.to_string())?;
    tauri::async_runtime::spawn_blocking(move || receiver.recv().map_err(|error| error.to_string())).await.map_err(|error| error.to_string())??
}

/// 一次选择的安装计划。校验失败的来源记在 errors 里，不丢掉其余计划。
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginInstallPreparation
{
    pub plans: Vec<PluginInstallPlanView>, // 可以执行的计划。
    pub errors: Vec<String>,               // 没通过校验的来源。
}

fn prepare_sources(paths: Vec<String>, plugin_root: &std::path::Path, inspect: fn(&std::path::Path, &std::path::Path) -> Result<crate::app::settings::PreparedPlugin, String>) -> PluginInstallPreparation
{
    let mut plans = Vec::new(); // 校验通过的计划。
    let mut errors = Vec::new(); // 单个来源的失败原因。
    for path in paths
    {
        match inspect(std::path::Path::new(&path), plugin_root)
        {
            Ok(prepared) => plans.push(remember_install(prepared)),
            Err(error) => errors.push(error),
        }
    }
    PluginInstallPreparation { plans, errors }
}

/// 选择一个或多个文件夹并给出安装计划。取消时计划为空。
#[tauri::command]
pub async fn prepare_plugin_install_from_folder(window: tauri::WebviewWindow) -> Result<PluginInstallPreparation, String>
{
    settings_window(&window)?;
    let chosen = choose_on_main(&window, choose_install_folders).await?;
    Ok(prepare_sources(chosen, window.state::<ComponentRuntimeState>().plugin_directory(), crate::app::settings::inspect_plugin_folder))
}

/// 选择一个或多个 zip 并给出安装计划。取消时计划为空。
#[tauri::command]
pub async fn prepare_plugin_install_from_archive(window: tauri::WebviewWindow) -> Result<PluginInstallPreparation, String>
{
    settings_window(&window)?;
    let chosen = choose_on_main(&window, crate::platform::windows::settings::choose_zip_archives).await?;
    Ok(prepare_sources(chosen, window.state::<ComponentRuntimeState>().plugin_directory(), crate::app::settings::inspect_plugin_archive))
}

/// 安装结果。设置窗口收不到桌面宿主那条事件转发，所以快照直接跟回来。
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginInstallApplied
{
    pub plugin_id: String,                                    // 装好的插件 id。
    pub snapshot: crate::contracts::ComponentRuntimeSnapshot, // 刷新后的运行时清单。
}

/// 执行已确认的计划。复制和解压在阻塞线程，界面线程保持可响应。
#[tauri::command]
pub async fn apply_plugin_install(app: AppHandle, plan_id: String, confirm_downgrade: bool) -> Result<PluginInstallApplied, String>
{
    let task = {
        let tasks = install_tasks().lock().unwrap_or_else(|error| error.into_inner());
        let task = tasks.get(&plan_id).ok_or_else(|| "安装任务不存在".to_string())?;
        if task.running.swap(true, std::sync::atomic::Ordering::Relaxed)
        {
            return Err("安装已在进行".to_string());
        }
        PluginInstallTask { prepared: task.prepared.clone(), cancel: std::sync::Arc::clone(&task.cancel), running: std::sync::atomic::AtomicBool::new(true) }
    };
    let app_for_install = app.clone(); // 阻塞线程里发进度和卸载。
    let plan_for_install = plan_id.clone();
    let result = tauri::async_runtime::spawn_blocking(move || run_plugin_install(app_for_install, plan_for_install, task, confirm_downgrade)).await.map_err(|error| format!("安装插件失败：{error}"))?;
    install_tasks().lock().unwrap_or_else(|error| error.into_inner()).remove(&plan_id);
    let plugin_id = result?;
    let snapshot = publish_plugin_catalog(&app)?;
    crate::log_info!("插件安装：已完成，插件={plugin_id}");
    Ok(PluginInstallApplied { plugin_id, snapshot })
}

fn run_plugin_install(app: AppHandle, plan_id: String, task: PluginInstallTask, confirm_downgrade: bool) -> Result<String, String>
{
    let source_name = task.prepared.source_name.clone(); // 进度事件里的来源名。
    let mut last_stage = String::new(); // 上次发出的阶段。
    let mut last_percent = 0u8; // 上次发出的百分比。
    let mut last_at = std::time::Instant::now() - std::time::Duration::from_secs(1); // 第一次进度允许立即发出。
    let emit = |stage: &str, percent: u8, last_stage: &mut String, last_percent: &mut u8, last_at: &mut std::time::Instant| {
        let stage_changed = stage != last_stage;
        let due = crate::app::settings::install_progress_due(stage_changed, last_at.elapsed().as_millis(), percent.saturating_sub(*last_percent));
        if !due
        {
            return;
        }
        if let Err(error) = app.emit("plugin-install-progress", serde_json::json!({ "taskId": &plan_id, "stage": stage, "percent": percent, "sourceName": &source_name }))
        {
            crate::log_warn!("插件安装：进度通知失败，任务={plan_id}，错误：{error}");
        }
        *last_stage = stage.to_string();
        *last_percent = percent;
        *last_at = std::time::Instant::now();
    };
    let result = crate::app::settings::install_prepared_plugin(app.state::<ComponentRuntimeState>().plugin_directory(), &task.prepared, confirm_downgrade, &task.cancel, |plugin_id| uninstall_plugin(app.clone(), plugin_id.to_string(), false).map(|_| ()), |stage, percent| emit(stage, percent, &mut last_stage, &mut last_percent, &mut last_at));
    if let Err(error) = &result
    {
        emit("failed", last_percent, &mut last_stage, &mut last_percent, &mut last_at);
        crate::log_warn!("插件安装：失败，任务={plan_id}，原因={error}");
    }
    result
}

/// 一个来源的安装结果。失败不影响后面的计划。
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginInstallTaskResult
{
    pub plan_id: String,                                              // 对应的计划。
    pub plugin_id: Option<String>,                                    // 成功时的插件 id。
    pub snapshot: Option<crate::contracts::ComponentRuntimeSnapshot>, // 该次成功后的运行时清单。
    pub error: Option<String>,                                        // 失败原因。
}

/// 按给定顺序逐个安装。进度事件用各自的计划 id。
#[tauri::command]
pub async fn apply_plugin_installs(app: AppHandle, plan_ids: Vec<String>, confirm_downgrade: bool) -> Vec<PluginInstallTaskResult>
{
    let mut results = Vec::new(); // 保持选择顺序，单个失败不中断。
    for plan_id in plan_ids
    {
        results.push(match apply_plugin_install(app.clone(), plan_id.clone(), confirm_downgrade).await
        {
            Ok(applied) => PluginInstallTaskResult { plan_id, plugin_id: Some(applied.plugin_id), snapshot: Some(applied.snapshot), error: None },
            Err(error) => PluginInstallTaskResult { plan_id, plugin_id: None, snapshot: None, error: Some(error) },
        });
    }
    results
}

fn publish_plugin_catalog(app: &AppHandle) -> Result<crate::contracts::ComponentRuntimeSnapshot, String>
{
    let runtime = app.state::<ComponentRuntimeState>();
    runtime.reload_plugins();
    let workspace = app.state::<crate::workspace::WorkspaceState>();
    workspace.refresh_capability_declarations(&runtime.capability_declarations());
    let snapshot = super::component::component_runtime_snapshot(&runtime, &workspace).map_err(|error| format!("插件清单刷新失败：{}", error.message))?;
    if let Err(error) = app.emit(crate::contracts::COMPONENT_RUNTIME_SNAPSHOT_CHANGED_EVENT, &snapshot)
    {
        crate::log_warn!("插件清单：刷新通知失败，错误：{error}");
    }
    Ok(snapshot)
}

/// 让正在进行的解压或复制停下来，并在循环里清掉临时目录。
#[tauri::command]
pub fn cancel_plugin_install(task_id: String) -> Result<(), String>
{
    let tasks = install_tasks().lock().unwrap_or_else(|error| error.into_inner());
    let task = tasks.get(&task_id).ok_or_else(|| "安装任务不存在".to_string())?;
    task.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    Ok(())
}

#[cfg(test)]
#[path = "plugin_settings.test.rs"]
mod tests;

/// 用资源管理器选中清单。open_path 会按文件关联打开，open_log_directory 不能选中文件。
fn reveal_plugin_manifest(path: &std::path::Path) -> Result<(), String>
{
    use windows::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};
    use windows::core::{PCWSTR, w};
    let parameter: Vec<u16> = format!("/select,\"{}\"", path.display()).encode_utf16().chain(Some(0)).collect(); // 选中清单，而不是打开目录本身。
    let result = unsafe { ShellExecuteW(None, w!("open"), w!("explorer.exe"), PCWSTR(parameter.as_ptr()), None, SW_SHOWNORMAL) }; // 与打开日志目录相同的 Shell 调用。
    if result.0 as isize <= 32
    {
        return Err(format!("打开插件目录失败，ShellExecuteW：{}", result.0 as isize));
    }
    Ok(())
}
