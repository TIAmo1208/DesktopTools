use super::*;
use crate::contracts::settings::ApplicationPreferences;
use crate::persistence::save_json_atomic;

/// 仓库里的预设插件来源。
fn repository_plugins() -> PathBuf { PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../plugin").canonicalize().unwrap() }

/// 本测试独占的临时目录。
fn temporary_root(name: &str) -> PathBuf { std::env::temp_dir().join(format!("desktop-tools-preset-{name}-{}", std::process::id())) }

/// 建立启动定位目录，并把插件位置指针指向测试自己的插件根。
fn prepare_bootstrap(root: &Path, plugins: &Path) -> PathBuf
{
    let bootstrap = root.join("bootstrap"); // 模拟 %APPDATA%\<bundle id>。
    std::fs::create_dir_all(&bootstrap).unwrap();
    save_json_atomic(&bootstrap.join("plugin-location.json"), &plugins).unwrap();
    bootstrap
}

/// 复制一个目录树供测试修改来源。
fn copy_directory(source: &Path, target: &Path)
{
    std::fs::create_dir_all(target).unwrap();
    for entry in std::fs::read_dir(source).unwrap()
    {
        let entry = entry.unwrap();
        let destination = target.join(entry.file_name());
        if entry.path().is_dir()
        {
            copy_directory(&entry.path(), &destination);
        }
        else
        {
            std::fs::copy(entry.path(), destination).unwrap();
        }
    }
}

/// 预设插件随安装复制到指针目录；同版本保留本地改动，版本不同整目录替换。
#[test]
fn install_copies_presets_and_only_replaces_changed_versions()
{
    let root = temporary_root("install");
    let plugins = root.join("plugins");
    let bootstrap = prepare_bootstrap(&root, &plugins);
    let source = repository_plugins();
    let presets = ["示例-最小格子", "示例-设置条目", "示例-格子与设置"]; // 仓库预设的三个示例。
    install_preset_plugins(&source, &bootstrap).unwrap();
    for name in presets
    {
        assert!(plugins.join(name).join("manifest.ini").is_file(), "{name}");
        assert!(plugins.join(name).join("index.js").is_file(), "{name}");
    }
    assert!(std::fs::read_dir(&plugins).unwrap().all(|entry| !entry.unwrap().file_name().to_string_lossy().starts_with(".preset-install-")), "临时目录应清理");

    let local = plugins.join(presets[0]).join("本地改动.txt"); // 用户在同版本上的本地文件。
    std::fs::write(&local, b"keep").unwrap();
    install_preset_plugins(&source, &bootstrap).unwrap();
    assert_eq!(std::fs::read(&local).unwrap(), b"keep", "同版本跳过并保留本地改动");

    let changed_source = root.join("changed-source"); // 带新版本的来源副本。
    copy_directory(&source, &changed_source);
    let manifest = changed_source.join(presets[0]).join("manifest.ini");
    let text = std::fs::read_to_string(&manifest).unwrap().replace("version = 1.0.0", "version = 1.1.0");
    std::fs::write(&manifest, text).unwrap();
    std::fs::write(changed_source.join(presets[0]).join("新版本文件.txt"), b"new").unwrap();
    install_preset_plugins(&changed_source, &bootstrap).unwrap();
    assert!(plugins.join(presets[0]).join("新版本文件.txt").is_file(), "版本不同应整目录替换");
    assert!(!local.exists(), "替换后旧本地改动不再保留");
    std::fs::remove_dir_all(root).unwrap();
}

/// 有效插件目录沿用启动解析：指针优先；没有指针和偏好时回退默认目录，且不创建真实目录。
#[test]
fn effective_directory_prefers_pointer_then_preference_then_default()
{
    let root = temporary_root("effective");
    let bootstrap = root.join("bootstrap");
    let config = root.join("config");
    std::fs::create_dir_all(&bootstrap).unwrap();
    std::fs::create_dir_all(&config).unwrap();
    let default = PathBuf::from(std::env::var_os("APPDATA").unwrap()).join("TIAmo").join("plugins"); // 空偏好的默认插件根。
    assert_eq!(crate::app::settings::locate_plugin_directory(&bootstrap).unwrap(), default);
    save_json_atomic(&bootstrap.join("config-location.json"), &config).unwrap();
    let configured = root.join("configured-plugins"); // 偏好里显式配置的目录。
    save_json_atomic(&config.join("settings.json"), &ApplicationPreferences { plugin_directory: configured.to_string_lossy().into_owned(), ..Default::default() }).unwrap();
    assert_eq!(crate::app::settings::locate_plugin_directory(&bootstrap).unwrap(), configured);
    let pointer = root.join("pointer-plugins"); // 指针记录的上次生效目录。
    save_json_atomic(&bootstrap.join("plugin-location.json"), &pointer).unwrap();
    assert_eq!(crate::app::settings::locate_plugin_directory(&bootstrap).unwrap(), pointer);
    std::fs::remove_dir_all(root).unwrap();
}

/// 卸载只删除清单标识匹配的预设文件夹，保留用户插件；来源或目标缺失都安全返回。
#[test]
fn uninstall_removes_only_matching_presets_and_keeps_user_plugins()
{
    let root = temporary_root("uninstall");
    let plugins = root.join("plugins");
    let bootstrap = prepare_bootstrap(&root, &plugins);
    let source = repository_plugins();
    install_preset_plugins(&source, &bootstrap).unwrap();
    let user = plugins.join("我的插件"); // 用户自行安装的插件。
    std::fs::create_dir_all(&user).unwrap();
    std::fs::write(user.join("manifest.ini"), "[plugin]\nid = plugin.user.mine\nname = 我的插件\nversion = 1.0.0\nentry = index.js\n").unwrap();
    std::fs::write(user.join("index.js"), b"var mine = 1;").unwrap();
    let disguised = plugins.join("伪装"); // 文件夹名与预设无关，但清单标识属于预设。
    std::fs::create_dir_all(&disguised).unwrap();
    std::fs::write(disguised.join("manifest.ini"), "[plugin]\nid = plugin.example.minimal\nname = 伪装\nversion = 1.0.0\nentry = index.js\n").unwrap();
    let replaced = plugins.join("示例-最小格子").join("manifest.ini"); // 预设文件夹被用户改成别的标识。
    std::fs::write(&replaced, "[plugin]\nid = plugin.user.fake\nname = 示例-最小格子\nversion = 1.0.0\nentry = index.js\n").unwrap();
    uninstall_preset_plugins(&source, &bootstrap).unwrap();
    assert!(!disguised.exists(), "按清单标识删除，不依赖文件夹名");
    assert!(plugins.join("示例-最小格子").exists(), "标识不匹配的文件夹保留");
    assert!(user.join("index.js").is_file(), "用户插件保留");
    assert!(!plugins.join("示例-设置条目").exists());
    assert!(!plugins.join("示例-格子与设置").exists());
    uninstall_preset_plugins(&root.join("missing-source"), &bootstrap).unwrap();
    assert!(user.exists(), "来源缺失时安全返回且不删除用户插件");
    let missing_target = temporary_root("uninstall-missing-target"); // 目标目录不存在的启动定位目录。
    let missing_bootstrap = prepare_bootstrap(&missing_target, &missing_target.join("absent-plugins"));
    uninstall_preset_plugins(&source, &missing_bootstrap).unwrap();
    std::fs::remove_dir_all(root).unwrap();
    std::fs::remove_dir_all(missing_target).unwrap();
}

/// 安装来源缺失或路径不是绝对路径时明确报错，不创建目录。
#[test]
fn install_rejects_missing_source_and_relative_paths()
{
    let root = temporary_root("reject");
    let plugins = root.join("plugins");
    let bootstrap = prepare_bootstrap(&root, &plugins);
    assert!(install_preset_plugins(&root.join("missing-source"), &bootstrap).unwrap_err().contains("不存在"));
    assert!(install_preset_plugins(Path::new("plugin"), &bootstrap).unwrap_err().contains("绝对路径"));
    assert!(install_preset_plugins(&repository_plugins(), Path::new("relative-bootstrap")).unwrap_err().contains("绝对路径"));
    assert!(!plugins.exists(), "参数失败不创建插件目录");
    std::fs::remove_dir_all(root).unwrap();
}
