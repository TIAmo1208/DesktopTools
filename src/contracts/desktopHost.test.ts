import assert from "node:assert/strict";
import test from "node:test";

import { parseDesktopHostSnapshot } from "./desktopHost.ts";

test("accepts a committed single-monitor host snapshot", () => {
    // Rust 发布的完整宿主快照载荷。
    const snapshot = parseDesktopHostSnapshot({
        phase: "committed",
        committed: true,
        hosts: [
            {
                monitor: {
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
                    scaleFactor: 1.5,
                    dpi: 144,
                },
                readiness: {
                    guardian: true,
                    page: true,
                    systemImages: true,
                    pixels: true,
                    input: true,
                    hitMap: true,
                },
            },
        ],
    });

    assert.equal(snapshot.committed, true);
    assert.equal(snapshot.hosts[0].monitor.scaleFactor, 1.5);
    assert.deepEqual(Object.values(snapshot.hosts[0].readiness), [true, true, true, true, true, true]);
});
