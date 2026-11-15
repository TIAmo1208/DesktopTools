import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import { collectViolations, newViolations } from "./检查架构边界.mjs";

test("架构边界没有新增违规", () => {
    const baseline = JSON.parse(readFileSync(new URL("./架构边界基线.json", import.meta.url), "utf8")); // 已登记的存量依赖。
    const extra = newViolations(collectViolations(process.cwd()), baseline); // 本次扫描相对基线的新增依赖。
    assert.deepEqual(extra, { frontend: [], settings: [], rust: [] });
});
