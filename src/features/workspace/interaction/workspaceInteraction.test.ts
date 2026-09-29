import assert from "node:assert/strict";
import test from "node:test";
import { registerHooks } from "node:module";
import { readFileSync } from "node:fs";

import {
    intersectInteractionRects,
    uncoveredInteractionRects,
    itemIdsIntersectingRect,
    nearestDirectionalItemId,
    nextDirectionalItemId,
    nextWorkspaceFocus,
    selectionModeForDirectionalKey,
    selectionModeForPointer,
    shouldOpenItemOnPointerRelease,
    workspaceFocusTargetsInVisualOrder,
} from "./workspaceInteraction.ts";

// 框选只命中可见交集，多个上层容器共同覆盖时也不能穿透。
test("clips marquee rectangles and removes overlapping semantic surfaces", () => {
    const rect = { left: 0, top: 0, width: 100, height: 100 }; // 起点容器内的候选选框。
    assert.equal(intersectInteractionRects(rect, { left: 100, top: 0, width: 20, height: 20 }), undefined);
    assert.deepEqual(intersectInteractionRects(rect, { left: 80, top: 80, width: 40, height: 40 }), { left: 80, top: 80, width: 20, height: 20 });
    assert.deepEqual(uncoveredInteractionRects(rect, [rect]), []);
    assert.deepEqual(uncoveredInteractionRects(rect, [{ ...rect, width: 50 }, { ...rect, left: 50, width: 50 }]), []);
    const visible = uncoveredInteractionRects(rect, [{ left: 20, top: 20, width: 60, height: 60 }]); // 中间被遮挡，四边仍可命中。
    assert.equal(visible.reduce((area, part) => area + part.width * part.height, 0), 6400);
    assert.equal(visible.some((part) => intersectInteractionRects(part, { left: 30, top: 30, width: 20, height: 20 })), false);
});

// 单击打开不得在拖动、取消、默认双击或修饰键选择时执行。
// 在正式 Hook 上验证入口、容器范围、取消清理与迟到异步结果。
test("marquee pointer sessions stay inside their origin container and clean up", async () => {
    class Surface extends EventTarget {
        dataset: Record<string, string> = {}; // 当前语义身份。
        style: Record<string, string> = {}; // 选框呈现结果。
        children: Surface[] = []; // 最小 DOM 树。
        parent?: Surface; // 语义容器祖先。
        isConnected = true; // 模拟挂载状态。
        captured = false; // 指针捕获状态。
        className = ""; // 复用正式预览样式。
        zIndex = "0"; // 原生呈现层级。
        rect = { left: 0, top: 0, width: 400, height: 300 }; // 当前可见几何。
        focus() {}
        getBoundingClientRect() { return this.rect; }
        append(child: Surface) { this.children.push(child); child.parent = this; }
        remove() { this.isConnected = false; if (this.parent) this.parent.children = this.parent.children.filter((child) => child !== this); }
        closest(): Surface | null { return this.dataset.fenceId ? this : this.parent?.closest() ?? null; }
        setPointerCapture() { this.captured = true; }
        hasPointerCapture() { return this.captured; }
        releasePointerCapture() { this.captured = false; this.dispatchEvent(new Event("lostpointercapture")); }
        querySelectorAll(selector: string): Surface[] {
            return this.children.flatMap((child) => [
                ...(selector === "[data-item-id]" ? (child.dataset.itemId ? [child] : []) : selector === ".desktop-tile" ? (child.className === selector.slice(1) ? [child] : []) : []),
                ...child.querySelectorAll(selector),
            ]);
        }
        querySelector(selector: string): Surface | null {
            if (selector === ".fence-workspace__unclassified") return this.children.find((child) => child.className === selector.slice(1)) ?? null;
            return null;
        }
    }
    const api = { effects: [] as (() => void | (() => void))[] }; // React 生命周期替身。
    const win = new EventTarget(); // 宿主事件入口。
    const previous = new Map(["window", "document", "Element", "__marqueeTest"].map((key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)])); // 测试结束恢复环境。
    Object.assign(globalThis, { window: Object.assign(win, { getComputedStyle: (node: Surface) => ({ zIndex: node.zIndex }) }), document: { createElement: () => new Surface() }, Element: Surface, __marqueeTest: api });
    const hooks = registerHooks({
        resolve(specifier, context, next) {
            if (specifier === "react") return { url: "data:text/javascript,export const useRef=v=>({current:v});export const useEffect=f=>globalThis.__marqueeTest.effects.push(f)", shortCircuit: true };
            if (specifier.endsWith("/workspaceInteraction")) return next(specifier + ".ts", context);
            return next(specifier, context);
        },
    }); // 只替换平台边界，真实 Hook 和几何逻辑直接执行。
    const settle = async () => { for (let i = 0; i < 8; i++) await Promise.resolve(); }; // 等待初始焦点及最终选择。
    const pointer = (type: string, x = 300, y = 280) => Object.assign(new Event(type), { pointerId: 1, clientX: x, clientY: y }); // 同一指针输入序列。
    try {
        const { useWorkspaceInteraction } = await import("./useWorkspaceInteraction.ts"); // 被测正式交互入口。
        const mount = (delayFocus?: Promise<void>) => {
            api.effects = [];
            const root = new Surface(); // 当前显示器工作区。
            const desktop = new Surface(); // 未归类范围。
            desktop.className = "fence-workspace__unclassified";
            root.append(desktop);
            const fence = new Surface(); // 当前起点围栏。
            fence.dataset.fenceId = "f";
            fence.className = "desktop-tile";
            fence.zIndex = "1001";
            fence.rect = { left: 100, top: 10, width: 120, height: 160 };
            root.append(fence);
            const content = new Surface(); // 可滚动内容视口。
            content.dataset.fenceContentId = "f";
            content.rect = { left: 110, top: 40, width: 100, height: 120 };
            fence.append(content);
            const upper = new Surface(); // 覆盖目标围栏右侧的上层表面。
            upper.className = "desktop-tile";
            upper.zIndex = "1002";
            upper.rect = { left: 150, top: 40, width: 60, height: 120 };
            root.append(upper);
            for (const [parent, id, x, y] of [[desktop, "desktop", 30, 50], [desktop, "under-fence", 160, 100], [content, "visible", 120, 60], [content, "clipped", 120, 240], [content, "partial", 120, 150], [content, "covered", 160, 60]] as const) {
                const item = new Surface(); // 实际候选节点。
                item.dataset.itemId = id;
                item.rect = { left: x, top: y, width: 20, height: 30 };
                parent.append(item);
            }
            const commands: any[] = []; // 回调触发、状态和异步结果证据。
            const options = { workspaceRef: { current: root }, unclassifiedItemIds: [], isContextMenuOpen: () => false, onInteraction: async (command: any) => { commands.push(command); if (command.type === "focus-surface" && delayFocus) await delayFocus; return {}; } }; // 正式交互回调的最小宿主。
            const interaction = useWorkspaceInteraction(options as any); // 未使用的键盘业务不参与框选场景。
            const cleanups = api.effects.map((effect) => effect()); // 挂载正式监听。
            return { root, desktop, content, commands,
                down: (target = content, x = 115, y = 45, button = 0) => interaction.handleWorkspacePointerDown({ currentTarget: target, target, button, pointerId: 1, clientX: x, clientY: y, preventDefault() {}, stopPropagation() {} } as any),
                dispose: () => cleanups.forEach((cleanup) => cleanup?.()),
            };
        };
        let current = mount();
        current.down();
        assert.equal(current.content.captured, true);
        assert.deepEqual(current.commands[0], { type: "focus-surface", focus: { type: "fence", fenceId: "f" } });
        current.content.dispatchEvent(pointer("pointermove"));
        assert.equal(current.root.children.at(-1)?.style.width, "95px");
        current.content.dispatchEvent(pointer("pointerup"));
        await settle();
        assert.deepEqual(current.commands.at(-1), { type: "select-items", itemIds: ["visible", "partial"] });
        assert.equal(current.content.captured, false);
        assert.equal(current.root.children.length, 3);
        current.dispose();

        current = mount();
        current.down(current.desktop, 0, 0);
        current.desktop.dispatchEvent(pointer("pointerup"));
        await settle();
        assert.deepEqual(current.commands.at(-1), { type: "select-items", itemIds: ["desktop"] });
        current.dispose();

        current = mount();
        current.down();
        current.content.dispatchEvent(pointer("pointerup", 115, 45));
        await settle();
        assert.equal(current.commands.length, 1);
        assert.equal(current.commands[0].focus.type, "fence");
        current.dispose();

        for (const reason of ["pointercancel", "lostpointercapture", "escape", "blur", "unmount"]) {
            current = mount();
            current.down();
            if (reason === "unmount") current.dispose();
            else if (reason === "escape") win.dispatchEvent(Object.assign(new Event("keydown", { cancelable: true }), { key: "Escape" }));
            else if (reason === "blur") win.dispatchEvent(new Event("blur"));
            else current.content.dispatchEvent(pointer(reason));
            current.content.dispatchEvent(pointer("pointerup"));
            await settle();
            assert.equal(current.commands.some((command) => command.type === "select-items"), false, reason);
            assert.equal(current.content.captured, false, reason);
            assert.equal(current.root.children.length, 3, reason);
            if (reason !== "unmount") current.dispose();
        }
        let resolveFocus!: () => void; // 模拟焦点请求晚于指针释放。
        current = mount(new Promise<void>((resolve) => { resolveFocus = resolve; }));
        current.down();
        current.content.dispatchEvent(pointer("pointerup"));
        current.dispose();
        resolveFocus();
        await settle();
        assert.equal(current.commands.some((command) => command.type === "select-items"), false);

        const fenceSource = readFileSync(new URL("../../tile-content/fence/FenceTile.tsx", import.meta.url), "utf8"); // 防止只测 Hook 而漏接围栏事件。
        const workspaceSource = readFileSync(new URL("../surface/DesktopWorkspace.tsx", import.meta.url), "utf8"); // 验证工作区传递真实入口。
        assert.match(fenceSource, /onPointerDown={onMarqueePointerDown}/);
        assert.match(workspaceSource, /onMarqueePointerDown={handleWorkspacePointerDown}/);
    } finally {
        hooks.deregister();
        for (const [key, descriptor] of previous) { if (descriptor) Object.defineProperty(globalThis, key, descriptor); else Reflect.deleteProperty(globalThis, key); }
    }
});

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
