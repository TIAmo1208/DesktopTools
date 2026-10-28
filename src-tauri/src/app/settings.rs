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
    if !path.is_absolute() || expanded.contains(['%', '\0', '*', '?', '"', '<', '>', '|']) || expanded.starts_with(r"\\.\") || expanded.starts_with(r"\\?\") || path.components().any(|part| matches!(part, std::path::Component::ParentDir) || matches!(part, std::path::Component::Normal(value) if value.to_string_lossy().contains(':')))
    {
        return Err("配置目录必须为绝对路径或以 %APPDATA% 开头，不能包含父级跳转或特殊字符".to_string());
    }
    Ok(path)
}

/// 空偏好使用当前生效配置目录的同级 plugin，显式路径沿用配置目录安全边界。
pub fn resolve_plugin_directory(value: &str, config_directory: &std::path::Path) -> Result<PathBuf, String>
{
    if value.trim().is_empty()
    {
        return config_directory.parent().map(|parent| parent.join("plugin")).ok_or_else(|| "配置目录没有可用的父目录".to_string());
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
    let pointer = bootstrap.join("config-location.json"); // 不随配置目录迁移的启动定位文件。
    let source: PathBuf = load_json(&pointer)?.unwrap_or_else(|| bootstrap.to_path_buf()); // 兼容旧版固定数据目录。
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

/// 复制数据并最后提交启动位置；拒绝非空目标，失败不删除或覆盖原数据。
fn migrate_config_directory(source: &std::path::Path, target: &std::path::Path, pointer: &std::path::Path) -> std::io::Result<()>
{
    use std::{fs, io};
    fs::create_dir_all(source)?;
    fs::create_dir_all(target)?;
    let source = fs::canonicalize(source)?; // 消除大小写、链接和路径别名差异。
    let target = fs::canonicalize(target)?; // 创建后验证最终真实位置。
    if source == target
    {
        return Ok(());
    }
    if source.starts_with(&target) || target.starts_with(&source) || fs::read_dir(&target)?.next().is_some()
    {
        return Err(io::Error::other("配置目录不能互相包含，且目标目录必须为空"));
    }
    // 整目录复制，不再按固定名单挑选；不跟随链接或重解析点，插件设置因此一起离开。
    let mut pending = vec![(source.clone(), target.clone())]; // 待复制的目录或文件，根目录本身已经存在。
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
            return Err(io::Error::other("配置迁移不支持链接或重解析点"));
        }
        if metadata.is_dir()
        {
            if from != source
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
    // 复制完成后才切换指针；中途失败留下的目标副本不自动清除或覆盖。
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
        save_json_atomic(&self.path, &preferences).map_err(|error| format!("保存设置失败：{error}"))?;
        *current = preferences;
        crate::log_info!("设置：用户偏好已保存");
        Ok(current.clone())
    }
}

/// 由清单 id 得到插件设置文件名；非法或无法往返的 id 不产生路径。
pub(crate) fn plugin_settings_file_name(plugin_id: &str) -> Result<String, String>
{
    let Some(name) = plugin_id.strip_prefix("plugin.")
    else
    {
        return Err("插件标识无效".to_string());
    }; // 只去掉一次 plugin. 前缀，官方 id 不能落到插件设置文件。
    if name.is_empty() || name.len() > 240 || !name.chars().all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-'))
    {
        return Err("插件标识无效".to_string());
    }
    let file_name = format!("plugins-{name}.json"); // 唯一允许写进配置目录的文件名。
    let restored = file_name.strip_prefix("plugins-").and_then(|value| value.strip_suffix(".json")).map(|value| format!("plugin.{value}")); // 文件名必须还原成原来的 id。
    let path = Path::new(&file_name); // 文件名本身不得再带目录。
    if restored.as_deref() != Some(plugin_id) || path.components().count() != 1 || path.file_name().and_then(|value| value.to_str()) != Some(file_name.as_str())
    {
        return Err("插件标识无效".to_string());
    }
    Ok(file_name)
}

/// 校验 id 后再拼路径；失败只记 WARN，不使用原始 id。
fn plugin_settings_path(config_directory: &Path, plugin_id: &str) -> Result<PathBuf, String>
{
    let file_name = plugin_settings_file_name(plugin_id).inspect_err(|_| crate::log_warn!("插件设置：拒绝非法插件标识，插件={plugin_id}"))?; // 已通过字符集与往返校验的文件名。
    Ok(config_directory.join(file_name))
}

/// 读取一份插件设置；文件缺失或 JSON 损坏都不改写磁盘。
pub(crate) fn read_plugin_settings(config_directory: &Path, plugin_id: &str) -> Result<Option<serde_json::Value>, String>
{
    let path = plugin_settings_path(config_directory, plugin_id)?; // 当前插件的设置文件。
    match std::fs::read(&path)
    {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("读取插件设置失败：{error}")),
        Ok(bytes) => match serde_json::from_slice(&bytes)
        {
            Ok(value) => Ok(Some(value)),
            Err(error) =>
            {
                crate::log_warn!("插件设置：内容不是合法 JSON，已保留原文件，路径={}，错误={error}", path.display());
                Ok(None)
            }
        },
    }
}

/// 读取插件根目录内的脚本；规则与桌面宿主的 read_plugin_asset 对齐，但只接受 .js。
pub(crate) fn read_plugin_script(directory: &Path, plugin_id: &str) -> Result<String, String>
{
    let relative = plugin_script_name(plugin_id).inspect_err(|_| crate::log_warn!("插件脚本：拒绝非法插件标识，插件={plugin_id}"))?; // 与标识往返一致的文件名。
    read_plugin_script_file(directory, &relative).inspect_err(|error| crate::log_warn!("插件脚本：读取失败，插件={plugin_id}，原因={error}"))
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

/// 只读取插件根内的普通 .js 文件，拒绝穿越、链接和重解析点。
pub(crate) fn read_plugin_script_file(directory: &Path, relative: &str) -> Result<String, String>
{
    use std::os::windows::fs::MetadataExt;
    use std::path::Component;
    let relative_path = Path::new(relative); // 不解码百分号，避免编码后的分隔符改变文件身份。
    if relative.is_empty() || relative.contains([':', '%', '\\', '\0']) || relative_path.components().any(|part| !matches!(part, Component::Normal(_))) || relative_path.extension().and_then(|value| value.to_str()) != Some("js")
    {
        return Err("插件脚本路径无效".to_string());
    }
    let candidate = directory.join(relative_path); // 尚未跟随链接的请求目标。
    for ancestor in candidate.ancestors().take_while(|ancestor| ancestor.starts_with(directory))
    {
        let metadata = std::fs::symlink_metadata(ancestor).map_err(|error| format!("读取插件脚本失败：{error}"))?; // 只检查插件根及其内部。
        if metadata.file_type().is_symlink() || metadata.file_attributes() & 0x400 != 0
        {
            return Err("插件脚本不能是链接或重解析点".to_string());
        }
    }
    let root = std::fs::canonicalize(directory).map_err(|error| format!("读取插件脚本失败：{error}"))?; // 实际插件根。
    let path = std::fs::canonicalize(&candidate).map_err(|error| format!("读取插件脚本失败：{error}"))?; // 实际文件必须仍在插件根内。
    if !path.starts_with(&root) || !std::fs::metadata(&path).map_err(|error| format!("读取插件脚本失败：{error}"))?.is_file()
    {
        return Err("插件脚本必须位于插件目录内".to_string());
    }
    let bytes = std::fs::read(&path).map_err(|error| format!("读取插件脚本失败：{error}"))?; // 交给调用窗口执行的源码。
    String::from_utf8(bytes).map_err(|_| "插件脚本不是 UTF-8 文本".to_string())
}

/// 只删除该插件的清单与脚本；缺失不算失败，设置文件不在删除范围内。
pub(crate) fn uninstall_plugin_files(directory: &Path, plugin_id: &str) -> Result<Vec<String>, String>
{
    let script = plugin_script_name(plugin_id).inspect_err(|_| crate::log_warn!("插件卸载：拒绝非法插件标识，插件={plugin_id}"))?; // 与脚本读取同一条标识规则。
    let manifest = format!("{plugin_id}.json"); // 与脚本同名的清单，不是 plugins- 设置文件。
    let mut pending = Vec::new(); // 校验通过后才删除，避免先删掉一半。
    for name in [manifest, script]
    {
        if let Some(path) = confirmed_plugin_file(directory, &name).inspect_err(|error| crate::log_warn!("插件卸载：拒绝删除，插件={plugin_id}，文件={name}，原因={error}"))?
        {
            pending.push((name, path));
        }
    }
    let mut removed = Vec::new(); // 实际删除的文件名。
    for (name, path) in pending
    {
        std::fs::remove_file(&path).map_err(|error| format!("删除插件文件失败：{error}"))?;
        removed.push(name);
    }
    Ok(removed)
}

/// 返回插件根内已校验的清单路径；文件不存在时不打开目录代替。
pub(crate) fn plugin_manifest_file(directory: &Path, plugin_id: &str) -> Result<PathBuf, String>
{
    plugin_script_name(plugin_id).inspect_err(|_| crate::log_warn!("插件目录：拒绝非法插件标识，插件={plugin_id}"))?;
    let manifest = format!("{plugin_id}.json"); // 要在资源管理器中选中的清单。
    confirmed_plugin_file(directory, &manifest).inspect_err(|error| crate::log_warn!("插件目录：清单不可用，插件={plugin_id}，原因={error}"))?.ok_or_else(|| {
        crate::log_warn!("插件目录：清单不存在，插件={plugin_id}");
        "插件清单不存在".to_string()
    })
}

/// 确认插件根内的普通清单或脚本；不存在返回空，链接和穿越直接拒绝。
fn confirmed_plugin_file(directory: &Path, relative: &str) -> Result<Option<PathBuf>, String>
{
    use std::os::windows::fs::MetadataExt;
    use std::path::Component;
    let relative_path = Path::new(relative); // 不解码百分号，避免编码后的分隔符改变文件身份。
    let extension = relative_path.extension().and_then(|value| value.to_str()); // 只允许清单和脚本两个名字。
    if relative.is_empty() || relative.contains([':', '%', '\\', '\0']) || relative_path.components().any(|part| !matches!(part, Component::Normal(_))) || !matches!(extension, Some("js") | Some("json"))
    {
        return Err("插件文件路径无效".to_string());
    }
    let candidate = directory.join(relative_path); // 尚未跟随链接的目标。
    for ancestor in candidate.ancestors().take_while(|ancestor| ancestor.starts_with(directory))
    {
        let metadata = match std::fs::symlink_metadata(ancestor)
        {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && ancestor == candidate => return Ok(None),
            Err(error) => return Err(format!("读取插件文件失败：{error}")),
        }; // 缺失的目标可以跳过，父级缺失和其他错误不能当成可删。
        if metadata.file_type().is_symlink() || metadata.file_attributes() & 0x400 != 0
        {
            return Err("插件文件不能是链接或重解析点".to_string());
        }
    }
    let root = std::fs::canonicalize(directory).map_err(|error| format!("读取插件文件失败：{error}"))?; // 实际插件根。
    let path = std::fs::canonicalize(&candidate).map_err(|error| format!("读取插件文件失败：{error}"))?; // 实际文件必须仍在插件根内。
    if !path.starts_with(&root) || !std::fs::metadata(&path).map_err(|error| format!("读取插件文件失败：{error}"))?.is_file()
    {
        return Err("插件文件必须位于插件目录内".to_string());
    }
    Ok(Some(path))
}

/// 原子保存一份插件设置；标识非法时不创建文件。
pub(crate) fn write_plugin_settings(config_directory: &Path, plugin_id: &str, value: &serde_json::Value) -> Result<(), String>
{
    let path = plugin_settings_path(config_directory, plugin_id)?; // 当前插件的设置文件。
    save_json_atomic(&path, value).map_err(|error| format!("保存插件设置失败：{error}"))
}

#[cfg(test)]
#[path = "settings.test.rs"]
mod tests;
