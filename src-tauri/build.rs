// 生成 Tauri 正式应用构建元数据。
fn main()
{
    // 关闭 tauri-build 自带的 Windows 清单注入：它只作用于 bin 目标，
    // 而 lib 测试二进制同样静态导入 comctl32 v6 才提供的 TaskDialogIndirect，
    // 缺清单时加载器在入口点之前就以 0xC0000139 (STATUS_ENTRYPOINT_NOT_FOUND) 退出。
    let attributes = tauri_build::Attributes::new().windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest());
    if let Err(error) = tauri_build::try_build(attributes)
    {
        panic!("tauri-build 生成构建元数据失败：{error}");
    }
    // 改为给全部目标注入同一条 comctl32 v6 依赖；版本信息、图标等资源仍由 tauri-build 生成。
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
    {
        // 全目标共用的清单文件。
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("windows.manifest");
        println!("cargo:rerun-if-changed={}", manifest.display());
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
    }
}
