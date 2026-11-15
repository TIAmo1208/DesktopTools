// 架构边界扫描：前端 feature 之间、设置子域之间、Rust 模块之间的禁止依赖。
// 规则以基线运行：既有依赖必须逐条登记，新增违规会让 test:scripts 失败。
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, posix } from "node:path";

const { dirname, normalize } = posix; // 源码路径统一用正斜杠比较。

const FEATURES_ROOT = "src/features";
const SETTINGS_ROOT = "src/features/settings";
const RUST_ROOT = "src-tauri/src";

/** Rust 模块禁止依赖的边；领域层不得依赖应用层，DTO 不得依赖平台。 */
const RUST_FORBIDDEN = {
    workspace: ["app", "commands"],
    contracts: ["platform", "app", "commands", "host", "workspace", "shell"],
    platform: ["app", "commands"],
    component_runtime: ["commands"],
    weather: ["app"],
};

/** 递归列出相对路径下的文件。 */
const walk = (root, dir, accept) => {
    const entries = [];
    for (const name of readdirSync(join(root, dir))) {
        const path = `${dir}/${name}`;
        if (statSync(join(root, path)).isDirectory()) entries.push(...walk(root, path, accept));
        else if (accept(path)) entries.push(path);
    }
    return entries;
};

/** 取源码里的相对导入目标；跳过查询串与 bare 模块。 */
const relativeImports = (source) => [...source.matchAll(/(?:from|import)\s*\(?\s*["'](\.[^"']+)["']/g)].map((match) => match[1].split("?")[0]);

/** 前端 feature 之间的依赖边。 */
const collectFeatureEdges = (root) => {
    const violations = [];
    for (const file of walk(root, "src", (path) => /\.(ts|tsx)$/.test(path))) {
        const from = file.match(/^src\/features\/([^/]+)\//)?.[1];
        if (!from) continue;
        const source = readFileSync(join(root, file), "utf8");
        for (const specifier of relativeImports(source)) {
            const target = normalize(`${dirname(file)}/${specifier}`);
            const to = target.match(/^src\/features\/([^/]+)\//)?.[1];
            if (to && to !== from) violations.push({ from, to, file });
        }
    }
    return violations;
};

/** 设置子域之间的依赖边：只允许经 ui 或基线里登记的跨域页。 */
const collectSettingsEdges = (root) => {
    const violations = [];
    for (const file of walk(root, SETTINGS_ROOT, (path) => /\.(ts|tsx)$/.test(path))) {
        const from = file.match(/^src\/features\/settings\/([^/]+)\//)?.[1] ?? "(root)";
        const source = readFileSync(join(root, file), "utf8");
        for (const specifier of relativeImports(source)) {
            const target = normalize(`${dirname(file)}/${specifier}`);
            if (!target.startsWith(`${SETTINGS_ROOT}/`)) continue;
            const to = target.match(/^src\/features\/settings\/([^/]+)\//)?.[1] ?? "(root)";
            if (from === "(root)" || to === "(root)" || to === from || to === "ui") continue; // 外壳聚合、公共基元与同域允许。
            violations.push({ from, to, file });
        }
    }
    return violations;
};

/** Rust 模块之间的禁止依赖边。 */
const collectRustEdges = (root) => {
    const violations = [];
    for (const file of walk(root, RUST_ROOT, (path) => path.endsWith(".rs") && !path.endsWith(".test.rs"))) {
        const from = file.match(/^src-tauri\/src\/([a-z_]+)\//)?.[1];
        if (!from || !RUST_FORBIDDEN[from]) continue;
        const source = readFileSync(join(root, file), "utf8");
        const targets = new Set([...source.matchAll(/use crate::([a-z_]+)/g)].map((match) => match[1]));
        for (const match of source.matchAll(/crate::([a-z_]+)::/g)) targets.add(match[1]);
        for (const to of targets) if (RUST_FORBIDDEN[from].includes(to)) violations.push({ from, to, file: file.replace(/\\/g, "/") });
    }
    return violations;
};

/** 汇总全部边界违规。 */
export const collectViolations = (root) => ({
    frontend: collectFeatureEdges(root),
    settings: collectSettingsEdges(root),
    rust: collectRustEdges(root),
});

/** 基线去重键：前端与设置按模块对，Rust 按文件与目标模块。 */
const keys = {
    frontend: (entry) => `${entry.from}>${entry.to}`,
    settings: (entry) => `${entry.from}>${entry.to}`,
    rust: (entry) => `${entry.file}>${entry.to}`,
};

/** 相对基线的新增违规；基线里已有的依赖不算回归。 */
export const newViolations = (violations, baseline) =>
    Object.fromEntries(
        Object.keys(violations).map((kind) => [
            kind,
            violations[kind].filter((entry) => !(baseline[kind] ?? []).some((allowed) => keys[kind](allowed) === keys[kind](entry))),
        ]),
    );

if (process.argv[1] && process.argv[1].endsWith("检查架构边界.mjs")) {
    const root = process.cwd();
    const baseline = JSON.parse(readFileSync(join(root, "scripts/架构边界基线.json"), "utf8"));
    const extra = newViolations(collectViolations(root), baseline);
    const lines = Object.entries(extra).flatMap(([kind, entries]) => entries.map((entry) => `${kind}: ${keys[kind](entry)}`));
    if (lines.length === 0) {
        console.log("架构边界：无新增违规");
    } else {
        console.error("架构边界：新增违规\n" + lines.join("\n"));
        process.exitCode = 1;
    }
}
