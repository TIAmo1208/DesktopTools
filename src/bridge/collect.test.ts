import assert from "node:assert/strict";
import test from "node:test";
import { mergeCollectItems } from "../features/tile-content/collect-box/collectItemState.ts";
import type { CollectItem } from "./collect.ts";

test("应用内重命名清除所有格子的旧身份，监听先行、迟到读取与暂停不留下不可用副本", async () => {
    const { readFile } = await import("node:fs/promises"); // 读取真实命令事件和列表刷新入口。
    const { runInNewContext } = await import("node:vm"); // 沿用现有无浏览器回归执行方式。
    const { stripTypeScriptTypes } = await import("node:module"); // 执行生产 TypeScript 函数。
    const commands = await readFile(new URL("../../src-tauri/src/commands/collect.rs", import.meta.url), "utf8"); // 共享后端入口。
    const rename = commands.slice(commands.indexOf("pub(crate) fn execute_rename_collect_item"), commands.indexOf("pub fn set_collect_item_pinned")); // 改名成功通知。
    const event = rename.match(/app\.emit\("collect-items-changed", serde_json::json!\((\{[^}]*\})\)\)/); // 提取真实载荷，普通刷新没有旧身份。
    assert.ok(event || rename.includes("changed(&app)"), "重命名必须发送列表变化事件");
    const old: CollectItem = {
        name: "旧名称.md",
        kind: "text",
        displayName: "旧名称",
        path: null,
        content: "正文",
        createdUnixMs: 1,
        pinned: true,
        available: true,
    }; // 会话原条目。
    const next = { ...old, name: "新名称.md", displayName: "新名称" }; // 改名后的磁盘扫描结果。
    const payload = event ? JSON.parse(event[1].replace(/\bold_name\b/g, JSON.stringify(old.name))) : null; // 重放后端事件内容。
    const bridge = await readFile(new URL("./collect.ts", import.meta.url), "utf8"); // 真实事件适配器。
    const listener = bridge.slice(bridge.indexOf("export const listenCollectChanges")).replace("export const", "const"); // 只执行订阅入口。
    const source = await readFile(new URL("../features/tile-content/collect-box/CollectBox.tsx", import.meta.url), "utf8"); // 真实刷新与版本校验。
    const reload = source.match(/const reload = \(deletedName\?: string\) => \{[\s\S]*?\n    \};/)?.[0]; // 刷新不含 JSX。
    assert.ok(reload);
    for (const watcherFirst of [false, true]) {
        for (const paused of [false, true]) {
            let items = watcherFirst ? mergeCollectItems([old], [next]) : [old]; // 模拟多个格子的不同读取进度。
            let refreshPaused = false; // 先开始旧读取，再模拟暂停。
            const reads: Array<(items: CollectItem[]) => void> = []; // 控制扫描结果返回顺序。
            const listeners = new Map<string, (payload: unknown) => void>(); // 真实桥接订阅。
            const context: Record<string, any> = {
                readRevision: { current: 0 },
                readyRef: { current: true },
                mergeCollectItems,
                window: { __DESKTOP_TOOLS_NATIVE__: { isRefreshPaused: () => refreshPaused } },
                setItems: (update: (current: CollectItem[]) => CollectItem[]) => {
                    items = update(items);
                },
                setReadError() {},
                setReady() {},
                handleFailure(error: unknown) {
                    throw error;
                },
                getCollectItems: () => new Promise<CollectItem[]>((resolve) => reads.push(resolve)),
                listenRuntime: async (name: string, callback: (payload: unknown) => void) => {
                    listeners.set(name, callback);
                    return () => {};
                },
            }; // 使用实际刷新逻辑，仅替代 I/O 和 React 状态写入。
            runInNewContext(
                `${stripTypeScriptTypes(listener)}\n${stripTypeScriptTypes(reload)}\nglobalThis.reload = reload; globalThis.listen = listenCollectChanges;`,
                context,
            );
            await context.listen(context.reload, context.handleFailure);
            context.reload();
            refreshPaused = paused;
            listeners.get("collect-items-changed")!(payload);
            if (!paused) reads[1]([next]);
            reads[0]([old]);
            await Promise.resolve();
            assert.ok(!items.some((item) => item.name === old.name), "应用内改名后旧名称不得残留或被迟到读取恢复");
            refreshPaused = false;
            context.reload();
            reads.at(-1)!([next]);
            await Promise.resolve();
            assert.deepEqual(
                items.map((item) => [item.name, item.available, item.pinned]),
                [[next.name, true, true]],
            );
        }
    }
});

test("GIF 仅悬停读取原图，关闭拒绝迟到图像", async () => {
    const { readFile } = await import("node:fs/promises");
    const { runInNewContext } = await import("node:vm");
    const { stripTypeScriptTypes } = await import("node:module");
    const source = await readFile(new URL("../features/tile-content/collect-box/CollectBox.tsx", import.meta.url), "utf8");
    const effect = source.slice(source.indexOf("    useEffect(() => {"), source.indexOf("\n    return (", source.indexOf("const CollectImage =")));
    for (const [path, enlarged, expected] of [
        ["image.GIF", true, undefined],
        ["image.gif", false, 384],
        ["image.png", true, 768],
    ] as const) {
        let intersect: (entries: unknown[]) => void = () => {};
        let dispose = () => {};
        let resolve: (value: string) => void = () => {};
        const requests: unknown[] = [];
        const images: unknown[] = [];
        runInNewContext(stripTypeScriptTypes(effect), {
            item: { name: path, path, available: true },
            collapsed: false,
            enlarged,
            container: { current: { getBoundingClientRect: () => ({ width: 520, height: 420 }) } },
            window: { devicePixelRatio: 2, addEventListener() {}, removeEventListener() {} },
            useEffect: (callback: () => () => void) => {
                dispose = callback();
            },
            setSource: (value: unknown) => images.push(value),
            setFailed() {},
            handleFailure() {},
            readCollectImage: (_name: string, _path: string, pixels: unknown) => {
                requests.push(pixels);
                return new Promise<string>((done) => {
                    resolve = done;
                });
            },
            IntersectionObserver: class {
                constructor(callback: typeof intersect) {
                    intersect = callback;
                }
                observe() {}
                disconnect() {}
            },
            ResizeObserver: class {
                observe() {}
                disconnect() {}
            },
        });
        intersect([{ isIntersecting: true }]);
        intersect([{ isIntersecting: true }]);
        assert.deepEqual(requests, [expected]);
        dispose();
        resolve("late-image");
        await Promise.resolve();
        assert.ok(!images.includes("late-image"));
    }
});

test("编辑窗清理只在成功保存关闭或明确放弃后执行", async () => {
    const fs = await import("node:fs/promises");
    const { runInNewContext } = await import("node:vm");
    const source = await fs.readFile(new URL("../features/tile-content/collect-box/CollectEditor.tsx", import.meta.url), "utf8");
    const save = source.match(/const handleSave = async \(close = false\) => \{[\s\S]*?\n    \};/)?.[0];
    const discard = source.match(/const handleDiscard = async \(\) => \{[\s\S]*?\n    \};/)?.[0];
    assert.ok(save && discard);
    const calls: string[] = [];
    let failSave = false;
    let failCleanup = false;
    const instance = { disabled() {}, enable() {}, getValue: () => "正文" };
    const context: Record<string, any> = {
        editor: { current: instance },
        readyRef: { current: true },
        pending: { current: false },
        currentName: { current: "note.md" },
        dirty: { current: true },
        savedThisSession: { current: false },
        allowClose: { current: false },
        setBusy() {},
        setOperation() {},
        setSaved() {},
        setFailure() {},
        setCloseFailure() {},
        handleFailure: () => calls.push("error"),
        saveCollectItem: async () => {
            calls.push("save");
            if (failSave) throw Error("保存失败");
        },
        cleanupCollectResources: async (_name: string, discarded: boolean) => {
            calls.push(`cleanup:${discarded}`);
            if (failCleanup) throw Error("清理失败");
        },
        getCurrentWindow: () => ({ destroy: async () => calls.push("destroy") }),
    };
    runInNewContext(`${save}\n${discard}\nglobalThis.save = handleSave; globalThis.discard = handleDiscard;`, context);
    await context.save(false);
    assert.deepEqual(calls.splice(0), ["save"]);
    failSave = true;
    await context.save(true);
    assert.deepEqual(calls.splice(0), ["save", "error"]);
    failSave = false;
    failCleanup = true;
    await context.save(true);
    assert.deepEqual(calls.splice(0), ["save", "cleanup:false", "error", "destroy"]);
    failCleanup = false;
    context.dirty.current = true;
    await context.discard();
    assert.deepEqual(calls.splice(0), ["cleanup:true", "destroy"]);
    context.readyRef.current = false;
    await context.discard();
    assert.deepEqual(calls.splice(0), ["destroy"]);
    context.pending.current = true;
    await context.save(true);
    await context.discard();
    assert.deepEqual(calls, []);
});

test("收集项打开共用失败刷新入口", async () => {
    const { readFile } = await import("node:fs/promises");
    const native = await readFile(new URL("../../src-tauri/src/commands/mod.rs", import.meta.url), "utf8");
    const commands = await readFile(new URL("../../src-tauri/src/commands/collect.rs", import.meta.url), "utf8");
    assert.match(native, /"open_collect_item"\s*=>\s*\{[^}]*collect::execute_open_collect_item\(app, name\)/);
    assert.match(commands, /spawn_blocking\(move \|\| execute_open_collect_item\(&app, name\)\)/);
    assert.match(commands, /if let Err\(error\) = &result\s*\{\s*changed\(app\)/);
});

test("收集箱图片与拖动命令同时接入 Tauri 和独立宿主", async () => {
    const { readFile } = await import("node:fs/promises"); // 核对两个真实 transport 的注册，避免替代桥隐藏路由遗漏。
    const native = await readFile(new URL("../../src-tauri/src/commands/mod.rs", import.meta.url), "utf8");
    const tauri = await readFile(new URL("../../src-tauri/src/lib.rs", import.meta.url), "utf8");
    for (const command of ["read_collect_image", "prepare_collect_drag"]) {
        assert.match(native, new RegExp(`"${command}"\\s*=>`), `独立宿主缺少 ${command}`);
        assert.ok(tauri.includes(`::${command},`), `Tauri 缺少 ${command}`);
    }
});

test("收集箱第二个监听失败时释放第一个监听", async () => {
    const source = await import("node:fs/promises").then((fs) => fs.readFile(new URL("./collect.ts", import.meta.url), "utf8"));
    const stops: string[] = [];
    const listeners: Array<(payload?: unknown) => void> = [];
    const onError = { value: undefined as unknown };
    const moduleSource = source
        .replace(/^import .*;\r?$/gm, "")
        .replace(/^export /gm, "")
        .replace(/export interface CollectItem \{[\s\S]*?\}\r?\n/, "")
        .replace(/export const /g, "const ");
    const context = {
        Promise,
        invokeRuntime: async () => undefined,
        listenRuntime: async (event: string, callback: (payload?: unknown) => void) => {
            if (event === "folder-portal-changed") throw new Error("第二监听失败");
            listeners.push(callback);
            return () => stops.push(event);
        },
    } as Record<string, unknown>;
    const { runInNewContext } = await import("node:vm");
    const { stripTypeScriptTypes } = await import("node:module");
    runInNewContext(`${stripTypeScriptTypes(moduleSource)}\nglobalThis.listen = listenCollectChanges;`, context);
    await (context.listen as (onChange: () => void, onError: (error: unknown) => void) => Promise<unknown>)(
        () => undefined,
        (error) => (onError.value = error),
    );
    assert.deepEqual(stops, ["collect-items-changed"]);
    assert.ok(onError.value instanceof Error);
    assert.equal(listeners.length, 1);
});
