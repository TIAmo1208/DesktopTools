import assert from "node:assert/strict";
import test from "node:test";

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
