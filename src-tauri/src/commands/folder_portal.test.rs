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
