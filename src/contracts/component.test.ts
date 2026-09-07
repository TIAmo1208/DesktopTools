import assert from "node:assert/strict";
import test from "node:test";

import { parseComponentHostEvent, parseComponentInstanceSnapshot, parseComponentRuntimeSnapshot } from "./component.ts";

// 验证官方清单、授权能力和实例隔离状态通过公开运行时快照。
test("parses the official component runtime snapshot", () => {
    const snapshot = parseComponentRuntimeSnapshot({
        revision: 2,
        manifests: [
            {
                id: "official.local-clock",
                name: "本地时间",
                entry: "official:local-clock",
                requiredCapabilities: ["read-host-time"],
                persistenceVersion: 1,
            },
        ],
        instances: [
            {
                instanceId: "official.local-clock.default",
                componentId: "official.local-clock",
                entry: "official:local-clock",
                authorizedCapabilities: ["read-host-time"],
                status: "ready",
                error: null,
            },
            {
                instanceId: "missing",
                componentId: "official.missing",
                entry: null,
                authorizedCapabilities: [],
                status: "failed",
                error: "组件清单不存在",
            },
        ],
        failures: [],
    });

    assert.equal(snapshot.manifests[0].entry, "official:local-clock");
    assert.equal(snapshot.instances[0].authorizedCapabilities[0], "read-host-time");
    assert.equal(snapshot.instances[1].status, "failed");
});

// 验证组件位置、显示器投射和版本化状态通过持久实例契约。
test("parses a persisted component instance and host event", () => {
    const instance = parseComponentInstanceSnapshot({
        instanceId: "official.local-clock.default",
        componentId: "official.local-clock",
        monitorId: "DISPLAY2",
        projectionMonitorId: "DISPLAY1",
        projectionBounds: { left: 120, top: 80, width: 220, height: 116 },
        bounds: { left: 1480, top: 820, width: 220, height: 116 },
        stateVersion: 1,
        state: { format: "24-hour" },
    });
    const event = parseComponentHostEvent({ sequence: 4, kind: "time-tick", unixMillis: 1_788_198_400_000 });

    assert.equal(instance.projectionMonitorId, "DISPLAY1");
    assert.equal(event.sequence, 4);
});

// 验证未知能力不会越过前端契约边界。
test("rejects an unknown component capability", () => {
    assert.throws(
        () =>
            parseComponentRuntimeSnapshot({
                revision: 1,
                manifests: [],
                instances: [
                    {
                        instanceId: "clock",
                        componentId: "official.local-clock",
                        entry: "official:local-clock",
                        authorizedCapabilities: ["read-files"],
                        status: "ready",
                        error: null,
                    },
                ],
                failures: [],
            }),
        { name: "ComponentContractError", message: "组件运行时数据无效" },
    );
});
