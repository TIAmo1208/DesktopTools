use super::{load_json, save_json_atomic};
use serde::{Deserialize, Serialize};

/// 原子 JSON 保存测试使用的最小载荷。
#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
struct Payload
{
    /// 当前测试修订号。
    revision: u64,
}

/// 验证原子替换可以连续更新同一快照文件。
#[test]
fn atomic_json_save_replaces_an_existing_snapshot()
{
    // 当前测试独占的快照文件。
    let path = std::env::temp_dir().join(format!("desktop-tools-persistence-{}.json", std::process::id()));
    save_json_atomic(&path, &Payload { revision: 1 }).expect("首次快照应可保存");
    save_json_atomic(&path, &Payload { revision: 2 }).expect("现有快照应可原子替换");

    assert_eq!(load_json(&path).expect("快照应可读取"), Some(Payload { revision: 2 }));
    std::fs::remove_file(path).expect("测试快照应可清理");
}
