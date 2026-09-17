use super::*;

/// 部分遮挡不能暂停，负坐标及跨屏覆盖分别判定。
#[test]
fn coverage_is_per_monitor()
{
    let left = RECT { left: -1920, top: 0, right: 0, bottom: 1080 }; // 左侧显示器覆盖窗口。
    assert!(covers(left, -1920, 0, 1920, 1040));
    assert!(!covers(left, 0, 0, 1920, 1040));
    assert!(!covers(RECT { right: -1, ..left }, -1920, 0, 1920, 1040));
    assert!(covers(RECT { right: 1920, ..left }, 0, 0, 1920, 1080));
}
