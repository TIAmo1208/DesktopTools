import assert from "node:assert/strict";
import test from "node:test";

import { DesktopItemContractError, parseDesktopItemSnapshot } from "./desktopItem.ts";

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
