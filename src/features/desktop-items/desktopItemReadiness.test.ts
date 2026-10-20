import assert from "node:assert/strict";
import test from "node:test";
import { registerDesktopItemImage, waitForDesktopItemImages } from "./desktopItemReadiness.ts";

/** 图像状态来自已注册项目，卸载与失败不会污染下一次接管计数。 */
test("desktop readiness counts mounted decoded images and releases stale nodes", async () => {
    assert.deepEqual(await waitForDesktopItemImages(), { total: 0, loaded: 0 });
    let finish: (() => void) | undefined; // 控制解码完成前的卸载场景。
    const image = { naturalWidth: 0, decode: () => new Promise<void>((resolve) => { finish = resolve; }) } as HTMLImageElement; // 无 DOM 查询的最小图像事实。
    const release = registerDesktopItemImage(image)!; // 与 React ref 相同的注册入口。
    const pending = waitForDesktopItemImages(); // 解码尚未完成。
    release();
    finish!();
    assert.deepEqual(await pending, { total: 0, loaded: 0 });
    const ready = registerDesktopItemImage({ naturalWidth: 48, decode: async () => {} } as HTMLImageElement)!; // 有效系统图像。
    try {
        assert.deepEqual(await waitForDesktopItemImages(), { total: 1, loaded: 1 });
    } finally {
        ready();
    }
    assert.deepEqual(await waitForDesktopItemImages(), { total: 0, loaded: 0 });
});
