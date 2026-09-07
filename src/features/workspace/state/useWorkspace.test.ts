import assert from "node:assert/strict";
import test from "node:test";

import type { WorkspaceSnapshot } from "../../../contracts/workspace.ts";
import { isWorkspaceSnapshotNewer } from "./workspaceSnapshotOrdering.ts";

// Minimal workspace state used by snapshot ordering tests.
const snapshot = (revision: number, interactionRevision: number): WorkspaceSnapshot => ({
    schemaVersion: 1,
    revision,
    grid: { left: 20, top: 60, columnWidth: 92, rowHeight: 112, rows: 6, columns: 10 },
    fences: [],
    unclassifiedItems: [],
    interaction: { revision: interactionRevision, selectedItemIds: [], selectionAnchorId: null, keyboardFocus: { type: "background" } },
});

/** 验证轮询旧快照不会覆盖刚提交的布局或交互状态。 */
test("accepts only newer workspace snapshots", () => {
    const current = snapshot(4, 7);
    assert.equal(isWorkspaceSnapshotNewer(current, snapshot(4, 6)), false);
    assert.equal(isWorkspaceSnapshotNewer(current, snapshot(4, 8)), true);
    assert.equal(isWorkspaceSnapshotNewer(current, snapshot(5, 0)), true);
});
