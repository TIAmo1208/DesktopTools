import assert from "node:assert/strict";
import test from "node:test";

import { DesktopItemContractError, parseDesktopItemSnapshot, parseShellOperationResult, parseShellDragPreparation } from "./desktopItem.ts";

test("validates one-shot drag preparation and whole-selection fallback", () => {
    assert.equal(parseShellDragPreparation(null), null);
    assert.deepEqual(parseShellDragPreparation({ token: 1, threshold: { left: 4, top: 4 } }), { token: 1, threshold: { left: 4, top: 4 } });
    for (const invalid of [undefined, {}, { token: 0, threshold: { left: 4, top: 4 } }, { token: 1, threshold: null }, { token: 1, threshold: { left: Infinity, top: 4 } }, { token: 1, threshold: { left: 4, top: 0 } }]) {
        assert.throws(() => parseShellDragPreparation(invalid), DesktopItemContractError);
    }
});

// 整体 HRESULT 与逐项成功事实同时跨越桥接边界，错误码只接受失败的有符号 32 位值。
test("preserves partial results with an overall native error", () => {
    const payload = {
        operationId: 0,
        completedAtUnixMicros: 0,
        kind: "paste",
        aborted: false,
        operationError: -2147467259,
        reversible: true,
        permanent: false,
        effect: "copy",
        items: [{ sourceReference: "source", resultReference: "destination", succeeded: true, resultCode: 0 }],
        message: "操作未全部完成",
    }; // 原生完成一项后报告整体错误。
    const result = parseShellOperationResult(payload); // 已校验的部分结果。
    assert.equal(result.operationError, payload.operationError);
    assert.deepEqual(result.items, payload.items);
    for (const operationError of [0, 1, -2147483649, -1.5, "-1", null]) {
        assert.throws(() => parseShellOperationResult({ ...payload, operationError }), DesktopItemContractError);
    }
});

// 验证 Shell 快照保留可用项目与暂时解析失败项目。
test("accepts available and recoverable desktop items", () => {
    // 已解析的桌面项目快照。
    const snapshot = parseDesktopItemSnapshot({
        revision: 4,
        items: [
            {
                stableReference: "C:\\Users\\User\\Desktop\\Project",
                displayName: "Project",
                systemImage: "data:image/png;base64,AA==",
                availability: "available",
            },
            {
                stableReference: "::{645FF040-5081-101B-9F08-00AA002F954E}",
                displayName: "回收站",
                systemImage: null,
                availability: "unavailable",
            },
        ],
    });

    assert.equal(snapshot.revision, 4);
    assert.equal(snapshot.items[0].displayName, "Project");
    assert.equal(snapshot.items[1].availability, "unavailable");
});

// 验证进程边界拒绝未知桌面项目可用状态。
test("rejects an unknown desktop item availability", () => {
    assert.throws(
        () =>
            parseDesktopItemSnapshot({
                revision: 1,
                items: [
                    {
                        stableReference: "::{project}",
                        displayName: "Project",
                        systemImage: null,
                        availability: "missing",
                    },
                ],
            }),
        (error: unknown) => error instanceof DesktopItemContractError && error.code === "invalid-desktop-item-snapshot",
    );
});

// 验证 Shell 操作结果保留实际项目状态与不可逆属性。
test("accepts an actual permanent Shell operation result", () => {
    const result = parseShellOperationResult({
        operationId: 8,
        completedAtUnixMicros: 1_788_198_400_000_000,
        kind: "permanent-delete",
        aborted: false,
        reversible: false,
        permanent: true,
        effect: null,
        items: [
            {
                sourceReference: "C:\\Users\\User\\Desktop\\obsolete.txt",
                resultReference: null,
                succeeded: true,
                resultCode: 0,
            },
        ],
        message: "永久删除完成：1 项",
    });

    assert.equal(result.operationId, 8);
    assert.equal(result.completedAtUnixMicros, 1_788_198_400_000_000);
    assert.equal(result.kind, "permanent-delete");
    assert.equal(result.permanent, true);
    assert.equal(result.items[0].succeeded, true);
});
