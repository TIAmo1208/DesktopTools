import assert from "node:assert/strict";
import test from "node:test";

import { parseWorkspaceSnapshot } from "./workspace.ts";

// 验证围栏完整状态通过前后端公开快照恢复。
test("parses the complete persisted fence snapshot", () => {
    // 模拟 Rust command 与 event 发布的工作区快照。
    const snapshot = parseWorkspaceSnapshot({
        schemaVersion: 1,
        revision: 7,
        grid: { left: 20, top: 60, columnWidth: 92, rowHeight: 112, rows: 9, columns: 13 },
        fences: [
            {
                id: "fence-1",
                title: "项目资料",
                bounds: { left: 120, top: 80, width: 360, height: 280 },
                members: ["::{first}", "::{second}"],
                layoutMode: "tile",
                scrollTop: 42,
                collapsed: true,
                lastExpandedHeight: 280,
                zIndex: 3,
            },
        ],
        unclassifiedItems: [{ stableReference: "::{first}", position: { left: 20, top: 40 } }],
    });

    assert.equal(snapshot.fences[0].members[1], "::{second}");
    assert.equal(snapshot.fences[0].lastExpandedHeight, 280);
    assert.equal(snapshot.grid.rows, 9);
    assert.deepEqual(snapshot.unclassifiedItems[0].position, { left: 20, top: 40 });
});

// 验证无效围栏状态在公开契约边界被拒绝。
test("rejects an invalid persisted fence snapshot", () => {
    assert.throws(
        () =>
            parseWorkspaceSnapshot({
                schemaVersion: 1,
                revision: 0,
                grid: { left: 20, top: 60, columnWidth: 92, rowHeight: 112, rows: 9, columns: 13 },
                fences: [{ id: "fence-1", bounds: { left: 0, top: 0, width: -1, height: 100 } }],
                unclassifiedItems: [],
            }),
        { name: "WorkspaceContractError", message: "工作区数据无效" },
    );
});
