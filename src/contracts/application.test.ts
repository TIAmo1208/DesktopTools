import assert from "node:assert/strict";
import test from "node:test";

import { ApplicationCommandError, ApplicationContractError, parseApplicationCommandError, parseApplicationSnapshot } from "./application.ts";

// 验证 command 与 event 共用的应用快照载荷。
test("accepts a valid application snapshot", () => {
    // 已解析的应用快照。
    const snapshot = parseApplicationSnapshot({ status: "active", revision: 3 });

    assert.deepEqual(snapshot, { status: "active", revision: 3 });
});

// 验证 bridge 解码 Rust 返回的结构化 command 错误。
test("decodes a structured application command error", () => {
    // 已解码的应用 command 错误。
    const error = parseApplicationCommandError({ code: "invalid-application-status", message: "应用状态参数无效" });

    assert.equal(error instanceof ApplicationCommandError, true);
    assert.equal(error.code, "invalid-application-status");
    assert.equal(error.message, "应用状态参数无效");
});

// 验证 bridge 拒绝畸形 command 错误载荷。
test("rejects an invalid application command error", () => {
    assert.throws(
        () => parseApplicationCommandError({ code: "invalid-application-status" }),
        (error: unknown) => error instanceof ApplicationContractError && error.code === "invalid-application-error",
    );
});

// 验证进程边界拒绝未知应用状态。
test("rejects an unknown application status", () => {
    assert.throws(
        () => parseApplicationSnapshot({ status: "starting", revision: 1 }),
        (error: unknown) => error instanceof ApplicationContractError && error.code === "invalid-application-snapshot",
    );
});

// 验证进程边界拒绝无效 revision。
test("rejects an invalid application revision", () => {
    assert.throws(
        () => parseApplicationSnapshot({ status: "ready", revision: -1 }),
        (error: unknown) => error instanceof ApplicationContractError && error.code === "invalid-application-snapshot",
    );
});
