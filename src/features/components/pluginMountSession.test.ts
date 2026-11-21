import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import { beginPluginMountSession, createPluginMountSessions, endPluginMountSession, ownsPluginMountSession } from "./pluginMountSession.ts";

test("换挂载位时旧会话失效，旧清理不能释放新实例", () => {
    const sessions = createPluginMountSessions();
    const oldSession = beginPluginMountSession(sessions, "island.plugin.demo");
    const newSession = beginPluginMountSession(sessions, "island.plugin.demo");
    assert.notEqual(oldSession, newSession);
    assert.equal(ownsPluginMountSession(sessions, "island.plugin.demo", oldSession), false); // 旧视图已让位。
    assert.equal(endPluginMountSession(sessions, "island.plugin.demo", oldSession), false); // 迟到清理不能结束新会话。
    assert.equal(ownsPluginMountSession(sessions, "island.plugin.demo", newSession), true);
    assert.equal(endPluginMountSession(sessions, "island.plugin.demo", newSession), true); // 当前会话可正常结束。
    assert.equal(ownsPluginMountSession(sessions, "island.plugin.demo", newSession), false);
    assert.equal(endPluginMountSession(sessions, "island.plugin.demo", newSession), false); // 重复结束无副作用。
});

test("PluginSlot 先同步结束旧实例再创建新会话", () => {
    const source = readFileSync(new URL("./PluginSlot.tsx", import.meta.url), "utf8");
    const mount = source.slice(source.indexOf("const mountPluginTile"), source.indexOf("/** 有实例的插件按标识导入一次模块。 */"));
    assert.ok(mount.length > 0);
    assert.ok(mount.indexOf("releasePluginTileNow(context.instanceId)") < mount.indexOf("beginPluginMountSession(tileSessions")); // 先释放旧 renderTile。
    assert.ok(mount.indexOf("releasePluginHoverNow(context.instanceId)") < mount.indexOf("beginPluginMountSession(hoverSessions")); // 先释放旧 hover 视图。
    assert.match(mount, /return session;/);
    assert.match(source, /releasePluginTileSession\(instanceId, sessionRef\.current\)/); // 清理只结束自己的会话。
    assert.match(source, /releasePluginHover\(instanceId, sessionRef\.current\)/);
    assert.match(source, /if \(!endPluginMountSession\(tileSessions, instanceId, session\)\) return;/); // 旧会话不释放新实例。
});
