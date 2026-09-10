use std::sync::atomic::{AtomicI64, Ordering};

use chrono::Utc;

/// 进程内最近一次统一时间线完成时间。
static LAST_TIMELINE_TIMESTAMP: AtomicI64 = AtomicI64::new(0);

/// 返回不早于真实 Unix 微秒且在当前进程内严格递增的完成时间。
pub fn next_timeline_timestamp() -> i64
{
    let observed = Utc::now().timestamp_micros(); // 当前系统时钟提供的 Unix 微秒值。
    let mut previous = LAST_TIMELINE_TIMESTAMP.load(Ordering::Relaxed); // 最近已经分配的统一时间线值。
    loop
    {
        let next = observed.max(previous.saturating_add(1)); // 系统时钟回拨或同刻度时仍保持严格顺序。
        match LAST_TIMELINE_TIMESTAMP.compare_exchange_weak(previous, next, Ordering::Relaxed, Ordering::Relaxed)
        {
            Ok(_) => return next,
            Err(actual) => previous = actual,
        }
    }
}

#[cfg(test)]
mod tests
{
    use super::next_timeline_timestamp;

    /// 验证同一系统时钟刻度内分配的完成时间仍严格递增。
    #[test]
    fn timeline_timestamps_are_strictly_increasing()
    {
        let first = next_timeline_timestamp(); // 首个统一时间线完成时间。
        let second = next_timeline_timestamp(); // 紧邻分配的下一个完成时间。

        assert!(second > first);
    }
}
