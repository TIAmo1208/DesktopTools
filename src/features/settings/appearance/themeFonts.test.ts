import assert from "node:assert/strict";
import test from "node:test";

import { interfaceFontStack, themeFontOptions } from "./themeFonts.ts";

/** 只带字体声明的清单投影，字段名与运行时契约一致。 */
const manifests = (fonts: { family: string; file: string }[], version = "1.0.0") =>
    [
        {
            id: "plugin.theme-songlv",
            name: "主题-宋律",
            entry: "plugin:slot",
            script: "",
            role: "theme",
            stylesheet: "主题-宋律/theme.css",
            version,
            requiredCapabilities: [],
            creatable: false,
            removable: true,
            maxTiles: 0,
            persistenceVersion: 1,
            themeFonts: fonts.map((font) => ({ weight: 400, style: "normal", ...font })),
        },
    ] as unknown as Parameters<typeof themeFontOptions>[0];

test("字体栈按选中字体在前、系统回退在后拼接", () => {
    assert.equal(interfaceFontStack("宋律宋体"), '"宋律宋体", "Segoe UI Variable", "Segoe UI", sans-serif');
    assert.equal(interfaceFontStack("Arial"), '"Arial", "Segoe UI Variable", "Segoe UI", sans-serif');
});

test("字体列表合并系统名单与主题自带字体，同名只保留一项", () => {
    const options = themeFontOptions(
        manifests([
            { family: "宋律宋体", file: "主题-宋律/fonts/songti.woff2" },
            { family: "SimSun", file: "主题-宋律/fonts/songti.woff2" },
        ]),
    );
    assert.deepEqual(options.slice(0, 6), ["Segoe UI Variable", "Segoe UI", "Microsoft YaHei UI", "Microsoft YaHei", "SimSun", "Arial"]);
    assert.deepEqual(options.slice(6), ["宋律宋体"]);
    assert.deepEqual(themeFontOptions(undefined), ["Segoe UI Variable", "Segoe UI", "Microsoft YaHei UI", "Microsoft YaHei", "SimSun", "Arial"]);
});
