use super::next_timeline_timestamp;

/// 验证同一系统时钟刻度内分配的完成时间仍严格递增。
#[test]
fn timeline_timestamps_are_strictly_increasing()
{
    let first = next_timeline_timestamp(); // 首个统一时间线完成时间。
    let second = next_timeline_timestamp(); // 紧邻分配的下一个完成时间。

    assert!(second > first);
}
