import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import { collectViolations, newViolations } from "./检查文档规则.mjs";

test("文档规则没有新增违规", () => {
    const baseline = JSON.parse(readFileSync(new URL("./文档规则基线.json", import.meta.url), "utf8")); // 已登记的存量违规。
    const extra = newViolations(collectViolations(process.cwd()), baseline); // 本次扫描相对基线的新增违规。
    assert.deepEqual(extra, {});
});
