import assert from "node:assert/strict";
import test from "node:test";
import { registerHooks } from "node:module";

/** 在真实 Hook 上验证输入与异步生命周期，原生窗口和视觉捕获使用最小替身。 */
test("公共格子统一锁定、准备取消、拖动回滚、释放等待与卸载", async () => {
    class Surface extends EventTarget {
        style: Record<string, string> = {}; // 临时几何与透明度。
        dataset: Record<string, string> = {}; // 公共命中标记。
        isConnected = true; // 模拟挂载状态。
        captured = false; // 当前原生指针捕获。
        control = false; // 标题中的内容控件。
        parentElement = { append() {} }; // 静态预览挂载点。
        closest() {
            return this.control ? this : null;
        }
        getBoundingClientRect() {
            return { left: 10, top: 10, width: 240, height: 180 };
        }
        setPointerCapture() {
            this.captured = true;
        }
        hasPointerCapture() {
            return this.captured;
        }
        releasePointerCapture() {
            this.captured = false;
        }
        remove() {
            this.isConnected = false;
        }
    }
    const win = new EventTarget(); // 当前宿主事件入口。
    const captures: boolean[] = []; // 验证 Escape 注册在捕获阶段。
    const add = win.addEventListener.bind(win); // 保留标准事件派发。
    win.addEventListener = (type, callback, options) => {
        if (type === "keydown") captures.push(options === true);
        add(type, callback, options);
    };
    const api = {
        effects: [] as (() => void | (() => void))[],
        captures: 0,
        previews: [] as Surface[],
        messages: [] as unknown[],
        captureThrows: false,
        resolve: undefined as undefined | ((value: unknown) => void),
    }; // 可控的原生与视觉边界。
    Object.assign(globalThis, {
        window: Object.assign(win, { innerWidth: 800, innerHeight: 600 }),
        document: { querySelectorAll: () => [] },
        Element: Surface,
        __tileTest: api,
    });
    const hooks = registerHooks({
        resolve(specifier, context, next) {
            if (specifier === "react")
                return {
                    url: "data:text/javascript,export const useRef=v=>({current:v});export const useEffect=f=>globalThis.__tileTest.effects.push(f)",
                    shortCircuit: true,
                };
            if (specifier.endsWith("/tileDragPreview"))
                return {
                    url: "data:text/javascript,export const captureTileDragVisual=()=>{const a=globalThis.__tileTest;a.captures++;if(a.captureThrows)throw Error('capture failed');return {tag:'section'};};export const createTileDragPreview=()=>{const n=new globalThis.Element();globalThis.__tileTest.previews.push(n);return n;}",
                    shortCircuit: true,
                };
            if (specifier.endsWith("/crossMonitorDrag"))
                return {
                    url: "data:text/javascript,export const publishCrossMonitorDrag=(d,p)=>{globalThis.__tileTest.messages.push({d,p});return Promise.resolve();};export const resolveCrossMonitorDrop=(d,s)=>new Promise((resolve,reject)=>{globalThis.__tileTest.resolve=resolve;s.addEventListener('abort',()=>reject(new DOMException('cancelled','AbortError')),{once:true});});",
                    shortCircuit: true,
                };
            if (specifier.endsWith("/tileGeometry") || specifier.endsWith("/workspaceDrag")) return next(specifier + ".ts", context);
            return next(specifier, context);
        },
    }); // 只替换 React 挂载和浏览器/原生边界，几何算法使用正式实现。
    const settle = async () => {
        for (let i = 0; i < 8; i++) await Promise.resolve();
    }; // 排空提交与 finally 微任务。
    const event = (type: string, extra: Record<string, unknown> = {}) =>
        Object.assign(new Event(type, { cancelable: true }), { pointerId: 1, clientX: 20, clientY: 20, ...extra }); // 当前唯一指针。
    try {
        const { useDesktopTileGeometry } = await import("./useDesktopTileGeometry.ts"); // 被测正式 Hook。
        const mount = (overrides: Record<string, unknown> = {}) => {
            api.effects = [];
            const surface = new Surface(); // 独立格子，避免跨场景共享状态。
            const result = { began: 0, ended: 0, commits: [] as any[], failures: [] as unknown[] }; // 可观察提交与释放结果。
            const monitors = [
                { id: "A", x: 0, y: 0, width: 800, height: 600, workX: 0, workY: 0, workWidth: 800, workHeight: 600, scaleFactor: 1 },
                { id: "B", x: 800, y: 0, width: 800, height: 600, workX: 800, workY: 0, workWidth: 800, workHeight: 600, scaleFactor: 1 },
            ]; // 两块相邻工作区。
            const options = {
                surfaceRef: { current: surface },
                bounds: { left: 10, top: 10, width: 240, height: 180 },
                collapsed: false,
                title: "测试格子",
                minimum: { width: 164, height: 90 },
                snap: true,
                currentMonitor: monitors[0],
                monitors,
                onFocus() {},
                async onBeginInputDrag() {
                    result.began++;
                },
                async onEndInputDrag() {
                    result.ended++;
                },
                async onCommit(bounds: unknown, monitorId: string) {
                    result.commits.push({ bounds, monitorId });
                },
                onFailure(error: unknown) {
                    result.failures.push(error);
                },
                ...overrides,
            }; // 使用与业务宿主相同的参数契约。
            const geometry = useDesktopTileGeometry(options as any); // DOM 替身仅替代平台，不改变几何状态机。
            const cleanups = api.effects.map((effect) => effect()).filter((cleanup) => typeof cleanup === "function"); // 执行正式挂载效果。
            return {
                surface,
                result,
                geometry,
                down: (direction?: any, control = false) => {
                    const target = new Surface();
                    target.control = control;
                    return geometry.handlePointerDown(
                        { button: 0, pointerId: 1, clientX: 20, clientY: 20, target, currentTarget: surface, preventDefault() {}, stopPropagation() {} } as any,
                        direction,
                    );
                },
                dispose: () => cleanups.forEach((cleanup) => cleanup!()),
            };
        };
        let current = mount({ locked: true }); // 锁定不得进入输入捕获或键盘提交。
        await current.down();
        current.geometry.handleKeyDown({ key: "ArrowRight", target: current.surface, currentTarget: current.surface } as any);
        assert.equal(current.result.began, 0);
        assert.equal(current.result.commits.length, 0);
        current.dispose();

        current = mount(); // 标题按钮不触发移动。
        await current.down(undefined, true);
        assert.equal(current.result.began, 0);
        const idleEscape = event("keydown", { key: "Escape" }); // 空闲时保留内容自己的 Escape。
        win.dispatchEvent(idleEscape);
        assert.equal(idleEscape.defaultPrevented, false);
        current.dispose();

        let ready!: () => void; // 输入建立的受控结果。
        current = mount({
            onBeginInputDrag: () =>
                new Promise<void>((resolve) => {
                    ready = resolve;
                }),
        });
        const preparing = current.down(); // 取消发生在原生输入尚未建立时。
        const pendingEscape = event("keydown", { key: "Escape" });
        win.dispatchEvent(pendingEscape);
        assert.equal(pendingEscape.defaultPrevented, true);
        ready();
        await preparing;
        assert.equal(current.surface.captured, false);
        assert.equal(current.result.ended, 1);
        assert.equal(current.result.commits.length, 0);
        current.dispose();

        current = mount(); // 缩放使用真实表面，Escape 恢复边界。
        const beforeResize = api.captures;
        await current.down("se");
        current.surface.dispatchEvent(event("pointermove", { clientX: 100, clientY: 100 }));
        assert.equal(current.surface.style.width, "320px");
        win.dispatchEvent(event("keydown", { key: "Escape" }));
        current.surface.dispatchEvent(event("pointerup"));
        await settle();
        assert.equal(current.surface.style.width, "240px");
        assert.equal(current.result.ended, 1);
        assert.equal(current.result.commits.length, 0);
        assert.equal(api.captures, beforeResize);
        current.dispose();

        current = mount(); // 多个移动帧只捕获一次，正常释放只提交一次。
        const beforeMove = api.captures;
        await current.down();
        current.surface.dispatchEvent(event("pointermove", { clientX: 100, clientY: 100 }));
        current.surface.dispatchEvent(event("pointermove", { clientX: 200, clientY: 100 }));
        current.surface.dispatchEvent(event("pointerup", { clientX: 200, clientY: 100 }));
        current.surface.dispatchEvent(event("pointerup", { clientX: 200, clientY: 100 }));
        await settle();
        assert.equal(api.captures - beforeMove, 1);
        assert.equal(current.result.commits.length, 1);
        assert.equal(current.result.ended, 1);
        assert.equal(current.surface.dataset.crossDragHidden, undefined);
        assert.equal(api.previews.at(-1)?.isConnected, false);
        current.dispose();

        current = mount({ collapsed: true, collapsedSize: { height: 56 } }); // 折叠外观不覆盖保存的展开尺寸。
        await current.down("se");
        assert.equal(current.result.began, 0);
        await current.down();
        current.surface.dispatchEvent(event("pointermove", { clientX: 100, clientY: 100 }));
        assert.equal(api.previews.at(-1)?.style.height, "56px");
        current.surface.dispatchEvent(event("pointerup", { clientX: 100, clientY: 100 }));
        await settle();
        assert.equal(current.result.commits[0].bounds.height, 180);
        current.dispose();

        for (const cancel of ["escape", "blur", "unmount"]) {
            current = mount(); // 释放等待中的所有取消入口拒绝迟到领域提交。
            await current.down();
            current.surface.dispatchEvent(event("pointermove", { clientX: 850 }));
            current.surface.dispatchEvent(event("pointerup", { clientX: 850 }));
            assert.equal(current.result.commits.length, 0);
            if (cancel === "unmount") current.dispose();
            else win.dispatchEvent(event(cancel === "escape" ? "keydown" : "blur", { key: "Escape" }));
            await settle();
            assert.equal(current.result.ended, 1, cancel);
            assert.equal(current.surface.dataset.crossDragHidden, undefined, cancel);
            api.resolve?.({ snapTargets: [] });
            await settle();
            assert.equal(current.result.commits.length, 0, cancel);
            if (cancel !== "unmount") current.dispose();
        }
        current = mount({ minimum: undefined }); // 省略覆盖后，真实指针缩放使用公共底线。
        await current.down("se");
        current.surface.dispatchEvent(event("pointermove", { clientX: -1000, clientY: -1000 }));
        current.surface.dispatchEvent(event("pointerup", { clientX: -1000, clientY: -1000 }));
        await settle();
        assert.equal(current.result.commits[0].bounds.width, 80);
        assert.equal(current.result.commits[0].bounds.height, 48);
        current.dispose();

        current = mount({ bounds: { left: 0, top: 0, width: 40, height: 24 } }); // 极小投射移动提交恢复持久底线。
        current.geometry.handleKeyDown({
            key: "ArrowRight",
            target: current.surface,
            currentTarget: current.surface,
            preventDefault() {},
            stopPropagation() {},
        } as any);
        await settle();
        assert.equal(current.result.commits[0].bounds.width, 80);
        assert.equal(current.result.commits[0].bounds.height, 48);
        current.dispose();

        current = mount(); // 捕获失败不留下输入或未处理异常。
        api.captureThrows = true;
        await current.down();
        assert.equal(current.result.began, 0);
        assert.equal(current.result.failures.length, 1);
        current.dispose();
        assert.ok(captures.length > 0 && captures.every(Boolean));
    } finally {
        hooks.deregister();
        for (const key of ["window", "document", "Element", "__tileTest"]) delete (globalThis as any)[key];
    }
});
