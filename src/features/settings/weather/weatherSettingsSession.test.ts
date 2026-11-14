import assert from "node:assert/strict";
import test from "node:test";

import { createWeatherOperationGate, type WeatherSessionPorts } from "./weatherSettingsSession.ts";

/** 记录取消调用的假端口，begin 默认立即成功。 */
const ports = (begin: WeatherSessionPorts["begin"], canceled: number[] = []) => ({
    begin,
    cancel: async (sessionId: number) => {
        canceled.push(sessionId);
    },
});

test("同一操作闸只为一次会话创建一次", async () => {
    let begins = 0;
    const gate = createWeatherOperationGate(
        ports(async () => {
            begins++;
            return 3;
        }),
    );
    const identity = gate.nextIdentity();
    const credentials = { apiKey: "k" };
    assert.equal(await gate.ensure(credentials, () => true, identity), 3);
    assert.equal(await gate.ensure(credentials, () => true, identity), 3);
    assert.equal(begins, 1);
    assert.equal(gate.sessionId, 3);
});

test("身份作废后创建的会话立即取消", async () => {
    const canceled: number[] = [];
    let settle: (value: number | null) => void = () => undefined;
    const gate = createWeatherOperationGate(
        ports(
            () =>
                new Promise<number | null>((resolve) => {
                    settle = resolve;
                }),
            canceled,
        ),
    );
    const pending = gate.ensure({ apiKey: "k" }, () => true, gate.nextIdentity());
    gate.invalidate();
    settle(7);
    await assert.rejects(pending, /操作已取消/);
    assert.deepEqual(canceled, [7]);
    assert.equal(gate.sessionId, null);
});

test("页面卸载后创建的会话立即取消", async () => {
    const canceled: number[] = [];
    const gate = createWeatherOperationGate(ports(async () => 9, canceled));
    await assert.rejects(gate.ensure({ apiKey: "k" }, () => false, gate.nextIdentity()), /操作已取消/);
    assert.deepEqual(canceled, [9]);
    assert.equal(gate.sessionId, null);
});

test("作废递增身份并取消已有会话，重复作废不重复取消", async () => {
    const canceled: number[] = [];
    const gate = createWeatherOperationGate(ports(async () => 5, canceled));
    const identity = gate.nextIdentity();
    await gate.ensure({ apiKey: "k" }, () => true, identity);
    gate.invalidate();
    assert.deepEqual(canceled, [5]);
    assert.equal(gate.sessionId, null);
    assert.equal(gate.isCurrent(identity), false);
    gate.invalidate();
    assert.deepEqual(canceled, [5]);
});

test("没有会话时作废不触发取消", () => {
    const canceled: number[] = [];
    const gate = createWeatherOperationGate(ports(async () => 1, canceled));
    gate.invalidate();
    assert.deepEqual(canceled, []);
});

test("身份只认最新一次操作", () => {
    const gate = createWeatherOperationGate(ports(async () => 1));
    const first = gate.nextIdentity();
    const second = gate.nextIdentity();
    assert.equal(gate.isCurrent(first), false);
    assert.equal(gate.isCurrent(second), true);
});
