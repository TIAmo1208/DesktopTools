import assert from "node:assert/strict";
import test from "node:test";
import { registerHooks } from "node:module";

/** 连续使用同一投放适配器，验证每帧替换预览及会话滚动恢复。 */
test("drop preview replaces old markers and cancels stale scroll frames", async () => {
    const nodes: { style: Record<string, string>; remove: () => void }[] = []; // 当前挂载的预览节点。
    const frames = new Map<number, FrameRequestCallback>(); // 尚未执行的自动滚动任务。
    let frameId = 0; // 帧任务唯一编号。
    let fenceId = "first"; // 当前指针命中的围栏。
    const bounds = { left: 0, top: 0, right: 240, bottom: 300, width: 240, height: 300 }; // 内容视口。
    const item = {
        dataset: { itemId: "item" },
        getBoundingClientRect: () => ({ left: 10, top: 100, right: 80, bottom: 190, width: 70, height: 90 }),
    }; // 命中的实际项目。
    const contents = ["first", "second"].map((id) => ({
        dataset: { fenceContentId: id, dragScrolling: undefined as string | undefined },
        scrollTop: 20,
        getClientRects: () => [bounds],
        getBoundingClientRect: () => bounds,
        contains: (node: unknown) => node === item,
    })); // 两个围栏独立保存滚动状态。
    const classes = contents.map(() => new Set<string>()); // 各围栏的高亮类名。
    const tiles = contents.map((content, index) => ({
        dataset: { fenceId: content.dataset.fenceContentId },
        classList: {
            add: (value: string) => classes[index].add(value),
            remove: (value: string) => classes[index].delete(value),
            has: (value: string) => classes[index].has(value),
        },
        querySelector: () => content,
    })); // 目标高亮与内容归属。
    const hit = { closest: (selector: string) => (selector === "[data-item-id]" ? item : tiles.find((tile) => tile.dataset.fenceId === fenceId)) }; // 模拟指针命中。
    const globals = {
        window: { innerWidth: 1000, innerHeight: 1000 },
        document: {
            elementsFromPoint: () => [hit],
            querySelectorAll: (selector: string) =>
                selector === ".fence--remote-drop-target" ? tiles.filter((tile) => tile.classList.has("fence--remote-drop-target")) : [],
            createElement: () => ({
                style: {},
                remove() {
                    nodes.splice(nodes.indexOf(this), 1);
                },
            }),
            body: { append: (node: (typeof nodes)[number]) => nodes.push(node) },
        },
        requestAnimationFrame: (callback: FrameRequestCallback) => {
            frames.set(++frameId, callback);
            return frameId;
        },
        cancelAnimationFrame: (id: number) => {
            frames.delete(id);
        },
    }; // 只替换 DOM 与帧调度，运行真实命中和插入计算。
    const descriptors = Object.fromEntries(Object.keys(globals).map((key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)])); // 恢复原全局环境。
    Object.assign(globalThis, globals);
    const hooks = registerHooks({
        resolve(specifier, context, next) {
            return next(specifier.startsWith(".") && !specifier.endsWith(".ts") ? `${specifier}.ts` : specifier, context);
        },
    }); // 与现有 Node 测试一致，直接加载 TypeScript 源码。
    try {
        const { createWorkspaceDropTarget } = await import("./workspaceDropTarget.ts"); // 真实投放适配器。
        const target = createWorkspaceDropTarget("screen", () => {}); // 连续帧共用一个实例。
        const workspace = {
            fences: tiles.map((tile) => ({ id: tile.dataset.fenceId, members: ["item"], layoutMode: "tile", sortMode: "none" })),
        } as Parameters<typeof target.update>[1]; // 围栏插入所需快照。
        const drag = { kind: "items", phase: "move", source: "ole", itemIds: ["other"], pointer: { left: 11, top: 150 } } as Parameters<
            typeof target.update
        >[0]; // 同一次拖动输入。
        for (const [left, expected] of [
            [11, "10px"],
            [79, "77px"],
            [11, "10px"],
        ] as const) {
            target.update({ ...drag, pointer: { left, top: 150 } }, workspace);
            assert.equal(nodes.length, 1);
            assert.equal(nodes[0].style.left, expected);
        }
        target.update({ ...drag, pointer: { left: 45, top: 150 } }, workspace);
        assert.equal(nodes.length, 0);
        target.update(drag, workspace);
        fenceId = "second";
        target.update(drag, workspace);
        assert.equal(nodes.length, 1);
        assert.equal(tiles[0].classList.has("fence--remote-drop-target"), false);
        assert.equal(tiles[1].classList.has("fence--remote-drop-target"), true);
        for (let index = 0; index < 3; index++) target.update({ ...drag, pointer: { left: 11, top: 299 } }, workspace);
        assert.equal(frames.size, 1);
        contents[1].scrollTop = 60;
        target.update(drag, workspace);
        assert.equal(frames.size, 0);
        assert.equal(contents[1].scrollTop, 60);
        target.clear(true, true);
        assert.equal(nodes.length, 0);
        assert.equal(contents[1].scrollTop, 20);
        assert.ok(contents.every((content) => content.dataset.dragScrolling === undefined));
        assert.ok(tiles.every((tile) => !tile.classList.has("fence--remote-drop-target")));
    } finally {
        hooks.deregister();
        for (const [key, descriptor] of Object.entries(descriptors)) {
            if (descriptor) Object.defineProperty(globalThis, key, descriptor);
            else Reflect.deleteProperty(globalThis, key);
        }
    }
});
