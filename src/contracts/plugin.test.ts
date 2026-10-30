import assert from "node:assert/strict";
import test from "node:test";

import { parsePluginInstallBatch, parsePluginInstallPlan } from "./plugin.ts";

const plan = { planId: "a", name: "甲", version: "1", targetFolder: "甲", sourceName: "甲" };

test("安装结果分开计划与错误，空结果是取消", () => {
    const batch = parsePluginInstallBatch({
        plans: [plan],
        errors: ["插件要求的程序版本过高，需要 99.0.0，当前 0.3.2", "插件清单缺少 name"],
    });
    assert.equal(batch.plans[0].planId, "a");
    assert.deepEqual(batch.errors, ["插件要求的程序版本过高，需要 99.0.0，当前 0.3.2", "插件清单缺少 name"]);
    assert.deepEqual(parsePluginInstallBatch({ plans: [], errors: [] }), { plans: [], errors: [] });
    assert.deepEqual(parsePluginInstallBatch(null), { plans: [], errors: [] });
});

test("带 errors 的对象不是非法计划，结构不对才拒绝", () => {
    assert.throws(() => parsePluginInstallPlan({ plans: [], errors: [] }));
    assert.throws(() => parsePluginInstallBatch({ plans: [], errors: "坏" }));
    assert.throws(() => parsePluginInstallBatch({ name: "甲" }));
    assert.throws(() => parsePluginInstallBatch([plan]));
});
