// 阶段二：插件安装任务的队列规则单测。
import assert from "node:assert/strict";
import test from "node:test";

import { cancelledInstallTask, failedInstallTask, installBoard, installStageLabel, progressedInstallTask, rememberInstallTask, type InstallTask } from "./pluginInstallTasks.ts";

/** 构造一条任务，字段默认与插件安装成功行一致。 */
const task = (id: string, overrides: Partial<InstallTask> = {}): InstallTask => ({ id, name: `插件${id}`, status: "running", stage: "copy", percent: 10, ...overrides });

test("同一计划只保留最新一条，超出二十条丢掉最旧记录", () => {
    let tasks = Array.from({ length: 20 }, (_, index) => task(String(index), { status: "done", stage: "done", percent: 100 }));
    tasks = rememberInstallTask(tasks, task("0", { status: "running" }));
    assert.equal(tasks.length, 20);
    assert.equal(tasks.at(-1)?.id, "0");
    assert.equal(tasks.filter((item) => item.id === "0").length, 1);
    tasks = rememberInstallTask(tasks, task("new"));
    assert.equal(tasks.length, 20);
    assert.equal(tasks[0].id, "2"); // 最旧的 1 与 2 依次被挤出，0 因刚更新保留在末尾之前。
});

test("迟到进度不把已结束的行改回进行中", () => {
    const done = [task("a", { status: "done", stage: "done", percent: 100 })];
    assert.deepEqual(progressedInstallTask(done, { taskId: "a", sourceName: "甲.zip", stage: "copy", percent: 30 }), done);
    const running = progressedInstallTask([task("a")], { taskId: "a", sourceName: "甲.zip", stage: "extract", percent: 55 });
    assert.equal(running[0].stage, "extract");
    assert.equal(running[0].percent, 55);
});

test("进度首次到达时用来源名，后续保留首个已知名称", () => {
    const first = progressedInstallTask([], { taskId: "a", sourceName: "甲.zip", stage: "validate", percent: 5 });
    assert.equal(first[0].name, "甲.zip");
    const second = progressedInstallTask([{ ...first[0], name: "甲" }], { taskId: "a", sourceName: "甲.zip", stage: "copy", percent: 40 });
    assert.equal(second[0].name, "甲");
    const done = progressedInstallTask(second, { taskId: "a", sourceName: "甲.zip", stage: "done", percent: 100 });
    assert.equal(done[0].status, "done");
});

test("失败沿用已取消结果与已有进度", () => {
    const cancelled = [task("a", { status: "cancelled", stage: "cancelled", percent: 20 })];
    assert.deepEqual(failedInstallTask(cancelled, { id: "a", name: "甲", cancelled: false, detail: "安装失败" }), cancelled);
    const failed = failedInstallTask([task("a", { percent: 40 })], { id: "a", name: "甲", cancelled: false, detail: "安装失败：磁盘已满" });
    assert.equal(failed[0].status, "failed");
    assert.equal(failed[0].percent, 40);
    assert.equal(failed[0].detail, "安装失败：磁盘已满");
    const aborted = failedInstallTask([], { id: "b", name: "乙", cancelled: true, detail: "安装已取消" });
    assert.equal(aborted[0].status, "cancelled");
    assert.equal(aborted[0].stage, "cancelled");
});

test("取消任务保留已有阶段与进度", () => {
    const tasks = cancelledInstallTask([task("a", { percent: 60, stage: "copy" })], task("a", { percent: 60, stage: "copy" }));
    assert.equal(tasks[0].status, "cancelled");
    assert.equal(tasks[0].percent, 60);
});

test("任务板只统计运行中的进度与完成件数", () => {
    assert.deepEqual(installBoard([]), { running: 0, text: "", percent: 0 });
    assert.deepEqual(installBoard([task("a", { percent: 50 }), task("b", { status: "done", percent: 100 })]), { running: 1, text: "正在安装 2/2", percent: 50 });
    assert.equal(installBoard([task("a", { percent: 40 }), task("b", { percent: 60 })]).percent, 50);
});

test("阶段文案命中已知阶段，未知阶段原样返回", () => {
    assert.equal(installStageLabel("copy"), "复制");
    assert.equal(installStageLabel("cancelled"), "已取消");
    assert.equal(installStageLabel("unknown"), "unknown");
});
