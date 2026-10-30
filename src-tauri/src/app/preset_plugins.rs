//! 安装器调用的预设插件交付命令：写入当前生效插件目录，卸载时按清单标识移除。
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

/// 安装预设插件：不存在则复制，同版本保留本地改动，不同版本整目录替换。
pub fn install_preset_plugins(source: &Path, bootstrap: &Path) -> Result<(), String>
{
    absolute_directory(source.to_path_buf(), "预设插件来源目录")?;
    absolute_directory(bootstrap.to_path_buf(), "启动定位目录")?;
    if !source.is_dir()
    {
        return Err("预设插件来源目录不存在".to_string());
    }
    let root = crate::app::settings::locate_plugin_directory(bootstrap)?; // 与程序启动时一致的目标位置，安装阶段只读解析。
    std::fs::create_dir_all(&root).map_err(|error| format!("创建插件目录失败：{error}"))?;
    update_presets(source, &root)
}

/// 卸载预设插件：只删除清单标识与来源某个预设一致的文件夹，用户自有插件保持原样。
pub fn uninstall_preset_plugins(source: &Path, bootstrap: &Path) -> Result<(), String>
{
    absolute_directory(source.to_path_buf(), "预设插件来源目录")?;
    absolute_directory(bootstrap.to_path_buf(), "启动定位目录")?;
    let ids = preset_ids(source)?; // 来源缺失时为空集合，卸载安全返回。
    if ids.is_empty()
    {
        crate::log_info!("预设插件：来源中没有可识别的预设插件，跳过卸载，来源={}", source.display());
        return Ok(());
    }
    let root = crate::app::settings::locate_plugin_directory(bootstrap)?; // 与程序启动时一致的目标位置，安装阶段只读解析。
    remove_presets(&root, &ids)
}

/// 遍历一层预设文件夹并逐个安装，返回后插件目录内只新增或替换这些预设。
fn update_presets(source: &Path, root: &Path) -> Result<(), String>
{
    let root = std::fs::canonicalize(root).map_err(|error| format!("读取插件目录失败：{error}"))?; // 目标目录的真实位置。
    let mut installed = 0usize; // 实际复制或替换的预设数量。
    let mut skipped = 0usize; // 因版本相同而保留本地改动的预设数量。
    for (name, preset) in preset_folders(source)?
    {
        let target = root.join(&name); // 目标文件夹沿用预设文件夹名。
        ensure_child(&root, &target)?;
        if target.exists()
        {
            let metadata = std::fs::symlink_metadata(&target).map_err(|error| format!("读取插件文件夹失败：{error}"))?;
            reject_reparse(&metadata, "插件文件夹")?;
            if !metadata.is_dir()
            {
                return Err(format!("插件文件夹已被同名文件占用：{}", target.display()));
            }
            let local = read_manifest(&target.join("manifest.ini"))?; // 已安装版本的清单。
            if local.get("version").is_some_and(|version| same_version(version, &preset.version))
            {
                crate::log_info!("预设插件：版本相同，保留本地改动，插件={name}，版本={}", preset.version);
                skipped += 1;
                continue;
            }
        }
        replace_folder(&preset.path, &root, &target)?;
        crate::log_info!("预设插件：已安装，插件={name}，版本={}", preset.version);
        installed += 1;
    }
    crate::log_info!("预设插件：安装完成，已安装={installed}，已跳过={skipped}，目录={}", root.display());
    Ok(())
}

/// 先完整写入插件根下的临时目录，再替换目标；失败只清理临时目录，不删除用户数据。
fn replace_folder(source: &Path, root: &Path, target: &Path) -> Result<(), String>
{
    let temp = root.join(format!(".preset-install-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|value| value.as_nanos()).unwrap_or(0))); // 成功前的临时目录。
    let result = (|| {
        copy_tree(source, &temp)?;
        if target.exists()
        {
            std::fs::remove_dir_all(target).map_err(|error| format!("替换插件文件夹失败：{error}"))?;
        }
        std::fs::rename(&temp, target).map_err(|error| format!("提交插件文件夹失败：{error}"))
    })();
    if result.is_err()
    {
        let _ = std::fs::remove_dir_all(&temp); // 半装内容不留下。
    }
    result
}

/// 复制预设文件夹到临时位置，逐项创建文件，拒绝链接、重解析点和越界路径。
fn copy_tree(source: &Path, target: &Path) -> Result<(), String>
{
    std::fs::create_dir_all(target).map_err(|error| format!("创建插件临时目录失败：{error}"))?;
    let mut pending = vec![(source.to_path_buf(), target.to_path_buf())]; // 待复制的目录或文件。
    while let Some((from, to)) = pending.pop()
    {
        let metadata = match std::fs::symlink_metadata(&from)
        {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue, // 扫描后消失的项跳过。
            Err(error) => return Err(format!("读取预设插件文件失败：{error}")),
        };
        reject_reparse(&metadata, "预设插件文件")?;
        if metadata.is_dir()
        {
            if from != source
            {
                std::fs::create_dir(&to).map_err(|error| format!("创建插件文件夹失败：{error}"))?;
            }
            for entry in std::fs::read_dir(&from).map_err(|error| format!("读取预设插件文件夹失败：{error}"))?
            {
                let entry = entry.map_err(|error| format!("读取预设插件文件夹失败：{error}"))?; // 当前子项。
                pending.push((entry.path(), to.join(entry.file_name())));
            }
        }
        else if metadata.is_file()
        {
            let mut input = std::fs::File::open(&from).map_err(|error| format!("读取预设插件文件失败：{error}"))?; // 来源只读打开。
            let mut output = std::fs::OpenOptions::new().write(true).create_new(true).open(&to).map_err(|error| format!("写入插件文件失败：{error}"))?; // 临时目录内不覆盖任何文件。
            std::io::copy(&mut input, &mut output).map_err(|error| format!("复制插件文件失败：{error}"))?;
            output.sync_all().map_err(|error| format!("写入插件文件失败：{error}"))?;
        }
        else
        {
            return Err("预设插件包含不支持的文件夹或文件类型".to_string());
        }
    }
    Ok(())
}

/// 删除目标目录中清单标识匹配来源预设的文件夹；缺失清单或不匹配的保持不动。
fn remove_presets(root: &Path, ids: &HashSet<String>) -> Result<(), String>
{
    let entries = match std::fs::read_dir(root)
    {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()), // 插件目录不存在时没有可删内容。
        Err(error) => return Err(format!("读取插件目录失败：{error}")),
    };
    let mut removed = Vec::new(); // 已删除的预设文件夹名。
    for entry in entries
    {
        let entry = entry.map_err(|error| format!("读取插件目录失败：{error}"))?;
        let folder = entry.path(); // 目标目录中的一层文件夹。
        let metadata = match std::fs::symlink_metadata(&folder)
        {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("读取插件文件夹失败：{error}")),
        };
        if !metadata.is_dir()
        {
            continue;
        }
        if reject_reparse(&metadata, "插件文件夹").is_err()
        {
            crate::log_warn!("预设插件：跳过链接或重解析点，路径={}", folder.display());
            continue;
        }
        let manifest = folder.join("manifest.ini"); // 文件夹身份以清单为准。
        if !manifest.is_file()
        {
            continue;
        }
        let values = match read_manifest(&manifest)
        {
            Ok(values) => values,
            Err(error) =>
            {
                crate::log_warn!("预设插件：跳过无法读取的清单，路径={}，原因={error}", manifest.display());
                continue;
            }
        }; // 用户自有插件的清单不阻塞其余预设删除。
        let Some(id) = values.get("id")
        else
        {
            continue;
        };
        if !ids.contains(id.trim())
        {
            continue;
        }
        std::fs::remove_dir_all(&folder).map_err(|error| format!("删除预设插件失败：{error}"))?;
        crate::log_info!("预设插件：已删除，插件={id}，路径={}", folder.display());
        removed.push(folder.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default());
    }
    crate::log_info!("预设插件：卸载完成，已删除={}，目录={}", removed.len(), root.display());
    Ok(())
}

/// 扫描来源目录的一层子文件夹，返回文件夹名、路径和清单版本。
fn preset_folders(source: &Path) -> Result<Vec<(String, PresetFolder)>, String>
{
    let mut presets = Vec::new(); // 含清单的预设文件夹。
    for entry in std::fs::read_dir(source).map_err(|error| format!("读取预设插件来源失败：{error}"))?
    {
        let entry = entry.map_err(|error| format!("读取预设插件来源失败：{error}"))?;
        let path = entry.path(); // 当前子项。
        let metadata = std::fs::symlink_metadata(&path).map_err(|error| format!("读取预设插件来源失败：{error}"))?;
        if !metadata.is_dir()
        {
            continue;
        }
        reject_reparse(&metadata, "预设插件文件夹")?;
        if !path.join("manifest.ini").is_file()
        {
            continue;
        }
        let name = entry.file_name().into_string().map_err(|_| "预设插件文件夹名不是有效 Unicode".to_string())?; // 目标文件夹名。
        ensure_single_name(&name)?;
        let values = read_manifest(&path.join("manifest.ini"))?;
        let version = values.get("version").map(|value| value.trim().to_string()).filter(|value| !value.is_empty()).ok_or_else(|| format!("预设插件清单缺少 version：{name}"))?;
        presets.push((name, PresetFolder { path, version }));
    }
    Ok(presets)
}

/// 收集来源目录一层文件夹清单里的插件标识；来源缺失时返回空集合。
fn preset_ids(source: &Path) -> Result<HashSet<String>, String>
{
    let mut ids = HashSet::new(); // 随程序安装的预设标识。
    let entries = match std::fs::read_dir(source)
    {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(ids), // 来源目录已被删除时安全返回。
        Err(error) => return Err(format!("读取预设插件来源失败：{error}")),
    };
    for entry in entries
    {
        let entry = entry.map_err(|error| format!("读取预设插件来源失败：{error}"))?;
        let path = entry.path(); // 当前子项。
        let metadata = match std::fs::symlink_metadata(&path)
        {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("读取预设插件来源失败：{error}")),
        };
        if !metadata.is_dir() || reject_reparse(&metadata, "预设插件文件夹").is_err() || !path.join("manifest.ini").is_file()
        {
            continue;
        }
        let values = read_manifest(&path.join("manifest.ini"))?;
        if let Some(id) = values.get("id").map(|value| value.trim()).filter(|value| !value.is_empty())
        {
            ids.insert(id.to_string());
        }
    }
    Ok(ids)
}

/// 只读解析清单 [plugin] 节；链接、重解析点或非 UTF-8 内容直接拒绝。
fn read_manifest(path: &Path) -> Result<HashMap<String, String>, String>
{
    let metadata = match std::fs::symlink_metadata(path)
    {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(HashMap::new()), // 缺失清单不产生任何键。
        Err(error) => return Err(format!("读取插件清单失败：{error}")),
    };
    reject_reparse(&metadata, "插件清单")?;
    let text = std::fs::read_to_string(path).map_err(|error| format!("读取插件清单失败：{error}"))?;
    let mut section = false; // 当前行是否属于 [plugin]。
    let mut values = HashMap::new(); // 该节的键值。
    for line in text.trim_start_matches('\u{feff}').lines()
    {
        let line = line.trim();
        if line.is_empty() || line.starts_with(['#', ';'])
        {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|value| value.strip_suffix(']'))
        {
            section = name.trim().eq_ignore_ascii_case("plugin");
            continue;
        }
        if section && let Some((key, value)) = line.split_once('=')
        {
            values.insert(key.trim().to_string(), value.trim().to_string());
        }
    }
    Ok(values)
}

/// 版本按三段数字比较；无法解析时退回字符串比较。
fn same_version(left: &str, right: &str) -> bool
{
    match (parse_version(left), parse_version(right))
    {
        (Some(left), Some(right)) => left == right,
        _ => left.trim() == right.trim(),
    }
}

fn parse_version(value: &str) -> Option<(u64, u64, u64)>
{
    let mut parts = value.split('.');
    let version = (parts.next()?.parse().ok()?, parts.next()?.parse().ok()?, parts.next()?.parse().ok()?);
    parts.next().is_none().then_some(version)
}

/// 安装器传入的目录必须是绝对路径，不能包含父级跳转或设备路径。
fn absolute_directory(path: PathBuf, label: &str) -> Result<PathBuf, String>
{
    use std::path::Component;
    if !path.is_absolute() || path.components().any(|part| matches!(part, Component::ParentDir))
    {
        return Err(format!("{label}必须为绝对路径"));
    }
    if path.to_string_lossy().starts_with(r"\\.\")
    {
        return Err(format!("{label}不能是系统设备路径"));
    }
    Ok(path)
}

/// 目标必须仍位于插件根内部，防止清单或文件夹名把写入带出目录。
fn ensure_child(root: &Path, target: &Path) -> Result<(), String>
{
    if !target.starts_with(root) || target == root
    {
        return Err("插件文件夹超出插件目录".to_string());
    }
    Ok(())
}

/// 预设文件夹名只能是单层普通名称，不能借助分隔符或父级跳转改变目标。
fn ensure_single_name(name: &str) -> Result<(), String>
{
    use std::path::Component;
    let path = Path::new(name); // 文件夹名本身。
    if name.is_empty() || path.components().count() != 1 || !matches!(path.components().next(), Some(Component::Normal(_)))
    {
        return Err(format!("预设插件文件夹名无效：{name}"));
    }
    Ok(())
}

/// 链接和重解析点可能指向插件目录之外，拒绝沿其读写或删除。
fn reject_reparse(metadata: &std::fs::Metadata, label: &str) -> Result<(), String>
{
    use std::os::windows::fs::MetadataExt;
    if metadata.file_type().is_symlink() || metadata.file_attributes() & 0x400 != 0
    {
        return Err(format!("{label}不能是链接或重解析点"));
    }
    Ok(())
}

struct PresetFolder
{
    path: PathBuf,   // 来源预设文件夹。
    version: String, // 清单版本。
}

#[cfg(test)]
#[path = "preset_plugins.test.rs"]
mod tests;
