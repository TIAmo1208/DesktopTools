use super::*;

/// 验证已转换的 DIP、遮挡和失效成员不会误命中围栏。
#[test]
fn drop_fence_uses_confirmed_geometry_and_current_membership()
{
    let mut snapshot: crate::contracts::WorkspaceSnapshot = serde_json::from_value(serde_json::json!({
        "schemaVersion": 1, "revision": 0, "unclassifiedItems": [],
        "fences": [{ "id": "fence", "title": "", "bounds": { "left": 0, "top": 0, "width": 200, "height": 200 }, "members": ["member"], "layoutMode": "tile", "scrollTop": 0, "collapsed": false, "lastExpandedHeight": 200, "zIndex": 1 }]
    }))
    .unwrap(); // 当前共享归属的最小快照。
    let mut regions = vec![HitRegion { id: "fence".into(), kind: "fence".into(), left: 10.0, top: 20.0, width: 100.0, height: 100.0, priority: 1, target_path: None }]; // 宿主发布的 DIP 围栏区域。
    let point = WorkspacePoint { left: 20.0, top: 30.0 }; // 平台已转换的工作区 DIP。
    assert_eq!(super::drop_fence_at(&point, &regions, &snapshot).as_deref(), Some("fence"));
    assert_eq!(super::drop_fence_at(&WorkspacePoint { left: 110.0, top: 30.0 }, &regions, &snapshot), None);
    regions.push(HitRegion { id: "member".into(), kind: "desktop-item".into(), priority: 2, ..regions[0].clone() });
    assert_eq!(super::drop_before_item_at(&point, &regions, &snapshot.fences[0], &[]).flatten().as_deref(), Some("member"));
    assert_eq!(super::drop_before_item_at(&point, &regions, &snapshot.fences[0], &["member".into()]), None);
    assert_eq!(super::drop_fence_at(&point, &regions, &snapshot).as_deref(), Some("fence"));
    snapshot.fences[0].members.clear();
    assert_eq!(super::drop_fence_at(&point, &regions, &snapshot), None);
    regions[1].kind = "component".into();
    assert_eq!(super::drop_fence_at(&point, &regions, &snapshot), None);
    regions.pop();
    snapshot.fences.clear();
    assert_eq!(super::drop_fence_at(&point, &regions, &snapshot), None);
}

/// 同围栏无预览保持原序，显式末尾插入与跨来源追加仍可提交。
#[test]
fn same_fence_without_insertion_keeps_order_but_explicit_end_is_valid()
{
    let snapshot: WorkspaceSnapshot = serde_json::from_value(serde_json::json!({"schemaVersion":1,"revision":0,"unclassifiedItems":[],"fences":[{"id":"fence","title":"","bounds":{"left":0,"top":0,"width":200,"height":200},"members":["source","last"],"layoutMode":"tile","scrollTop":0,"collapsed":false,"lastExpandedHeight":200,"zIndex":1}]})).unwrap(); // 手动排序的两项围栏。
    let input = |x: f64| DropInput { monitor_id: "screen".into(), pointer: WorkspacePoint { left: x, top: 50.0 }, regions: vec![HitRegion { id: "fence".into(), kind: "fence".into(), left: 0.0, top: 0.0, width: 200.0, height: 200.0, priority: 1, target_path: None }, HitRegion { id: "last".into(), kind: "desktop-item".into(), left: 20.0, top: 20.0, width: 80.0, height: 80.0, priority: 2, target_path: None }], scrolls: Vec::new() }; // 目标项目中心与左右插入区。
    assert!(target_at(input(60.0), &snapshot, &["source".into()]).unwrap().is_none());
    assert!(target_at(input(150.0), &snapshot, &["source".into()]).unwrap().is_none());
    assert_eq!(target_at(input(25.0), &snapshot, &["source".into()]).unwrap().unwrap().before_item_id.as_deref(), Some("last"));
    assert!(target_at(input(95.0), &snapshot, &["source".into()]).unwrap().unwrap().before_item_id.is_none());
    assert!(target_at(input(150.0), &snapshot, &["external".into()]).unwrap().is_some());
}

/// 门户命中使用已发布的当前目录，不能把普通组件当作目录目标。
#[test]
fn portal_target_requires_a_matching_instance_and_current_path()
{
    let mut snapshot = WorkspaceSnapshot::default();
    snapshot.folder_portals.push(crate::contracts::FolderPortalInstance { id: "portal".into(), root_path: "D:/Project".into(), monitor_id: "screen".into(), bounds: crate::contracts::WorkspaceRect { left: 10.0, top: 10.0, width: 100.0, height: 100.0 }, z_index: 1, appearance: Default::default() });
    let input = |id: &str, path: Option<&str>| DropInput { monitor_id: "screen".into(), pointer: WorkspacePoint { left: 20.0, top: 20.0 }, regions: vec![HitRegion { id: id.into(), kind: "component".into(), left: 10.0, top: 10.0, width: 100.0, height: 100.0, priority: 1, target_path: path.map(str::to_string) }], scrolls: Vec::new() };
    let target = super::target_at(input("portal", Some("D:/Project/child")), &snapshot, &[]).unwrap().unwrap();
    assert_eq!(target.portal_id.as_deref(), Some("portal"));
    assert_eq!(target.portal_path.as_deref(), Some("D:/Project/child"));
    assert!(super::target_at(input("music", None), &snapshot, &[]).unwrap().is_none());
}
