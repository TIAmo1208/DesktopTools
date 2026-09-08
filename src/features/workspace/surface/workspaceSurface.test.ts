import assert from "node:assert/strict";
import test from "node:test";
import { parseWorkspaceSnapshot } from "../../../contracts/workspace.ts";
import { projectWorkspaceSurface } from "./workspaceSurfaceProjection.ts";

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
    assert.equal(projectWorkspaceSurface(snapshot, undefined), snapshot);
});
