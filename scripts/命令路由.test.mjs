import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const ROUTER = new URL("../src-tauri/src/commands/router.rs", import.meta.url);
const MOD = new URL("../src-tauri/src/commands/mod.rs", import.meta.url);
const MANIFEST = new URL("./命令路由清单.json", import.meta.url);

/** 从路由表的 match 模式提取命令名；or 模式拆成多条，保持源顺序。 */
const commandNames = (source) => [...source.matchAll(/^        ("[^"]+"(?:\s*\|\s*"[^"]+")*) =>/gm)].flatMap((match) => [...match[1].matchAll(/"([^"]+)"/g)].map((name) => name[1]));

test("路由表覆盖清单中的全部命令且只登记一次", () => {
    const names = commandNames(readFileSync(ROUTER, "utf8")); // 当前 transport 命令。
    const manifest = JSON.parse(readFileSync(MANIFEST, "utf8")); // 已确认的命令清单。
    assert.equal(names.length, new Set(names).size, "同一命令不能在路由表中出现两次");
    assert.deepEqual([...names].sort(), [...manifest.commands].sort(), "新增或删除命令时同步 scripts/命令路由清单.json");
});

test("领域路由表全部接入分发表", () => {
    const router = readFileSync(ROUTER, "utf8");
    const mod = readFileSync(MOD, "utf8");
    const routes = [...router.matchAll(/pub\(crate\) fn route_([a-z_]+)\(/g)].map((match) => match[1]); // 路由表函数名。
    assert.ok(routes.length >= 10, `领域路由表过少：${routes.length}`);
    for (const route of routes) assert.ok(mod.includes(`router::route_${route}(`), `分发表未接入 ${route} 路由`);
});
