import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { runInNewContext } from "node:vm";

test("宿主初始化脚本在 React 加载前记录逐屏异常并去重", async () => {
    const source = readFileSync(new URL("../../src-tauri/src/platform/windows/desktop_host.rs", import.meta.url), "utf8"); // 执行实际注入脚本。
    const script = source.split('const NATIVE_BRIDGE_SCRIPT: &str = r#"')[1].split('"#;')[0]; // 原生脚本边界。
    const handlers = new Map<string, (event: any) => void>(); // 模拟浏览器事件源。
    const reports: any[] = []; // 实际发往 Rust 的诊断载荷。
    const window: any = { location: { href: "http://localhost/?monitor=1" }, addEventListener: (name: string, handler: (event: any) => void) => handlers.set(name, handler) }; // 副屏页面。
    runInNewContext(script, { window, __DESKTOP_NATIVE_ORIGIN__: "http://host-1.localhost", __REFRESH_POLICY_FALLBACK__: false, fetch: async (url: string, request: any) => {
        if (url.endsWith("record_renderer_diagnostic")) { reports.push(JSON.parse(request.body)); return {}; }
        return { json: async () => ({ ok: false, error: "门槛无效" }) };
    } });
    handlers.get("error")!({ target: { src: "http://localhost/react-missing.js" } });
    handlers.get("error")!({ target: { src: "http://localhost/react-missing.js" } });
    handlers.get("unhandledrejection")!({ reason: new Error("渲染失败") });
    await assert.rejects(window.__DESKTOP_TOOLS_NATIVE__.invoke("publish_desktop_host_readiness", { report: { monitorId: "DISPLAY2" } }));
    assert.equal(reports.length, 3);
    assert.ok(reports.every((report) => report.url.includes("monitor=1") && report.origin.includes("host-1")));
    assert.match(reports[0].message, /react-missing/);
    assert.match(reports[2].stage, /DISPLAY2/);
});
