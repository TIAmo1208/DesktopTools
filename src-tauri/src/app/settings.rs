use std::{path::PathBuf, sync::Mutex};

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
    if !path.is_absolute() || expanded.contains(['%', '\0', '*', '?', '"', '<', '>', '|']) || expanded.starts_with(r"\\.\") || expanded.starts_with(r"\\?\") || path.components().any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return Err("配置目录必须为绝对路径或以 %APPDATA% 开头，不能包含父级跳转或特殊字符".to_string());
    }
    Ok(path)
}

/// 在保存偏好或关联布局前完成格式与实际路径校验。
pub(crate) fn validate_preferences(preferences: &ApplicationPreferences) -> Result<PathBuf, String>
{
    preferences.validate()?;
    resolve_config_directory(&preferences.config_directory)
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
            crate::log_info!("设置：配置目录已迁移，原目录={}，当前目录={}，原文件保留", source.display(), target.display());
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
    // 只复制本应用拥有的数据；显式队列遍历备份目录，不跟随链接或重解析点。
    let mut pending = vec![(source.join("settings.json"), target.join("settings.json")), (source.join("workspace-layout.json"), target.join("workspace-layout.json")), (source.join("workspace-layout.snapshots"), target.join("workspace-layout.snapshots"))]; // 等待复制的固定数据入口及备份子项。
    while let Some((from, to)) = pending.pop()
    {
        use std::os::windows::fs::MetadataExt;
        let metadata = match fs::symlink_metadata(&from)
        {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        }; // 缺失数据允许首次启动，其他读取错误不得忽略。
        if metadata.file_attributes() & 0x400 != 0
        {
            return Err(io::Error::other("配置迁移不支持链接或重解析点"));
        }
        if metadata.is_dir()
        {
            fs::create_dir(&to)?;
            for entry in fs::read_dir(&from)?
            {
                let entry = entry?; // 当前备份子项。
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

    /// 保存成功后替换内存状态，失败时保留原值。
    pub fn save(&self, preferences: ApplicationPreferences) -> Result<ApplicationPreferences, String>
    {
        validate_preferences(&preferences)?;
        let mut current = self.preferences.lock().map_err(|_| "设置状态不可用".to_string())?; // 当前串行提交锁。
        save_json_atomic(&self.path, &preferences).map_err(|error| format!("保存设置失败：{error}"))?;
        *current = preferences;
        crate::log_info!("设置：用户偏好已保存");
        Ok(current.clone())
    }
}

#[cfg(test)]
#[path = "settings.test.rs"]
mod tests;
