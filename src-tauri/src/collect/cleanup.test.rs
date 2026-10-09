use super::*;

/// 隔离目录内创建受管图片，测试不会访问用户附件。
fn fixture() -> (PathBuf, String)
{
    let config = std::env::temp_dir().join(format!("collect-cleanup-{}", crate::tools::time::next_timeline_timestamp()));
    let item = super::super::create(&config, "正文").unwrap();
    (config, item.name)
}

fn image(config: &Path, name: &str, file: &str) -> String { super::super::write_resource_bytes(config, name, file, b"image").unwrap() }

#[test]
fn managed_only_with_markdown_html_encoded_and_cross_document_references()
{
    let (config, name) = fixture();
    let keep = image(&config, &name, "有 空格.png");
    let html = image(&config, &name, "html&image.png");
    let cross = image(&config, &name, "cross.png");
    let unused = image(&config, &name, "unused.png");
    let resource = resource_directory(&config, &name).unwrap();
    fs::write(resource.join("unknown.png"), b"unknown").unwrap();
    fs::write(directory(&config).join("independent.png"), b"independent").unwrap();
    super::super::save(&config, &name, &format!("![引用][image]\n\n[image]: <{}>\n\n<img src=\"{}\">", keep.replace(' ', "%20"), html.replace('&', "&amp;"))).unwrap();
    super::super::create(&config, &format!("![跨文档](./{cross})")).unwrap();
    let mut recycled = Vec::new();
    cleanup(&config, &name, None, |proof| {
        recycled.push(proof.path.canonicalize().unwrap());
        fs::remove_file(&proof.path).map_err(|e| e.to_string())
    })
    .unwrap();
    assert_eq!(recycled, vec![resource.join("unused.png")]);
    assert!(!directory(&config).join(unused).exists());
    for reference in [keep, html, cross, "independent.png".into()]
    {
        assert!(directory(&config).join(reference).exists());
    }
    assert!(resource.join("unknown.png").exists());
    fs::remove_dir_all(config).unwrap();
}

#[test]
fn discard_only_new_and_saved_references_win()
{
    let (config, name) = fixture();
    let old = image(&config, &name, "old.png");
    let baseline = baseline(&config, &name).unwrap();
    let saved = image(&config, &name, "saved.png");
    let unused = image(&config, &name, "new.png");
    super::super::save(&config, &name, &format!("![已保存](<{saved}>)")).unwrap();
    cleanup(&config, &name, Some(&baseline), |proof| fs::remove_file(&proof.path).map_err(|e| e.to_string())).unwrap();
    assert!(directory(&config).join(old).exists());
    assert!(directory(&config).join(saved).exists());
    assert!(!directory(&config).join(unused).exists());
    fs::remove_dir_all(config).unwrap();
}

#[test]
fn modified_files_and_uncertain_references_are_preserved()
{
    let (config, name) = fixture();
    let modified = image(&config, &name, "modified.png");
    let unused = image(&config, &name, "unused.png");
    fs::write(directory(&config).join(modified), b"external changes").unwrap();
    for content in ["![未定义][missing]", "![未闭合](", "<img srcset=\"anything.png 1x\">", "<script>dynamic()</script>"]
    {
        super::super::save(&config, &name, content).unwrap();
        assert!(cleanup(&config, &name, None, |_| panic!("不确定时不能回收")).is_err());
    }
    super::super::save(&config, &name, "正文").unwrap();
    let mut count = 0;
    cleanup(&config, &name, None, |proof| {
        assert!(proof.path.ends_with("unused.png"));
        count += 1;
        Err("模拟系统回收失败".into())
    })
    .unwrap();
    assert_eq!(count, 1);
    assert!(directory(&config).join(unused).exists());
    fs::write(directory(&config).join("broken.md"), [0xff]).unwrap();
    assert!(cleanup(&config, &name, None, |_| panic!("读取失败不能回收")).is_err());
    fs::remove_dir_all(config).unwrap();
}

#[test]
fn renamed_resources_keep_identity_and_external_links_are_never_owned()
{
    let (config, name) = fixture();
    image(&config, &name, "unused.png");
    let external = config.join("external.png");
    fs::write(&external, b"external").unwrap();
    super::super::import_resource(&config, &name, &external, "link").unwrap();
    super::super::rename(&config, &name, "renamed.md", None).unwrap();
    let mut count = 0;
    cleanup(&config, "renamed.md", None, |proof| {
        assert!(proof.path.ends_with("renamed/unused.png"));
        count += 1;
        Ok(())
    })
    .unwrap();
    assert_eq!(count, 1);
    assert!(external.exists());
    fs::remove_dir_all(config).unwrap();
}

#[test]
fn html_attributes_and_file_urls_preserve_references()
{
    assert_eq!(destinations("<img src='a&amp;b.png'>").unwrap(), vec!["a&b.png"]);
    assert_eq!(destinations("<img src=a.png>").unwrap(), vec!["a.png"]);
    let (config, name) = fixture();
    let reference = image(&config, &name, "encoded.png");
    let path = directory(&config).join(reference).canonicalize().unwrap();
    let url = tauri::Url::from_file_path(path).unwrap();
    super::super::save(&config, &name, &format!("![图片](<{url}>)")).unwrap();
    cleanup(&config, &name, None, |_| panic!("文件 URL 仍被引用")).unwrap();
    fs::remove_dir_all(config).unwrap();
}

/// 嵌套图片需要新证据，记录损坏时保留全部附件。
#[test]
fn nested_managed_images_and_corrupt_records_are_conservative()
{
    let (config, name) = fixture();
    let root = resource_directory(&config, &name).unwrap();
    let nested = root.join("copy/images");
    fs::create_dir_all(&nested).unwrap();
    let image = nested.join("managed.png");
    fs::write(&image, b"nested").unwrap();
    record(&root, &image);
    let mut count = 0;
    cleanup(&config, &name, None, |proof| {
        assert!(proof.path.ends_with("copy/images/managed.png"));
        count += 1;
        Err("测试保留文件".into())
    })
    .unwrap();
    assert_eq!(count, 1);
    fs::write(root.join(MANIFEST), b"broken json").unwrap();
    assert!(cleanup(&config, &name, None, |_| panic!("归属记录损坏不能删除")).is_err());
    assert!(image.exists());
    fs::remove_dir_all(config).unwrap();
}

/// 使用项目已有 Shell 回收路径验证实际结果，只操作隔离夹具。
#[test]
fn native_cleanup_recycles_with_file_proof()
{
    use crate::contracts::ShellOperationKind;
    use crate::platform::windows::{ShellCompensationAction, ShellMenuOwner, compensate_desktop_items, initialize_shell_apartment, uninitialize_shell_apartment};
    initialize_shell_apartment().unwrap();
    let (config, name) = fixture();
    let reference = image(&config, &name, "recycle.png");
    let owner = ShellMenuOwner::create().unwrap();
    cleanup(&config, &name, None, |proof| {
        let result = compensate_desktop_items(&owner, ShellOperationKind::Recycle, vec![ShellCompensationAction::Recycle { reference: proof.path.to_string_lossy().into_owned() }], vec![proof.clone()])?;
        assert!(!result.permanent && !result.aborted);
        assert!(result.items.iter().any(|item| item.succeeded && item.result_reference.is_some()), "{result:?}");
        Ok(())
    })
    .unwrap();
    assert!(!directory(&config).join(reference).exists());
    drop(owner);
    fs::remove_dir_all(config).unwrap();
    uninitialize_shell_apartment();
}
