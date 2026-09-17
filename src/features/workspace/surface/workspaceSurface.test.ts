import assert from "node:assert/strict";
import test from "node:test";
import { parseWorkspaceSnapshot } from "../../../contracts/workspace.ts";
import { projectWorkspaceSurface, projectHitRegions } from "./workspaceSurfaceProjection.ts";

/** 围栏跨屏后，保留的源桌面槽位不能变成第二份可见项目。 */
test("fence members appear only on the fence monitor", () => {
    const snapshot = { fences: [{ id: "f", monitorId: "B", members: ["x"], bounds: {} }], unclassifiedItems: [{ stableReference: "x", monitorId: "A", position: { left: 0, top: 0 } }], components: [], monitorGrids: [], interaction: { selectedItemIds: ["x"], selectionAnchorId: "x", keyboardFocus: { type: "item", itemId: "x" } } } as Parameters<typeof projectWorkspaceSurface>[0]; // 保留源槽位的真实投影输入。
    const source = projectWorkspaceSurface(snapshot, "A")!; // 围栏原来所在的屏幕。
    const target = projectWorkspaceSurface(snapshot, "B")!; // 围栏当前所在的屏幕。
    assert.deepEqual(source.unclassifiedItems, []);
    assert.deepEqual(source.interaction.selectedItemIds, []);
    assert.deepEqual(target.fences.flatMap((fence) => fence.members), ["x"]);
});

test("native hit regions follow stacking and exclude scrolled-out members", () => {
    const item = { id: "member", kind: "desktop-item", left: 10, top: -20, width: 40, height: 60, priority: 70, stackingOrder: 1001, clip: { left: 0, top: 20, width: 200, height: 100 } }; // 只露出底部二十 DIP 的成员。
    const regions = projectHitRegions([item, { ...item, id: "hidden", top: -100 }, { ...item, id: "front", kind: "fence", top: 0, priority: 50, stackingOrder: 1002, clip: undefined }], 200, 120);
    assert.equal(regions.some((region) => region.id === "hidden"), false);
    assert.deepEqual(regions[0], { id: "member", kind: "desktop-item", left: 10, top: 20, width: 40, height: 20, priority: 1 });
    assert.ok(regions[1].priority > regions[0].priority);
});

test("portal hit region retains the current directory through projection", () => {
    const portal = { id: "portal", kind: "component", left: 10, top: 10, width: 80, height: 60, priority: 60, stackingOrder: 1000, targetPath: "D:/Project/child" };
    assert.equal(projectHitRegions([portal], 200, 120)[0].targetPath, portal.targetPath);
});

/** 验证断屏投射只改变呈现，重连与跨屏拖动仍可使用原始权威事实。 */
test("projects a disconnected display without mutating global selection or ownership", () => {
    const grid = { left: 0, top: 0, columnWidth: 78, rowHeight: 98, rows: 9, columns: 12 }; // 当前宿主网格。
    const snapshot = parseWorkspaceSnapshot({
        schemaVersion: 1, revision: 7, grid, monitorGrids: [{ monitorId: "A", grid }],
        fences: [{ id: "f", title: "工作", monitorId: "B", projectionMonitorId: "A", bounds: { left: 500, top: 50, width: 300, height: 200 }, projectionBounds: { left: 10, top: 20, width: 300, height: 200 }, members: ["inside"], layoutMode: "tile", scrollTop: 0, collapsed: false, lastExpandedHeight: 200, zIndex: 1 }],
        unclassifiedItems: [
            { stableReference: "local", monitorId: "A", position: { left: 0, top: 0 }, zIndex: 0 },
            { stableReference: "remote", monitorId: "C", position: { left: 78, top: 0 }, zIndex: 0 },
        ],
        components: [],
        interaction: { revision: 3, selectedItemIds: ["local", "inside", "remote"], selectionAnchorId: "remote", keyboardFocus: { type: "item", itemId: "remote" } },
    }); // 来自完整虚拟桌面的权威快照。
    const original = structuredClone(snapshot); // 操作前完整事实。
    const projected = projectWorkspaceSurface(snapshot, "A")!; // 主屏当前展示。
    assert.deepEqual(projected.interaction.selectedItemIds, ["local", "inside"]);
    assert.equal(projected.interaction.selectionAnchorId, null);
    assert.deepEqual(projected.interaction.keyboardFocus, { type: "background" });
    assert.equal(projected.fences[0].bounds.left, 10);
    assert.equal(projected.fences[0].monitorId, "B");
    assert.deepEqual(snapshot, original);
    assert.equal(projectWorkspaceSurface(snapshot, "C")!.unclassifiedItems[0].stableReference, "remote");
    assert.equal(projectWorkspaceSurface(undefined, "A"), undefined);
    assert.equal(projectWorkspaceSurface(snapshot, undefined), undefined);
});
