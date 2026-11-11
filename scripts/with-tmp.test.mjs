import test from "node:test"; // Node 内置测试入口。
import assert from "node:assert/strict"; // 严格断言。
import { 临时目录, 运行环境, 组装命令 } from "./with-tmp.mjs"; // 被测纯函数。

// TMP 必须落在工作树 build/tmp，且不依赖调用方当前目录。
test("临时目录固定在 build/tmp", () => {
    assert.match(临时目录("C:/repo"), /build[\\/]tmp$/);
});

// 只覆盖 TMP，保留调用方已有环境，且不把 TEMP 当作替代。
test("运行环境只覆盖 TMP", () => {
    const 环境 = 运行环境("C:/repo", { 保留项: "1", TEMP: "C:/temp" });
    assert.equal(环境.保留项, "1");
    assert.equal(环境.TEMP, "C:/temp");
    assert.match(环境.TMP, /build[\\/]tmp$/);
});

// Windows 经 shell 执行时，含空白或引号的参数必须补引号，避免路径被拆开。
test("组装命令在 Windows 下引用含空白的参数", () => {
    assert.equal(组装命令("tauri", ["dev", "--config", "a b.json"], "win32"), 'tauri dev --config "a b.json"');
    assert.equal(组装命令("C:/Program Files/tool.exe", ["x"], "win32"), '"C:/Program Files/tool.exe" x');
});

// 非 Windows 平台保持参数原样，由 spawn 的数组形式处理。
test("组装命令在非 Windows 平台不补引号", () => {
    assert.equal(组装命令("cargo", ["a b"], "linux"), "cargo a b");
});
