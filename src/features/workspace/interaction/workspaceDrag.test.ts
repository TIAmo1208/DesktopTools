import assert from "node:assert/strict";
import test from "node:test";

import {
    beforeItemIdForFenceDrop,
    clientPointToPhysical,
    itemIdsInWorkspaceVisualOrder,
    monitorAtPhysicalPoint,
    physicalPointToMonitorDip,
    resolveFenceAutoScroll,
    resolveFenceInsertion,
    resolveGridDrop,
    resolveFreeDrop,
    resolveGridInsertion,
} from "./workspaceDrag.ts";

// 当前拖动算法测试使用的三行两列网格。
const grid = { left: 0, top: 0, columnWidth: 80, rowHeight: 100, rows: 3, columns: 2 };

test("preserves free-position group offsets and keeps the whole group in bounds", () => {
    const positions = new Map([["anchor", { left: 80, top: 100 }], ["left", { left: 0, top: 100 }]]); // 锚点不在整组最左侧。
    const result = resolveFreeDrop(["anchor", "left"], { left: 13, top: 27 }, positions);
    assert.deepEqual(result?.get("anchor"), { left: 80, top: 27 });
    assert.deepEqual(result?.get("left"), { left: 0, top: 27 });
    assert.equal(resolveFreeDrop(["missing"], { left: 0, top: 0 }, positions), null);
});

/** 验证组插入按主导轴和稳定传播顺序移动项目。 */
test("propagates a dragged group through the desktop grid", () => {
    // 当前组插入开始时的网格位置。
    const positions = new Map([
        ["a", { left: 0, top: 0 }],
        ["b", { left: 0, top: 100 }],
        ["target", { left: 0, top: 200 }],
        ["tail", { left: 80, top: 0 }],
    ]);
    // 当前目标上半区产生的组插入预览。
    const preview = resolveGridInsertion(["a", "b"], "target", { x: 30, y: 202 }, { left: 0, top: 200, width: 64, height: 76 }, positions, grid);
    assert.equal(preview?.active, true);
    assert.deepEqual(preview?.positions?.get("a"), { left: 0, top: 200 });
    assert.deepEqual(preview?.positions?.get("b"), { left: 80, top: 0 });
    assert.deepEqual(preview?.positions?.get("target"), { left: 80, top: 100 });
});

/** 验证自由释放为组成员分配最近空网格并保持顺序。 */
test("snaps a dragged group to available grid positions", () => {
    // 当前自由释放开始时的网格位置。
    const positions = new Map([
        ["a", { left: 0, top: 0 }],
        ["b", { left: 0, top: 100 }],
        ["fixed", { left: 0, top: 200 }],
    ]);
    const result = resolveGridDrop(["a", "b"], { left: 80, top: 0 }, positions, grid); // 当前自由释放后的完整网格位置。
    assert.deepEqual(result?.get("a"), { left: 80, top: 0 });
    assert.deepEqual(result?.get("b"), { left: 80, top: 100 });
});

/** 验证旧网格非整齐来源位置不会覆盖空槽后的项目。 */
test("keeps later icons stable when persisted positions are off-grid", () => {
    const positions = new Map([
        ["dragged", { left: 0, top: 0 }],
        ["later", { left: 0, top: 196 }],
        ["legacy", { left: 20, top: 396 }],
    ]);
    const result = resolveGridDrop(["dragged"], { left: 0, top: 98 }, positions, { left: 0, top: 0, columnWidth: 78, rowHeight: 98, rows: 7, columns: 2 });
    assert.deepEqual(result?.get("dragged"), { left: 0, top: 98 });
    assert.deepEqual(result?.get("later"), { left: 0, top: 196 });
});

/** 验证跨屏网格释放优先保持拖动组的 DIP 相对几何。 */
test("preserves dragged group geometry when the target grid has room", () => {
    const wideGrid = { ...grid, columns: 3 }; // 当前可以容纳 L 形组的三列目标网格。
    const positions = new Map([
        ["anchor", { left: 0, top: 0 }],
        ["right", { left: 80, top: 0 }],
        ["down", { left: 0, top: 100 }],
    ]); // 当前 L 形拖动组的来源 DIP 网格位置。
    const result = resolveGridDrop(["anchor", "right", "down"], { left: 80, top: 100 }, positions, wideGrid); // 当前目标显示器上的组释放结果。

    assert.deepEqual(result?.get("anchor"), { left: 80, top: 100 });
    assert.deepEqual(result?.get("right"), { left: 160, top: 100 });
    assert.deepEqual(result?.get("down"), { left: 80, top: 200 });
});

/** 验证平铺和列表围栏使用各自布局轴。 */
test("uses the fence layout axis for insertion", () => {
    const bounds = { left: 0, top: 0, width: 64, height: 76 }; // 当前围栏成员插入目标边界。
    assert.equal(resolveFenceInsertion({ x: 2, y: 38 }, bounds, "tile").placement, "before");
    assert.equal(resolveFenceInsertion({ x: 32, y: 74 }, bounds, "list").placement, "after");
    assert.equal(beforeItemIdForFenceDrop(["a", "b", "c"], ["a"], "b", "after"), "c");
});

/** 验证目标前已有空网格时不会把目标项目无谓下推。 */
test("fills an empty insertion gap before shifting later icons", () => {
    const positions = new Map([
        ["dragged", { left: 0, top: 0 }],
        ["target", { left: 0, top: 200 }],
    ]);
    const preview = resolveGridInsertion(["dragged"], "target", { x: 32, y: 170 }, { left: 0, top: 200, width: 64, height: 76 }, positions, grid);
    assert.deepEqual(preview?.positions?.get("dragged"), { left: 0, top: 100 });
    assert.deepEqual(preview?.positions?.get("target"), { left: 0, top: 200 });
});

/** 验证围栏边缘距离连续映射到自动滚动速度。 */
test("scales fence auto-scroll near the viewport edge", () => {
    assert.equal(resolveFenceAutoScroll(134, { top: 20, bottom: 140 }), 8);
    assert.equal(resolveFenceAutoScroll(80, { top: 20, bottom: 140 }), 0);
    assert.equal(resolveFenceAutoScroll(20, { top: 20, bottom: 140 }), -12);
});

/** 验证来源 DIP 经物理虚拟桌面坐标进入目标混合 DPI 工作区。 */
test("converts a cross-monitor pointer through physical coordinates", () => {
    const primary = {
        id: "DISPLAY1",
        name: "DISPLAY1",
        primary: true,
        x: 0,
        y: 0,
        width: 1920,
        height: 1080,
        workX: 0,
        workY: 0,
        workWidth: 1920,
        workHeight: 1040,
        scaleFactor: 1,
        dpi: 96,
    }; // 当前来源显示器物理状态。
    const secondary = {
        ...primary,
        id: "DISPLAY2",
        name: "DISPLAY2",
        primary: false,
        x: 1920,
        workX: 1920,
        width: 2560,
        workWidth: 2560,
        scaleFactor: 1.5,
        dpi: 144,
    }; // 当前目标混合 DPI 显示器物理状态。
    const physical = clientPointToPhysical({ x: 2070, y: 300 }, primary); // 来源逐屏客户区转换后的物理释放点。
    const target = monitorAtPhysicalPoint(physical, [primary, secondary]); // 物理释放点命中的目标显示器。

    assert.equal(target?.id, "DISPLAY2");
    assert.deepEqual(physicalPointToMonitorDip(physical, secondary), { left: 100, top: 200 });
});

/** 验证跨屏选择按显示器和容器视觉顺序组成一个拖动事务。 */
test("orders a cross-monitor selection as one stable drag group", () => {
    const monitors = [
        {
            id: "DISPLAY2",
            name: "DISPLAY2",
            primary: false,
            x: -1280,
            y: 0,
            width: 1280,
            height: 1024,
            workX: -1280,
            workY: 0,
            workWidth: 1280,
            workHeight: 984,
            scaleFactor: 1.25,
            dpi: 120,
        },
        {
            id: "DISPLAY1",
            name: "DISPLAY1",
            primary: true,
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
            workX: 0,
            workY: 0,
            workWidth: 1920,
            workHeight: 1040,
            scaleFactor: 1,
            dpi: 96,
        },
    ]; // 当前测试的负坐标双屏环境。
    const workspace = {
        schemaVersion: 1,
        revision: 1,
        grid,
        monitorGrids: monitors.map((monitor) => ({ monitorId: monitor.id, grid })),
        fences: [],
        unclassifiedItems: [
            { stableReference: "right", monitorId: "DISPLAY1", projectionMonitorId: null, projectionPosition: null, position: { left: 20, top: 60 }, zIndex: 1 },
            { stableReference: "left-bottom", monitorId: "DISPLAY2", projectionMonitorId: null, projectionPosition: null, position: { left: 20, top: 172 }, zIndex: 1 },
            { stableReference: "left-top", monitorId: "DISPLAY2", projectionMonitorId: null, projectionPosition: null, position: { left: 20, top: 60 }, zIndex: 1 },
        ],
        interaction: { revision: 1, selectedItemIds: [], selectionAnchorId: null, keyboardFocus: { type: "background" as const } },
    }; // 当前跨屏选择使用的完整工作区快照。

    assert.deepEqual(itemIdsInWorkspaceVisualOrder(["right", "left-bottom", "left-top"], workspace, monitors), ["left-top", "left-bottom", "right"]);
});
