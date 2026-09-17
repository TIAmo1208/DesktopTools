use super::*;

/// 同长度内容、附加流、新增后代和同名替换均使旧补偿授权失效。
#[test]
fn proof_detects_content_stream_tree_and_identity_changes()
{
    let root = std::env::temp_dir().join(format!("desktop-tools-proof-{}-{}", std::process::id(), crate::tools::time::next_timeline_timestamp())); // 独占测试目录。
    std::fs::create_dir(&root).unwrap();
    let path = root.join("item.txt");
    std::fs::write(&path, b"before").unwrap();
    let initial = FileProof::capture(&root).unwrap();
    assert!(initial.unchanged());
    assert!(initial.subtree_unchanged(&path));
    std::fs::write(&path, b"after!").unwrap();
    assert!(!initial.unchanged());
    let modified = FileProof::capture(&root).unwrap();
    let mut stream = path.as_os_str().to_os_string();
    stream.push(":proof-test:$DATA");
    std::fs::write(stream, b"named stream").unwrap();
    assert!(!modified.unchanged());
    let with_stream = FileProof::capture(&root).unwrap();
    std::fs::write(root.join("new.txt"), b"concurrent").unwrap();
    assert!(!with_stream.unchanged());
    let before_replace = FileProof::capture(&path).unwrap();
    std::fs::rename(&path, root.join("old.txt")).unwrap();
    std::fs::write(&path, b"after!").unwrap();
    assert!(!before_replace.unchanged());
    std::fs::remove_dir_all(root).unwrap();
}
