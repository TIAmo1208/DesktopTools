import test from "node:test"; // Node 内置测试入口。
import assert from "node:assert/strict"; // 严格断言。
import { 影响文档 } from "./文档影响.mjs"; // 被测纯函数。

// 原生改动提示 CONTEXT、Rust README 与 ADR。
test("Rust 源码改动提示事实文档", () => {
    const 文档 = 影响文档(["src-tauri/src/lib.rs"]);
    assert.ok(文档.includes("CONTEXT.md"));
    assert.ok(文档.includes("src-tauri/README.md"));
    assert.ok(文档.includes("docs/adr/"));
});

// 浮岛改动提示设计文档与对应 ADR，并去重。
test("浮岛改动提示设计文档且去重", () => {
    const 文档 = 影响文档(["src/features/island/island.tsx", "src/features/island/island.css"]);
    assert.ok(文档.includes("docs/design/浮岛.md"));
    assert.equal(new Set(文档).size, 文档.length);
});

// 没有匹配规则时返回空清单，由人按改动内容判断。
test("未知路径不硬猜文档", () => {
    assert.deepEqual(影响文档(["README.md"]), []);
});
