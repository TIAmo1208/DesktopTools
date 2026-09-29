import assert from "node:assert/strict";
import test from "node:test";
import { registerHooks } from "node:module";

/** 目标预览消费正式会话与几何逻辑，DOM 只记录节点创建和回收。 */
test("跨屏目标复用静态节点，结束后迟到消息不恢复预览", async () => {
    const nodes: { style: Record<string, string>; remove: () => void; removed: boolean }[] = []; // 已创建的视觉节点。
    const effects: (() => void | (() => void))[] = []; // 正式 Hook 挂载入口。
    const timers = new Map<number, () => void>(); // 可控过期时间，不引入等待。
    let nextTimer = 0; // 唯一计时器身份。
    const events = new EventTarget(); // 目标页面事件入口。
    const create = () => {
        const node = {
            style: {},
            removed: false,
            remove() {
                this.removed = true;
            },
        };
        nodes.push(node);
        return node;
    }; // 最小 DOM 替身。
    Object.assign(globalThis, {
        window: Object.assign(events, {
            innerWidth: 800,
            innerHeight: 600,
            setTimeout: (fn: () => void) => {
                timers.set(++nextTimer, fn);
                return nextTimer;
            },
            clearTimeout: (id: number) => timers.delete(id),
        }),
        document: { querySelector: () => ({ append() {} }) },
        cancelAnimationFrame() {},
        __previewTest: { create, effects },
    });
    const hooks = registerHooks({
        resolve(specifier, context, next) {
            if (specifier === "react")
                return {
                    url: "data:text/javascript,export const useRef=v=>({current:v});export const useEffect=f=>globalThis.__previewTest.effects.push(f)",
                    shortCircuit: true,
                };
            if (specifier.endsWith("/tileDragPreview"))
                return { url: "data:text/javascript,export const createTileDragPreview=()=>globalThis.__previewTest.create()", shortCircuit: true };
            if (specifier.endsWith("/desktopItemPreview"))
                return { url: "data:text/javascript,export const createDesktopItemPreview=()=>{throw Error('unexpected OLE preview')}", shortCircuit: true };
            if (specifier.endsWith("/workspaceDropTarget"))
                return {
                    url: "data:text/javascript,export const createWorkspaceDropTarget=()=>({clear(){},update(){return {drop:{snapTargets:[]},regions:[],fenceScrolls:[]}}})",
                    shortCircuit: true,
                };
            if (specifier.endsWith("bridge/runtime")) return { url: "data:text/javascript,export const invokeRuntime=async()=>{}", shortCircuit: true };
            if (specifier.endsWith("/tileGeometry") || specifier.endsWith("/crossMonitorDragPreview")) return next(specifier + ".ts", context);
            return next(specifier, context);
        },
    }); // 页面与传输边界之外的逻辑全部使用正式实现。
    try {
        const { createCrossMonitorDragPreview } = await import("./crossMonitorDragPreview.ts");
        const visual = { title: "收集箱", content: { tag: "section" }, bounds: { left: 10, top: 10, width: 240, height: 56 } }; // 有界静态帧。
        const drag = {
            phase: "move",
            session: "first",
            source: "A",
            target: "B",
            pointer: { left: 40, top: 40 },
            origin: { left: 20, top: 20 },
            delta: { left: 20, top: 20 },
            kind: "fence" as const,
            visuals: [visual],
            itemIds: [],
        }; // 目标屏首次消息。
        const preview = createCrossMonitorDragPreview(); // 正式目标预览控制器。
        preview.update(drag, undefined, undefined);
        preview.update({ ...drag, pointer: { left: 50, top: 50 }, visuals: [{ ...visual, content: undefined }] }, undefined, undefined);
        assert.equal(nodes.length, 1);
        assert.equal(nodes[0].style.left, "40px");
        assert.equal(nodes[0].style.height, "56px");
        preview.update({ ...drag, phase: "drop", visuals: [] }, undefined, undefined);
        assert.equal(nodes[0].removed, false);
        preview.update({ ...drag, session: "missing", visuals: [{ ...visual, content: undefined }] }, undefined, undefined);
        assert.equal(nodes.length, 1);
        preview.update({ ...drag, session: "second" }, undefined, undefined);
        assert.equal(nodes.length, 2);
        assert.equal(nodes[0].removed, true);
        preview.clear();
        assert.equal(nodes[1].removed, true);

        const { useCrossMonitorPreview } = await import("./useCrossMonitorPreview.ts");
        useCrossMonitorPreview("B", undefined, undefined);
        const cleanups = effects.map((effect) => effect()).filter(Boolean); // 建立目标会话监听。
        const deliver = (detail: unknown) => events.dispatchEvent(new CustomEvent("desktop-tools-drag", { detail })); // 实际页面事件。
        deliver(drag);
        const count = nodes.length;
        deliver({ ...drag, phase: "end" });
        deliver(drag);
        assert.equal(nodes.length, count);
        assert.equal(nodes.at(-1)?.removed, true);
        assert.equal(timers.size, 0);
        deliver({ ...drag, session: "third" });
        assert.equal(nodes.length, count + 1);
        for (const cleanup of cleanups) cleanup?.();
        assert.equal(nodes.at(-1)?.removed, true);
        assert.equal(timers.size, 0);
    } finally {
        hooks.deregister();
        for (const name of ["window", "document", "cancelAnimationFrame", "__previewTest"]) delete (globalThis as any)[name];
    }
});
