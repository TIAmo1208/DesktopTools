import assert from "node:assert/strict";
import test from "node:test";

import { projectDesktopItems } from "./desktopItemViewModel.ts";

// 验证页面投影保留 Shell 身份、名称、图像和恢复状态。
test("projects desktop items for rendering", () => {
    // 页面使用的桌面项目展示模型。
    const items = projectDesktopItems({
        revision: 2,
        items: [
            {
                stableReference: "::{project}",
                displayName: "Project",
                systemImage: "data:image/png;base64,AA==",
                availability: "available",
            },
            {
                stableReference: "::{recovering}",
                displayName: "Recovering",
                systemImage: null,
                availability: "unavailable",
            },
        ],
    });

    assert.deepEqual(items[0], {
        key: "::{project}",
        label: "Project",
        imageSource: "data:image/png;base64,AA==",
        isAvailable: true,
        imageRevision: 2,
    });
    assert.equal(items[1].isAvailable, false);
});
