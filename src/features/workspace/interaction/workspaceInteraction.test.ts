import assert from "node:assert/strict";
import test from "node:test";

import {
    itemIdsIntersectingRect,
    nearestDirectionalItemId,
    nextDirectionalItemId,
    nextWorkspaceFocus,
    selectionModeForDirectionalKey,
    selectionModeForPointer,
    shouldOpenItemOnPointerRelease,
    workspaceFocusTargetsInVisualOrder,
} from "./workspaceInteraction.ts";

// 单击打开不得在拖动、取消、默认双击或修饰键选择时执行。
test("opens a single click only after an unmodified pointer release", () => {
    const modifiers = { ctrlKey: false, shiftKey: false, altKey: false, metaKey: false }; // 普通点击修饰键。
    assert.equal(shouldOpenItemOnPointerRelease(false, false, "pointerup", modifiers), true);
    assert.equal(shouldOpenItemOnPointerRelease(true, false, "pointerup", modifiers), false);
    assert.equal(shouldOpenItemOnPointerRelease(false, true, "pointerup", modifiers), false);
    for (const finishType of ["pointercancel", "lostpointercapture", "virtual-desktop-switch"]) assert.equal(shouldOpenItemOnPointerRelease(false, false, finishType, modifiers), false);
    for (const key of ["ctrlKey", "shiftKey", "altKey", "metaKey"]) assert.equal(shouldOpenItemOnPointerRelease(false, false, "pointerup", { ...modifiers, [key]: true }), false);
});

// 验证指针和方向键修饰键映射到已确认的选择规则。
test("maps pointer and directional modifiers to selection modes", () => {
    assert.equal(selectionModeForPointer({ ctrlKey: false, shiftKey: false }), "replace");
    assert.equal(selectionModeForPointer({ ctrlKey: true, shiftKey: false }), "toggle");
    assert.equal(selectionModeForPointer({ ctrlKey: true, shiftKey: true }), "range");
    assert.equal(selectionModeForDirectionalKey({ ctrlKey: true, shiftKey: false }), "preserve");
    assert.equal(selectionModeForDirectionalKey({ ctrlKey: false, shiftKey: true }), "range");
});

// 验证 Tab 与 Shift+Tab 在高层焦点目标间循环。
test("cycles high-level workspace focus", () => {
    // 当前测试覆盖的完整高层焦点顺序。
    const targets = [{ type: "background" } as const, { type: "fence", fenceId: "fence-1" } as const, { type: "component", componentId: "clock" } as const];
    assert.deepEqual(nextWorkspaceFocus(targets, targets[0], false), targets[1]);
    assert.deepEqual(nextWorkspaceFocus(targets, targets[0], true), targets[2]);
    assert.deepEqual(nextWorkspaceFocus(targets, { type: "item", itemId: "a" }, false), targets[0]);
    assert.deepEqual(nextWorkspaceFocus(targets, { type: "item", itemId: "a" }, true), targets[2]);
});

// 验证高层焦点按屏幕位置排列，并以持久层级裁决重叠目标。
test("orders high-level workspace targets by visual position and persistent layer", () => {
    // 当前测试按视觉位置和层级排序后的焦点目标。
    const targets = workspaceFocusTargetsInVisualOrder([
        { target: { type: "fence", fenceId: "lower" }, rect: { left: 300, top: 200, width: 120, height: 80 }, zIndex: 2 },
        { target: { type: "component", componentId: "clock" }, rect: { left: 80, top: 40, width: 120, height: 80 }, zIndex: 1 },
        { target: { type: "fence", fenceId: "upper" }, rect: { left: 300, top: 200, width: 120, height: 80 }, zIndex: 9 },
    ]);
    assert.deepEqual(targets, [
        { type: "background" },
        { type: "component", componentId: "clock" },
        { type: "fence", fenceId: "upper" },
        { type: "fence", fenceId: "lower" },
    ]);
});

// 验证方向键在当前焦点容器内选择对应空间方向的最近项目。
test("finds the nearest item in a spatial direction", () => {
    // 当前测试使用的规则网格项目。
    const items = [
        { id: "a", rect: { left: 0, top: 0, width: 40, height: 40 } },
        { id: "b", rect: { left: 100, top: 0, width: 40, height: 40 } },
        { id: "c", rect: { left: 0, top: 100, width: 40, height: 40 } },
        { id: "d", rect: { left: 100, top: 100, width: 40, height: 40 } },
    ];
    assert.equal(nearestDirectionalItemId(items, "a", "right"), "b");
    assert.equal(nearestDirectionalItemId(items, "a", "down"), "c");
    assert.equal(nearestDirectionalItemId(items, "d", "left"), "c");
    assert.equal(nearestDirectionalItemId(items, "a", "up"), undefined);
});

// 验证主轴投影相交的候选优先于更近的斜向候选。
test("prioritizes projected candidates during directional navigation", () => {
    // 当前测试覆盖投影候选和更近斜向候选。
    const items = [
        { id: "current", rect: { left: 0, top: 0, width: 40, height: 40 } },
        { id: "projected", rect: { left: 200, top: 0, width: 40, height: 40 } },
        { id: "diagonal", rect: { left: 50, top: 60, width: 40, height: 40 } },
    ];
    assert.equal(nearestDirectionalItemId(items, "current", "right"), "projected");
});

// 验证当前容器到达边缘后继续进入相邻容器。
test("continues directional navigation into an adjacent container", () => {
    // 当前焦点容器中的唯一项目。
    const currentContainer = [{ id: "current", rect: { left: 0, top: 0, width: 40, height: 40 } }];
    // 当前显示器中的完整可见项目集合。
    const allItems = [...currentContainer, { id: "adjacent", rect: { left: 180, top: 0, width: 40, height: 40 } }];
    assert.equal(nextDirectionalItemId(currentContainer, allItems, "current", "right"), "adjacent");
});

// 验证框选按输入视觉顺序返回与矩形相交的项目。
test("returns marquee intersections in visual order", () => {
    assert.deepEqual(
        itemIdsIntersectingRect(
            { left: 20, top: 20, width: 100, height: 100 },
            [
                { id: "a", rect: { left: 0, top: 0, width: 30, height: 30 } },
                { id: "b", rect: { left: 150, top: 20, width: 30, height: 30 } },
                { id: "c", rect: { left: 90, top: 90, width: 40, height: 40 } },
            ],
        ),
        ["a", "c"],
    );
});

// 验证框选锚点顺序来自屏幕几何位置。
test("orders marquee intersections by screen geometry", () => {
    assert.deepEqual(
        itemIdsIntersectingRect(
            { left: 0, top: 0, width: 400, height: 400 },
            [
                { id: "lower", rect: { left: 20, top: 200, width: 40, height: 40 } },
                { id: "upper", rect: { left: 100, top: 20, width: 40, height: 40 } },
            ],
        ),
        ["upper", "lower"],
    );
});
