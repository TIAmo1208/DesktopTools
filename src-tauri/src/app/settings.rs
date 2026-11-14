use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};

use crate::contracts::settings::ApplicationPreferences;
use crate::persistence::{load_json, save_json_atomic};

/// 展开默认环境变量并拒绝相对路径、父级跳转和 Windows 特殊设备路径。
pub fn resolve_config_directory(value: &str) -> Result<PathBuf, String>
{
    let value = value.trim(); // 去除输入首尾空白后的配置路径。
    let expanded = if value.get(..9).is_some_and(|prefix| prefix.eq_ignore_ascii_case("%APPDATA%"))
    {
        let root = std::env::var("APPDATA").map_err(|_| "无法读取 APPDATA 目录")?; // 当前用户漫游配置根。
        format!("{root}{}", &value[9..])
    }
    else
    {
        value.to_string()
    }; // 不展开未知变量，避免静默写入错误位置。
    let path = PathBuf::from(&expanded); // 待验证的绝对目录。
    if directory_outside_boundary(&expanded, &path)
    {
        return Err("配置目录必须为绝对路径或以 %APPDATA% 开头，不能包含父级跳转或特殊字符".to_string());
    }
    Ok(path)
}

/// 配置路径与启动定位指针共用的目录安全边界：拒绝相对路径、父级跳转、设备前缀和特殊字符。
fn directory_outside_boundary(value: &str, path: &Path) -> bool
{
    use std::path::Component;
    !path.is_absolute() || value.contains(['%', '\0', '*', '?', '"', '<', '>', '|']) || value.starts_with(r"\\.\") || value.starts_with(r"\\?\") || path.components().any(|part| matches!(part, Component::ParentDir) || matches!(part, Component::Normal(value) if value.to_string_lossy().contains(':')))
}

/// 空偏好使用可执行文件所在目录下的 plugin；显式路径沿用配置目录安全边界。
pub fn resolve_plugin_directory(value: &str, config_directory: &std::path::Path) -> Result<PathBuf, String>
{
    let _ = config_directory; // 显式路径不相对配置目录解析。
    if value.trim().is_empty()
    {
        let executable = std::env::current_exe().map_err(|error| format!("无法定位程序以解析插件目录：{error}"))?; // 安装与开发运行均以实际可执行文件为准。
        let directory = executable.parent().ok_or("无法读取程序所在目录")?;
        return Ok(directory.join("plugin"));
    }
    resolve_config_directory(value).map_err(|error| error.replace("配置目录", "插件目录"))
}

/// 在保存偏好或关联布局前完成格式与实际路径校验。
pub(crate) fn validate_preferences(preferences: &ApplicationPreferences) -> Result<PathBuf, String>
{
    preferences.validate()?;
    let config_directory = resolve_config_directory(&preferences.config_directory)?; // 两个目录在写入前统一完成解析。
    resolve_plugin_directory(&preferences.plugin_directory, &config_directory)?;
    Ok(config_directory)
}

/// 在加载任何写入线程前切换数据目录；固定引导文件只记录当前生效位置。
pub fn prepare_config_directory(bootstrap: &std::path::Path) -> std::io::Result<PathBuf>
{
    let pointer = bootstrap.join(CONFIG_LOCATION_FILE); // 不随配置目录迁移的启动定位文件。
    let source = current_config_directory(bootstrap)?; // 兼容旧版固定数据目录。
    let preferences: ApplicationPreferences = load_json(&source.join("settings.json"))?.unwrap_or_default(); // 从当前目录读取待迁移意图。
    let target = validate_preferences(&preferences).map_err(std::io::Error::other)?; // 下次启动目标。
    if source == target
    {
        return Ok(source);
    }

    match migrate_config_directory(&source, &target, &pointer)
    {
        Ok(()) =>
        {
            crate::log_debug!("设置：配置目录已迁移，原目录={}，当前目录={}，原文件保留", source.display(), target.display());
            Ok(target)
        }
        Err(error) =>
        {
            crate::log_error!("设置：配置目录迁移失败，继续使用原目录={}，目标={}，错误={error}", source.display(), target.display());
            Ok(source)
        }
    }
}

const CONFIG_LOCATION_FILE: &str = "config-location.json"; // 启动定位目录里的配置目录指针。

/// 当前配置目录：启动定位文件优先，缺失时仍使用启动定位目录本身。
fn current_config_directory(bootstrap: &std::path::Path) -> std::io::Result<PathBuf> { Ok(load_json(&bootstrap.join(CONFIG_LOCATION_FILE))?.unwrap_or_else(|| bootstrap.to_path_buf())) }

/// 复制数据并最后提交启动位置；拒绝非空目标，失败不删除或覆盖原数据。
fn migrate_config_directory(source: &std::path::Path, target: &std::path::Path, pointer: &std::path::Path) -> std::io::Result<()>
{
    use std::{fs, io};
    fs::create_dir_all(source)?;
    fs::create_dir_all(target)?;
    let source_real = fs::canonicalize(source)?; // 消除大小写、链接和路径别名差异，仅用于判断与复制。
    let target_real = fs::canonicalize(target)?; // 创建后验证最终真实位置。
    if source_real == target_real
    {
        return Ok(());
    }
    if source_real.starts_with(&target_real) || target_real.starts_with(&source_real) || fs::read_dir(&target_real)?.next().is_some()
    {
        return Err(io::Error::other("目录不能互相包含，且目标目录必须为空"));
    }
    // 整目录复制，不再按固定名单挑选；不跟随链接或重解析点，插件设置因此一起离开。
    let mut pending = vec![(source_real.clone(), target_real)]; // 待复制的目录或文件，根目录本身已经存在。
    while let Some((from, to)) = pending.pop()
    {
        use std::os::windows::fs::MetadataExt;
        let metadata = match fs::symlink_metadata(&from)
        {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        }; // 复制过程中消失的项跳过，其他读取错误不得忽略。
        if metadata.file_attributes() & 0x400 != 0
        {
            return Err(io::Error::other("目录迁移不支持链接或重解析点"));
        }
        if metadata.is_dir()
        {
            if from != source_real
            {
                fs::create_dir(&to)?;
            }
            for entry in fs::read_dir(&from)?
            {
                let entry = entry?; // 当前子项。
                pending.push((entry.path(), to.join(entry.file_name())));
            }
        }
        else
        {
            let mut input = fs::File::open(&from)?; // 原数据仅以只读方式打开。
            let mut output = fs::OpenOptions::new().write(true).create_new(true).open(&to)?; // 永不覆盖现有文件。
            io::copy(&mut input, &mut output)?;
            output.sync_all()?;
        }
    }
    // 复制完成后才切换指针；写调用方传入的目标绝对路径，避免 canonicalize 的 verbatim 形式进入指针。
    save_json_atomic(pointer, &target)
}

/// 持有配置文件路径，并串行提交已校验的用户偏好。
pub struct SettingsState
{
    path: PathBuf,                              // 当前 Windows 用户独占的偏好文件。
    preferences: Mutex<ApplicationPreferences>, // 仅保存成功后更新的权威内存状态。
}

/// 提供设置恢复与原子更新操作。
impl SettingsState
{
    /// 启动时恢复配置；损坏文件保持原样并返回明确错误。
    pub fn load(path: PathBuf) -> std::io::Result<Self>
    {
        let preferences: ApplicationPreferences = load_json(&path)?.unwrap_or_default(); // 当前已恢复的偏好或首次启动默认值。
        validate_preferences(&preferences).map_err(std::io::Error::other)?;
        Ok(Self { path, preferences: Mutex::new(preferences) })
    }

    /// 返回已保存的偏好，不向调用方泄漏可变状态。
    pub fn preferences(&self) -> Result<ApplicationPreferences, String> { self.preferences.lock().map(|value| value.clone()).map_err(|_| "设置状态不可用".to_string()) }

    /// 返回当前生效的配置目录；尚未重启的目录偏好不改变这个位置。
    pub fn directory(&self) -> Result<PathBuf, String> { self.path.parent().map(Path::to_path_buf).ok_or_else(|| "设置路径缺少配置目录".to_string()) }

    /// 保存成功后替换内存状态，失败时保留原值。
    pub fn save(&self, mut preferences: ApplicationPreferences) -> Result<ApplicationPreferences, String>
    {
        preferences.disabled_plugins = crate::contracts::settings::normalize_disabled_plugins(std::mem::take(&mut preferences.disabled_plugins)); // 手写结构也走同一条丢弃规则。
        validate_preferences(&preferences)?;
        let mut current = self.preferences.lock().map_err(|_| "设置状态不可用".to_string())?; // 当前串行提交锁。
        self.commit(&mut current, preferences)
    }

    /// 在同一提交锁内基于最新偏好更新字段，patch 或落盘失败均不改变内存事实。
    pub fn update(&self, patch: impl FnOnce(&mut ApplicationPreferences) -> Result<(), String>) -> Result<ApplicationPreferences, String>
    {
        let mut current = self.preferences.lock().map_err(|_| "设置状态不可用".to_string())?; // 与整份保存共用串行锁。
        let mut next = current.clone(); // 修改候选副本，失败不泄漏部分字段。
        patch(&mut next)?;
        next.disabled_plugins = crate::contracts::settings::normalize_disabled_plugins(std::mem::take(&mut next.disabled_plugins));
        validate_preferences(&next)?;
        self.commit(&mut current, next)
    }

    /// 两种保存入口共用唯一的落盘与成功提交逻辑；调用方必须持有偏好锁。
    fn commit(&self, current: &mut ApplicationPreferences, preferences: ApplicationPreferences) -> Result<ApplicationPreferences, String>
    {
        save_json_atomic(&self.path, &preferences).map_err(|error| format!("保存设置失败：{error}"))?;
        *current = preferences;
        crate::log_info!("设置：用户偏好已保存");
        Ok(current.clone())
    }
}

#[cfg(test)]
mod atomic_update_tests
{
    use super::*;

    #[test]
    fn settings_atomic_patch_serializes_and_preserves_state_on_every_failure()
    {
        let directory = std::env::temp_dir().join(format!("desktop-tools-settings-patch-{}", std::process::id())); // 独占测试路径。
        let path = directory.join("settings.json");
        let state = std::sync::Arc::new(SettingsState::load(path.clone()).unwrap());
        state
            .update(|preferences| {
                preferences.island_enabled = true;
                Ok(())
            })
            .unwrap();
        let workers = (0..8)
            .map(|_| {
                let state = std::sync::Arc::clone(&state); // 竞争真实 SettingsState 提交锁。
                std::thread::spawn(move || {
                    state
                        .update(|preferences| {
                            preferences.island_offset += 1.0;
                            Ok(())
                        })
                        .unwrap();
                })
            })
            .collect::<Vec<_>>();
        for worker in workers
        {
            worker.join().unwrap();
        }
        let original = state.preferences().unwrap();
        assert_eq!(original.island_offset, 8.0); // 每次 patch 读取最新值，而非锁外快照。
        let bytes = std::fs::read(&path).unwrap();
        assert!(
            state
                .update(|preferences| {
                    preferences.island_offset = 10.0;
                    Err("注入 patch 失败".into())
                })
                .is_err()
        );
        assert!(
            state
                .update(|preferences| {
                    preferences.island_offset = f64::NAN;
                    Ok(())
                })
                .is_err()
        );
        assert_eq!(state.preferences().unwrap(), original);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        let blocked = SettingsState { path: path.join("blocked.json"), preferences: Mutex::new(original.clone()) }; // 文件作为父目录，注入真实写盘失败。
        assert!(
            blocked
                .update(|preferences| {
                    preferences.island_offset = 30.0;
                    Ok(())
                })
                .is_err()
        );
        assert_eq!(blocked.preferences().unwrap(), original);
        assert_eq!(SettingsState::load(path).unwrap().preferences().unwrap(), original);
        std::fs::remove_dir_all(directory).unwrap();
    }
}

const PLUGIN_SETTINGS_FILE: &str = "settings-plugins.json"; // 配置目录里唯一的插件设置文件。

/// 由作者和清单 id 得到设置键。作者为空时键以 @ 开头。非法 id 不产生键。
pub(crate) fn plugin_settings_key(author: &str, plugin_id: &str) -> Result<String, String>
{
    let Some(name) = plugin_id.strip_prefix("plugin.")
    else
    {
        return Err("插件标识无效".to_string());
    }; // 只去掉一次 plugin. 前缀，官方 id 不能写入插件设置。
    if name.is_empty() || name.len() > 240 || !name.chars().all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-'))
    {
        return Err("插件标识无效".to_string());
    }
    Ok(format!("{author}@{plugin_id}"))
}

/// 清单缺失时不拼键，避免把设置写到只有 id 的位置。
pub(crate) fn plugin_settings_author(manifests: &[crate::contracts::ComponentManifest], plugin_id: &str) -> Result<String, String> { manifests.iter().find(|manifest| manifest.id == plugin_id).map(|manifest| manifest.author.clone()).ok_or_else(|| "插件清单不存在".to_string()) }

fn plugin_settings_store(config_directory: &Path) -> PathBuf { config_directory.join(PLUGIN_SETTINGS_FILE) }

/// 设置文件不能是链接或重解析点，否则读写会落到配置目录外面。
fn reject_plugin_settings_link(config_directory: &Path) -> Result<(), String>
{
    use std::os::windows::fs::MetadataExt;
    let path = plugin_settings_store(config_directory); // 统一设置文件。
    match std::fs::symlink_metadata(&path)
    {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("读取插件设置失败：{error}")),
        Ok(metadata) if metadata.file_type().is_symlink() || metadata.file_attributes() & 0x400 != 0 => Err("插件设置文件不能是链接或重解析点".to_string()),
        Ok(_) => Ok(()),
    }
}

/// 读出整份设置。文件缺失、损坏或不是对象时都不改写磁盘。
fn load_plugin_settings_store(path: &Path) -> Result<Option<serde_json::Map<String, serde_json::Value>>, String>
{
    match std::fs::read(path)
    {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("读取插件设置失败：{error}")),
        Ok(bytes) => match serde_json::from_slice::<serde_json::Value>(&bytes)
        {
            Ok(serde_json::Value::Object(map)) => Ok(Some(map)),
            Ok(_) =>
            {
                crate::log_warn!("插件设置：内容不是对象，已保留原文件，路径={}", path.display());
                Err("插件设置文件已损坏，未覆盖".to_string())
            }
            Err(error) =>
            {
                crate::log_warn!("插件设置：内容不是合法 JSON，已保留原文件，路径={}，错误={error}", path.display());
                Err("插件设置文件已损坏，未覆盖".to_string())
            }
        },
    }
}

/// 读取一个插件的设置；文件缺失或损坏都不改写磁盘。损坏时返回空。
pub(crate) fn read_plugin_settings(config_directory: &Path, author: &str, plugin_id: &str) -> Result<Option<serde_json::Value>, String>
{
    let key = plugin_settings_key(author, plugin_id).inspect_err(|_| crate::log_warn!("插件设置：拒绝非法插件标识，插件={plugin_id}"))?; // 作者@插件ID。
    let path = plugin_settings_store(config_directory); // 统一设置文件。
    match load_plugin_settings_store(&path)
    {
        Ok(None) => Ok(None),
        Ok(Some(map)) => Ok(map.get(&key).cloned()),
        Err(error) if error.contains("已损坏") => Ok(None),
        Err(error) => Err(error),
    }
}

/// 标识必须带 plugin. 前缀，且只能对应一个 plugin.<名称>.js 文件名。
fn plugin_script_name(plugin_id: &str) -> Result<String, String>
{
    let relative = format!("{plugin_id}.js"); // 插件目录内的相对路径。
    let path = Path::new(&relative); // 不允许把标识拆成目录。
    let single_file = path.components().count() == 1 && path.file_name().and_then(|value| value.to_str()) == Some(relative.as_str());
    if !plugin_id.starts_with("plugin.") || plugin_id.len() == "plugin.".len() || !single_file || relative.strip_suffix(".js") != Some(plugin_id) || relative.contains([':', '%', '\\', '\0'])
    {
        return Err("插件标识无效".to_string());
    }
    Ok(relative)
}

/// 只校验卸载目标，不删除。工作区清理插在校验和删除之间。
pub(crate) fn plan_plugin_removal(plugin_directory: &Path, config_directory: &Path, plugin_id: &str, remove_settings: bool, author: &str) -> Result<Vec<(String, PathBuf)>, String>
{
    plugin_script_name(plugin_id).inspect_err(|_| crate::log_warn!("插件卸载：拒绝非法插件标识，插件={plugin_id}"))?; // 仍只接受 plugin. 前缀。
    let mut pending = Vec::new(); // 全部通过后才允许删除。
    if let Some(folder) = plugin_folder(plugin_directory, plugin_id).inspect_err(|error| crate::log_warn!("插件卸载：拒绝删除，插件={plugin_id}，原因={error}"))?
    {
        let name = folder.file_name().and_then(|value| value.to_str()).unwrap_or(plugin_id).to_string(); // 实际删除的文件夹名。
        pending.push((name, folder));
    }
    if remove_settings
    {
        plugin_settings_key(author, plugin_id).inspect_err(|_| crate::log_warn!("插件卸载：拒绝非法插件标识，插件={plugin_id}"))?; // 与读写设置同一条键校验。
        reject_plugin_settings_link(config_directory).inspect_err(|error| crate::log_warn!("插件卸载：拒绝删除设置，插件={plugin_id}，原因={error}"))?; // 链接在删文件夹之前拒绝。
    }
    Ok(pending)
}

/// 删除已经校验过的卸载目标。中途失败时已删文件不会恢复，再次调用会补删剩余文件。
pub(crate) fn commit_plugin_removal(pending: Vec<(String, PathBuf)>) -> Result<Vec<String>, String>
{
    let mut removed = Vec::new(); // 实际删除的文件名。
    for (name, path) in pending
    {
        let delete = if path.is_dir() { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) }; // 插件是整个文件夹，设置仍是单个文件。
        match delete
        {
            Ok(()) => removed.push(name),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound =>
            {}
            Err(error) => return Err(format!("删除插件文件失败：{error}")),
        }
    }
    Ok(removed)
}

/// 删除清单与脚本；remove_settings 为真时再从统一设置文件删掉对应键。键删空后删除文件。
#[cfg(test)]
pub(crate) fn uninstall_plugin_files(plugin_directory: &Path, config_directory: &Path, plugin_id: &str, remove_settings: bool, author: &str) -> Result<Vec<String>, String>
{
    let mut removed = commit_plugin_removal(plan_plugin_removal(plugin_directory, config_directory, plugin_id, remove_settings, author)?)?; // 设置键在文件夹删除之后再改。
    if remove_settings && remove_plugin_settings(config_directory, author, plugin_id)?
    {
        removed.push(PLUGIN_SETTINGS_FILE.to_string());
    }
    Ok(removed)
}

/// 返回插件根内已校验的清单路径；文件不存在时不打开目录代替。
pub(crate) fn plugin_manifest_file(directory: &Path, plugin_id: &str) -> Result<PathBuf, String>
{
    plugin_script_name(plugin_id).inspect_err(|_| crate::log_warn!("插件目录：拒绝非法插件标识，插件={plugin_id}"))?;
    let folder = plugin_folder(directory, plugin_id).inspect_err(|error| crate::log_warn!("插件目录：清单不可用，插件={plugin_id}，原因={error}"))?.ok_or_else(|| {
        crate::log_warn!("插件目录：清单不存在，插件={plugin_id}");
        "插件清单不存在".to_string()
    })?; // 要在资源管理器中选中的是文件夹里的 manifest.ini。
    std::fs::canonicalize(folder.join("manifest.ini")).map_err(|error| format!("读取插件清单失败：{error}"))
}

/// 找到该插件在插件根下的第一层文件夹。链接和重解析点不能当插件包删除。
fn plugin_folder(directory: &Path, plugin_id: &str) -> Result<Option<PathBuf>, String>
{
    use std::os::windows::fs::MetadataExt;
    let Some(folder) = crate::component_runtime::find_plugin_folder(directory, plugin_id)?
    else
    {
        return Ok(None);
    };
    if folder.parent() != Some(directory)
    {
        return Err("插件文件夹不在插件根下".to_string()); // 只删除插件根的第一层，绝不退到插件根自己。
    }
    let metadata = std::fs::symlink_metadata(&folder).map_err(|error| format!("读取插件文件夹失败：{error}"))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() || metadata.file_attributes() & 0x400 != 0
    {
        return Err("插件文件夹不能是链接或重解析点".to_string());
    }
    Ok(Some(folder))
}

/// 原子保存一个插件的设置；标识非法或文件已损坏时不创建、不覆盖。
pub(crate) fn write_plugin_settings(config_directory: &Path, author: &str, plugin_id: &str, value: &serde_json::Value) -> Result<(), String>
{
    let key = plugin_settings_key(author, plugin_id).inspect_err(|_| crate::log_warn!("插件设置：拒绝非法插件标识，插件={plugin_id}"))?; // 作者@插件ID。
    reject_plugin_settings_link(config_directory)?;
    let path = plugin_settings_store(config_directory); // 统一设置文件。
    let mut map = match load_plugin_settings_store(&path)?
    {
        None => serde_json::Map::new(),
        Some(map) => map,
    }; // 损坏的文件在加载时已经返回错误，这里不会覆盖。
    map.insert(key, value.clone());
    save_json_atomic(&path, &serde_json::Value::Object(map)).map_err(|error| format!("保存插件设置失败：{error}"))
}

/// 从统一设置文件删除一个键。文件因此变空时删除文件，并返回真。
pub(crate) fn remove_plugin_settings(config_directory: &Path, author: &str, plugin_id: &str) -> Result<bool, String>
{
    let key = plugin_settings_key(author, plugin_id)?; // 作者@插件ID。
    reject_plugin_settings_link(config_directory)?;
    let path = plugin_settings_store(config_directory); // 统一设置文件。
    let Some(mut map) = load_plugin_settings_store(&path)?
    else
    {
        return Ok(false);
    }; // 文件不存在就没有键可删。
    map.remove(&key);
    if map.is_empty()
    {
        match std::fs::remove_file(&path)
        {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
            Err(error) => Err(format!("删除插件设置失败：{error}")),
        }
    }
    else
    {
        save_json_atomic(&path, &serde_json::Value::Object(map)).map_err(|error| format!("保存插件设置失败：{error}"))?;
        Ok(false)
    }
}

const ZIP_FILE_LIMIT: u64 = 64 * 1024 * 1024; // 单个压缩条目的解压上限。
const ZIP_TOTAL_LIMIT: u64 = 256 * 1024 * 1024; // 整包解压上限。
const ZIP_ENTRY_LIMIT: usize = 2000; // 条目数上限。

/// 目标里已经装着的版本。降级为真时必须等调用方确认。
#[derive(Clone, Debug)]
pub(crate) struct PluginInstallConflict
{
    pub installed_version: String, // 已安装版本。
    pub incoming_version: String,  // 待安装版本。
    pub downgrade: bool,           // 待安装版本更低。
    pub installed_folder: String,  // 已安装那份的文件夹名，覆盖时删掉。
}

/// 校验通过、尚未落盘的安装来源。
#[derive(Clone, Debug)]
pub(crate) struct PreparedPlugin
{
    pub id: String,                              // 清单声明的标识，安装时原样保留。
    pub name: String,                            // 清单名称；目标文件夹是 作者@名称。
    pub author: String,                          // 待安装清单里的作者，没有就是空。
    pub version: String,                         // 待安装版本。
    pub source_name: String,                     // 进度里显示的来源名。
    pub package_dir: PathBuf,                    // 文件夹来源中含 manifest.ini 的目录。
    pub archive: Option<(PathBuf, String)>,      // zip 路径，以及包内前缀（空或 `文件夹/`）。
    pub conflict: Option<PluginInstallConflict>, // 同一作者同一标识已装过时才有。
}

/// 阶段变化、间隔至少 100 毫秒，或进度至少前进 2%，才发送一次。
pub(crate) fn install_progress_due(stage_changed: bool, elapsed_ms: u128, percent_delta: u8) -> bool { stage_changed || elapsed_ms >= 100 || percent_delta >= 2 }

/// 检查文件夹来源。清单可以在根上，也可以只在下一层。
pub(crate) fn inspect_plugin_folder(source: &Path, plugin_root: &Path) -> Result<PreparedPlugin, String>
{
    let package_dir = locate_plugin_folder(source)?; // 含 manifest.ini 的那一层。
    let values = read_plugin_ini(&package_dir.join("manifest.ini"))?;
    let prepared = validate_plugin_source(&values, plugin_root, source.file_name().and_then(|name| name.to_str()).unwrap_or("插件"))?;
    confirm_entry_file(&package_dir, &values["entry"])?;
    Ok(PreparedPlugin { package_dir, archive: None, ..prepared })
}

/// 只读检查 zip。清单可以在包根，也可以只在下一层。
pub(crate) fn inspect_plugin_archive(archive_path: &Path, plugin_root: &Path) -> Result<PreparedPlugin, String>
{
    let file = std::fs::File::open(archive_path).map_err(|error| format!("读取插件压缩包失败：{error}"))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|error| format!("插件压缩包无效：{error}"))?;
    let (entries, manifests) = zip_listing(&mut archive)?; // 先看目录，不写出文件。
    let (prefix, manifest) = zip_manifest(&manifests)?;
    let values = parse_plugin_ini(&manifest)?;
    let source_name = archive_path.file_name().and_then(|name| name.to_str()).unwrap_or("插件");
    let prepared = validate_plugin_source(&values, plugin_root, source_name)?;
    let entry = prefix.clone() + &values["entry"].replace('\\', "/");
    if !entries.iter().any(|item| item.name == entry && !item.directory)
    {
        crate::log_warn!("插件安装：压缩包里没有入口脚本，仍继续安装，入口={}", values["entry"]);
    }
    Ok(PreparedPlugin { package_dir: PathBuf::new(), archive: Some((archive_path.to_path_buf(), prefix)), ..prepared })
}

/// 先写到插件根下的临时目录，成功后再改名为目标文件夹。失败和取消都会删掉临时目录。
pub(crate) fn install_prepared_plugin(plugin_root: &Path, prepared: &PreparedPlugin, confirm_downgrade: bool, cancel: &std::sync::atomic::AtomicBool, mut uninstall: impl FnMut(&str) -> Result<(), String>, mut progress: impl FnMut(&str, u8)) -> Result<String, String>
{
    use std::sync::atomic::Ordering;
    if prepared.conflict.as_ref().is_some_and(|conflict| conflict.downgrade) && !confirm_downgrade
    {
        return Err("降级安装需要确认".to_string());
    }
    progress("validate", 5);
    if cancel.load(Ordering::Relaxed)
    {
        return Err("安装已取消".to_string());
    }
    std::fs::create_dir_all(plugin_root).map_err(|error| format!("安装插件失败：{error}"))?;
    let temp = plugin_root.join(format!(".install-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|value| value.as_nanos()).unwrap_or(0))); // 成功前的临时目录。
    let install = (|| {
        if let Some((archive_path, prefix)) = &prepared.archive
        {
            extract_archive(archive_path, prefix, &temp, cancel, &mut progress)?;
        }
        else
        {
            copy_package(&prepared.package_dir, &temp, cancel, &mut progress)?;
        }
        if cancel.load(Ordering::Relaxed)
        {
            return Err("安装已取消".to_string());
        }
        let target_name = crate::component_runtime::plugin_folder_name(&prepared.author, &prepared.name); // 作者@名称。
        let target = plugin_root.join(&target_name); // 最终文件夹。
        if let Some(conflict) = &prepared.conflict
        {
            if conflict.downgrade
            {
                uninstall(&prepared.id)?;
            }
            if conflict.installed_folder != target_name
            {
                let replaced = plugin_root.join(&conflict.installed_folder); // 同一插件改过名称时先删旧文件夹。
                if replaced.exists()
                {
                    std::fs::remove_dir_all(&replaced).map_err(|error| format!("安装插件失败：{error}"))?;
                }
            }
        }
        else if target.exists()
        {
            return Err(format!("插件目录已有同名文件夹：{target_name}"));
        }
        if target.exists()
        {
            std::fs::remove_dir_all(&target).map_err(|error| format!("安装插件失败：{error}"))?;
        }
        progress("copy", 98);
        std::fs::rename(&temp, &target).map_err(|error| format!("安装插件失败：{error}"))?;
        progress("done", 100);
        Ok(prepared.id.clone())
    })();
    if install.is_err()
    {
        let _ = std::fs::remove_dir_all(&temp); // 半包不留下。
    }
    install
}

/// 压缩条目的路径和体积边界。链接条目直接拒绝。
pub(crate) fn reject_zip_entry(name: &str, unix_mode: Option<u32>, size: u64) -> Result<(), String>
{
    use std::path::Component;
    if size > ZIP_FILE_LIMIT
    {
        return Err("压缩包内单个文件超过 64 MiB".to_string());
    }
    if unix_mode.is_some_and(|mode| mode & 0o170000 == 0o120000)
    {
        return Err("压缩包包含链接".to_string());
    }
    let path = Path::new(name);
    if name.starts_with(['/', '\\']) || path.is_absolute() || name.contains(':') || path.components().any(|part| matches!(part, Component::ParentDir | Component::Prefix(_)))
    {
        return Err(format!("压缩包包含非法路径：{name}"));
    }
    Ok(())
}

/// 根上有清单就用根，否则只接受下一层里唯一的一份。
fn locate_plugin_folder(source: &Path) -> Result<PathBuf, String>
{
    if source.join("manifest.ini").is_file()
    {
        return Ok(source.to_path_buf());
    }
    let mut found = Vec::new(); // 下一层里带清单的文件夹。
    for entry in std::fs::read_dir(source).map_err(|error| format!("读取插件来源失败：{error}"))?
    {
        let path = entry.map_err(|error| format!("读取插件来源失败：{error}"))?.path();
        if path.is_dir() && path.join("manifest.ini").is_file()
        {
            found.push(path);
        }
    }
    match found.len()
    {
        1 => Ok(found.remove(0)),
        0 => Err("未找到插件清单".to_string()),
        _ => Err("找到多份插件清单".to_string()),
    }
}

fn validate_plugin_source(values: &std::collections::HashMap<String, String>, plugin_root: &Path, source_name: &str) -> Result<PreparedPlugin, String>
{
    let name = values.get("name").map(|value| value.trim()).filter(|value| !value.is_empty()).ok_or_else(|| "插件清单缺少 name".to_string())?.to_string();
    if name.chars().count() > 80 || name.chars().any(|character| character.is_control() || matches!(character, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
    {
        return Err(format!("插件名称无效：{name}"));
    }
    let version = values.get("version").map(|value| value.trim()).filter(|value| !value.is_empty()).ok_or_else(|| "插件清单缺少 version".to_string())?;
    let incoming = parse_version(version).ok_or_else(|| "插件版本无效".to_string())?;
    let entry = values.get("entry").map(|value| value.trim()).filter(|value| !value.is_empty()).ok_or_else(|| "插件清单缺少 entry".to_string())?;
    if Path::new(entry).is_absolute() || entry.contains('\\') || Path::new(entry).components().any(|part| !matches!(part, std::path::Component::Normal(_))) || !entry.ends_with(".js")
    {
        return Err(format!("插件入口脚本超出插件目录：{entry}"));
    }
    let app_version = parse_version(env!("CARGO_PKG_VERSION")).ok_or_else(|| "程序版本无效".to_string())?;
    if let Some(minimum) = values.get("minAppVersion").map(|value| value.trim()).filter(|value| !value.is_empty())
    {
        if app_version < parse_version(minimum).ok_or_else(|| "插件要求的程序版本无效".to_string())?
        {
            return Err(format!("插件要求的程序版本过高，需要 {minimum}，当前 {}", env!("CARGO_PKG_VERSION")));
        }
    }
    if let Some(maximum) = values.get("maxAppVersion").map(|value| value.trim()).filter(|value| !value.is_empty())
    {
        if app_version > parse_version(maximum).ok_or_else(|| "插件要求的程序版本无效".to_string())?
        {
            return Err(format!("插件不支持当前程序版本，最高 {maximum}，当前 {}", env!("CARGO_PKG_VERSION")));
        }
    }
    let id = values.get("id").map(|value| value.trim()).filter(|value| !value.is_empty()).ok_or_else(|| "插件清单缺少 id".to_string())?.to_string(); // 内容不校验，也不改写。
    let author = values.get("author").map(|value| value.trim().to_string()).unwrap_or_default(); // 覆盖弹窗要显示作者，也参与同一插件判定。
    let conflict = installed_conflict(plugin_root, &author, &id, version, incoming)?;
    Ok(PreparedPlugin { id, name, author, version: version.to_string(), source_name: source_name.to_string(), package_dir: PathBuf::new(), archive: None, conflict })
}

/// 同一作者的同一标识就是同一个插件，与文件夹名无关。
fn installed_conflict(plugin_root: &Path, author: &str, plugin_id: &str, version: &str, incoming: (u64, u64, u64)) -> Result<Option<PluginInstallConflict>, String>
{
    let Ok(entries) = std::fs::read_dir(plugin_root)
    else
    {
        return Ok(None); // 插件根还不存在，视为新安装。
    };
    for entry in entries.flatten()
    {
        let folder = entry.path();
        let folder_name = folder.file_name().and_then(|value| value.to_str()).unwrap_or("").to_string();
        if folder_name.starts_with('.')
        {
            continue; // 安装临时目录不算已装。
        }
        let Ok(values) = read_plugin_ini(&folder.join("manifest.ini"))
        else
        {
            continue; // 没有清单或清单损坏的文件夹不参与冲突判定。
        };
        let same_author = values.get("author").map(|value| value.trim()).unwrap_or("") == author; // 作者缺省为空。
        let same_id = values.get("id").map(|value| value.trim()).unwrap_or("") == plugin_id;
        if !same_author || !same_id
        {
            continue;
        }
        let installed = values.get("version").map(|value| value.trim()).filter(|value| !value.is_empty()).ok_or_else(|| "已安装插件版本无效".to_string())?;
        let installed_version = parse_version(installed).ok_or_else(|| "已安装插件版本无效".to_string())?;
        return Ok(Some(PluginInstallConflict { installed_version: installed.to_string(), incoming_version: version.to_string(), downgrade: incoming < installed_version, installed_folder: folder_name }));
    }
    Ok(None)
}

fn confirm_entry_file(package_dir: &Path, entry: &str) -> Result<(), String>
{
    use std::os::windows::fs::MetadataExt;
    let path = package_dir.join(entry); // 入口必须是包内普通文件。路径形状已在清单校验里拒绝。
    let metadata = match std::fs::symlink_metadata(&path)
    {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound =>
        {
            crate::log_warn!("插件安装：入口脚本不存在，仍继续安装，入口={entry}");
            return Ok(());
        }
        Err(error) => return Err(format!("读取插件入口脚本失败：{error}")),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.file_attributes() & 0x400 != 0
    {
        return Err("插件入口脚本不能是链接或重解析点".to_string());
    }
    Ok(())
}

fn parse_version(value: &str) -> Option<(u64, u64, u64)>
{
    let mut parts = value.split('.');
    let version = (parts.next()?.parse().ok()?, parts.next()?.parse().ok()?, parts.next()?.parse().ok()?);
    parts.next().is_none().then_some(version)
}

fn read_plugin_ini(path: &Path) -> Result<std::collections::HashMap<String, String>, String>
{
    let text = std::fs::read_to_string(path).map_err(|error| format!("读取插件清单失败：{error}"))?;
    parse_plugin_ini(&text)
}

fn parse_plugin_ini(text: &str) -> Result<std::collections::HashMap<String, String>, String>
{
    let mut section = false; // 当前行是否属于 [plugin]。
    let mut values = std::collections::HashMap::new(); // 该节的键。作者写的 id 不参与校验。
    for line in text.trim_start_matches('\u{feff}').lines()
    {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';')
        {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|value| value.strip_suffix(']'))
        {
            section = name.trim().eq_ignore_ascii_case("plugin");
            continue;
        }
        if !section
        {
            continue;
        }
        let Some((key, value)) = line.split_once('=')
        else
        {
            return Err("插件清单格式无效".to_string());
        };
        values.insert(key.trim().to_string(), value.trim().to_string());
    }
    if values.is_empty()
    {
        return Err("插件清单缺少 [plugin]".to_string());
    }
    Ok(values)
}

fn copy_package(source: &Path, temp: &Path, cancel: &std::sync::atomic::AtomicBool, progress: &mut impl FnMut(&str, u8)) -> Result<(), String>
{
    use std::sync::atomic::Ordering;
    let files = list_package_files(source)?; // 复制前先数完，进度才有分母。
    std::fs::create_dir_all(temp).map_err(|error| format!("安装插件失败：{error}"))?;
    for (index, relative) in files.iter().enumerate()
    {
        if cancel.load(Ordering::Relaxed)
        {
            return Err("安装已取消".to_string());
        }
        let destination = temp.join(relative); // 临时目录里的对应文件。
        if let Some(parent) = destination.parent()
        {
            std::fs::create_dir_all(parent).map_err(|error| format!("安装插件失败：{error}"))?;
        }
        copy_regular_file(&source.join(relative), &destination)?;
        progress("copy", (index as u32 * 90 / files.len().max(1) as u32) as u8);
    }
    Ok(())
}

fn list_package_files(source: &Path) -> Result<Vec<PathBuf>, String>
{
    let mut files = Vec::new(); // 包内相对路径。
    fn walk(directory: &Path, prefix: &Path, files: &mut Vec<PathBuf>) -> Result<(), String>
    {
        for entry in std::fs::read_dir(directory).map_err(|error| format!("读取插件来源失败：{error}"))?
        {
            let path = entry.map_err(|error| format!("读取插件来源失败：{error}"))?.path();
            let relative = prefix.join(path.file_name().unwrap_or_default()); // 保持包内结构。
            if path.is_dir()
            {
                walk(&path, &relative, files)?;
            }
            else
            {
                files.push(relative);
            }
        }
        Ok(())
    }
    walk(source, Path::new(""), &mut files)?;
    Ok(files)
}

fn copy_regular_file(source: &Path, destination: &Path) -> Result<(), String>
{
    use std::os::windows::fs::MetadataExt;
    let metadata = std::fs::symlink_metadata(source).map_err(|error| format!("安装插件失败：{error}"))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.file_attributes() & 0x400 != 0
    {
        return Err("插件文件不能是链接或重解析点".to_string());
    }
    std::fs::copy(source, destination).map_err(|error| format!("安装插件失败：{error}"))?;
    Ok(())
}

struct ZipEntry
{
    name: String,    // 包内正斜杠路径。
    directory: bool, // 目录条目本身不写出内容。
}

fn zip_listing(archive: &mut zip::ZipArchive<std::fs::File>) -> Result<(Vec<ZipEntry>, Vec<(String, String)>), String>
{
    use std::io::Read;
    if archive.len() > ZIP_ENTRY_LIMIT
    {
        return Err("压缩包条目超过 2000".to_string());
    }
    let mut entries = Vec::new(); // 只读目录。
    let mut manifests = Vec::new(); // 一层以内的清单正文。
    let mut total = 0u64; // 声明体积合计。
    for index in 0..archive.len()
    {
        let mut file = archive.by_index(index).map_err(|error| format!("插件压缩包无效：{error}"))?;
        let name = file.name().replace('\\', "/");
        reject_zip_entry(&name, file.unix_mode(), file.size())?;
        total = total.saturating_add(file.size());
        if total > ZIP_TOTAL_LIMIT
        {
            return Err("压缩包总体积超过 256 MiB".to_string());
        }
        let depth = name.trim_end_matches('/').matches('/').count();
        if name == "manifest.ini" || name.ends_with("/manifest.ini") && depth == 1
        {
            if file.size() > 65536
            {
                return Err("插件清单超过 64 KiB".to_string());
            }
            let mut text = String::new(); // 校验阶段就读完清单，不解压其他文件。
            file.read_to_string(&mut text).map_err(|error| format!("读取插件清单失败：{error}"))?;
            manifests.push((name.clone(), text));
        }
        entries.push(ZipEntry { name, directory: file.is_dir() });
    }
    Ok((entries, manifests))
}

fn zip_manifest(manifests: &[(String, String)]) -> Result<(String, String), String>
{
    match manifests.len()
    {
        1 =>
        {
            let (name, text) = &manifests[0];
            Ok((name.trim_end_matches("manifest.ini").to_string(), text.clone()))
        }
        0 => Err("未找到插件清单".to_string()),
        _ => Err("找到多份插件清单".to_string()),
    }
}

fn extract_archive(archive_path: &Path, prefix: &str, temp: &Path, cancel: &std::sync::atomic::AtomicBool, progress: &mut impl FnMut(&str, u8)) -> Result<(), String>
{
    use std::{io::copy, sync::atomic::Ordering};
    let file = std::fs::File::open(archive_path).map_err(|error| format!("读取插件压缩包失败：{error}"))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|error| format!("插件压缩包无效：{error}"))?;
    let count = archive.len().max(1);
    std::fs::create_dir_all(temp).map_err(|error| format!("安装插件失败：{error}"))?;
    for index in 0..archive.len()
    {
        if cancel.load(Ordering::Relaxed)
        {
            return Err("安装已取消".to_string());
        }
        let mut entry = archive.by_index(index).map_err(|error| format!("插件压缩包无效：{error}"))?;
        let name = entry.name().replace('\\', "/");
        reject_zip_entry(&name, entry.unix_mode(), entry.size())?;
        if !prefix.is_empty() && !name.starts_with(prefix) || entry.is_dir()
        {
            progress("extract", (index as u32 * 90 / count as u32) as u8);
            continue;
        }
        let relative = name.trim_start_matches(prefix); // 去掉包内那一层文件夹。
        let destination = temp.join(relative);
        if let Some(parent) = destination.parent()
        {
            std::fs::create_dir_all(parent).map_err(|error| format!("安装插件失败：{error}"))?;
        }
        let mut output = std::fs::File::create(&destination).map_err(|error| format!("安装插件失败：{error}"))?;
        copy(&mut entry, &mut output).map_err(|error| format!("安装插件失败：{error}"))?;
        progress("extract", (index as u32 * 90 / count as u32) as u8);
    }
    Ok(())
}

#[cfg(test)]
#[path = "settings.test.rs"]
mod tests;
