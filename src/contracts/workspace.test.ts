import assert from "node:assert/strict";
import test from "node:test";

import { parseWorkspaceBackupSummary, parseWorkspaceHistoryResult, parseWorkspaceSnapshot, parseWorkspaceRestoreResult } from "./workspace.ts";

// 备份与统一历史契约测试复用的最小工作区快照。
const emptyWorkspace = {
    schemaVersion: 1,
    revision: 2,
    grid: { left: 20, top: 60, columnWidth: 92, rowHeight: 112, rows: 9, columns: 13 },
    monitorGrids: [],
    fences: [],
    unclassifiedItems: [],
    interaction: {
        revision: 0,
        selectedItemIds: [],
        selectionAnchorId: null,
        keyboardFocus: { type: "background" },
    },
};

// 验证旧排序字段默认值、完整状态和非法输入边界。
test("门户允许重复路径但拒绝重复身份和非法几何", () => {
    const portal = { id: "portal-1", rootPath: "C:\\资料", monitorId: "DISPLAY1", bounds: { left: 0, top: 0, width: 360, height: 280 }, zIndex: 0, appearance: { title: null, titleMode: "fixed", buttonMode: "hover", collapsed: false, autoExpand: false } }; // 真实工作区门户契约。
    const parsed = parseWorkspaceSnapshot({ ...emptyWorkspace, folderPortals: [portal, { ...portal, id: "portal-2" }] });
    assert.equal(parsed.folderPortals?.length, 2);
    assert.throws(() => parseWorkspaceSnapshot({ ...emptyWorkspace, folderPortals: [portal, portal] }));
    assert.throws(() => parseWorkspaceSnapshot({ ...emptyWorkspace, folderPortals: [{ ...portal, bounds: { ...portal.bounds, width: -1 } }] }));
});

test("validates fence sorting modes and defaults legacy snapshots to none", () => {
    const fence = {
        id: "fence-1",
        title: "排序",
        bounds: { left: 0, top: 0, width: 320, height: 240 },
        members: [],
        layoutMode: "tile",
        scrollTop: 0,
        collapsed: false,
        lastExpandedHeight: 240,
        zIndex: 1,
    }; // 旧围栏快照不包含排序字段。
    assert.equal(parseWorkspaceSnapshot({ ...emptyWorkspace, fences: [fence] }).fences[0].sortMode, "none");
    for (const sortMode of ["none", "name-ascending", "name-descending"])
        assert.equal(parseWorkspaceSnapshot({ ...emptyWorkspace, fences: [{ ...fence, sortMode }] }).fences[0].sortMode, sortMode);
    for (const sortMode of [null, "unknown", 1, {}]) assert.throws(() => parseWorkspaceSnapshot({ ...emptyWorkspace, fences: [{ ...fence, sortMode }] }));
});

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
        unclassifiedItems: [{ stableReference: "::{first}", position: { left: 20, top: 40 }, zIndex: 4 }],
        components: [
            {
                instanceId: "official.local-clock.default",
                componentId: "official.local-clock",
                monitorId: "DISPLAY1",
                projectionMonitorId: null,
                projectionBounds: null,
                bounds: { left: 860, top: 560, width: 220, height: 116 },
                stateVersion: 1,
                state: { format: "24-hour" },
            },
        ],
        interaction: {
            revision: 3,
            selectedItemIds: ["::{first}"],
            selectionAnchorId: "::{first}",
            keyboardFocus: { type: "item", itemId: "::{first}" },
        },
    });

    assert.equal(snapshot.fences[0].members[1], "::{second}");
    assert.equal(snapshot.fences[0].lastExpandedHeight, 280);
    assert.equal(snapshot.grid.rows, 9);
    assert.deepEqual(snapshot.unclassifiedItems[0].position, { left: 20, top: 40 });
    assert.equal(snapshot.unclassifiedItems[0].zIndex, 4);
    assert.equal(snapshot.components[0].componentId, "official.local-clock");
    assert.deepEqual(snapshot.interaction, {
        revision: 3,
        selectedItemIds: ["::{first}"],
        selectionAnchorId: "::{first}",
        keyboardFocus: { type: "item", itemId: "::{first}" },
    });
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

// 验证版本、完整性校验和布局数量通过备份摘要契约。
test("parses a validated workspace backup summary", () => {
    const summary = parseWorkspaceBackupSummary({
        schemaVersion: 5,
        createdAtUnix: 1_788_198_400,
        path: "C:\\Backups\\workspace.json",
        checksum: "fnv1a64:0123456789abcdef",
    });

    assert.equal(summary.checksum, "fnv1a64:0123456789abcdef");
});

// 验证完整还原结果同时携带权威快照和还原结果。
test("parses a transactional workspace restore result", () => {
    const result = parseWorkspaceRestoreResult({
        snapshot: emptyWorkspace,
        message: "工作区还原完成",
    });

    assert.equal(result.snapshot.revision, 2);
});

// 验证统一 Shell 历史结果保留部分恢复说明和完成时间。
test("parses a unified Shell history result", () => {
    const result = parseWorkspaceHistoryResult({
        snapshot: emptyWorkspace,
        kind: "shell",
        shellOperation: {
            operationId: 9,
            completedAtUnixMicros: 1_788_198_400_000_000,
            kind: "rename",
            aborted: false,
            reversible: true,
            permanent: false,
            effect: null,
            items: [
                {
                    sourceReference: "C:\\Desktop\\new.txt",
                    resultReference: "C:\\Desktop\\old.txt",
                    succeeded: true,
                    resultCode: 0,
                },
            ],
            message: "已恢复 1 项，0 项状态已变化",
        },
        message: "已恢复 1 项，0 项状态已变化",
    });

    assert.equal(result.kind, "shell");
    assert.equal(result.shellOperation?.operationId, 9);
});
