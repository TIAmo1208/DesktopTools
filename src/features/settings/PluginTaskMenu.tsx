import { Fragment, useEffect } from "react";

import "./PluginTaskMenu.css";

/** 任务菜单里的一行。阶段用后端原词，界面再换成短文案。 */
export interface PluginTaskItem {
    id: string; // 计划标识。
    name: string; // 插件名。
    status: "queued" | "running" | "done" | "failed" | "cancelled"; // 当前结果。
    stage: string; // 后端阶段。
    percent: number; // 0 到 100。
    detail?: string; // 失败原因。
}

/** 任务菜单只负责展示。展开、取消和向上翻由调用方决定。 */
export const PluginTaskMenu = ({
    tasks,
    open,
    flipped = false,
    onCancel,
    onCollapse,
}: {
    tasks: readonly PluginTaskItem[]; // 当前要显示的任务。
    open: boolean; // 是否展开。
    flipped?: boolean; // 下方放不下时改到上方。
    onCancel: (taskId: string) => void; // 取消一条进行中的任务。
    onCollapse: () => void; // 收起菜单。
}) => {
    useEffect(() => {
        if (!open) return;
        const onKey = (event: KeyboardEvent) => {
            if (event.key === "Escape") onCollapse();
        };
        document.addEventListener("keydown", onKey);
        return () => document.removeEventListener("keydown", onKey);
    }, [open, onCollapse]);
    if (!open) return null;
    const running = tasks.filter((task) => task.status === "running"); // 还在进行的行，用来算总进度。
    const done = tasks.filter((task) => task.status === "done").length; // 已经完成的行。
    const total = Math.max(running.length + done, running.length); // 分母至少覆盖进行中的件数。
    const percent = running.length ? Math.round(running.reduce((sum, task) => sum + task.percent, 0) / running.length) : 0;
    return (
        <div className={`plugin-settings__task-menu${flipped ? " plugin-settings__task-menu--up" : ""}`}>
            {tasks.length === 0 ? (
                <p className="plugin-settings__tasks-empty">
                    暂无进行中的操作
                    <small>安装、更新与卸载的进度将显示在这里</small>
                </p>
            ) : (
                <div className="plugin-settings__tasks-body">
                    {running.length > 0 && (
                        <div className="plugin-settings__task-block">
                            <div className="plugin-settings__task-head">
                                <p>{`正在安装 ${Math.min(done + 1, total)}/${total}`}</p>
                                <button className="plugin-settings__task-cancel" type="button" onClick={() => onCancel(running[0].id)}>
                                    取消
                                </button>
                            </div>
                            <div className="plugin-settings__progress" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={percent}>
                                <span style={{ width: `${percent}%` }} />
                            </div>
                        </div>
                    )}
                    {tasks.map((task, index) => (
                        <Fragment key={task.id}>
                            {(index > 0 || running.length > 0) && <span className="plugin-settings__task-divider" aria-hidden="true" />}
                            <div className="plugin-settings__task">
                                <span>
                                    安装 {task.name}
                                    <small>{pluginTaskCaption(task)}</small>
                                </span>
                            </div>
                        </Fragment>
                    ))}
                </div>
            )}
        </div>
    );
};

/** 一行下面的短状态。失败优先显示后端原因。 */
export const pluginTaskCaption = (task: PluginTaskItem) => {
    if (task.status === "failed" && task.detail) return task.detail;
    if (task.status === "cancelled") return "已取消";
    if (task.status === "queued") return "排队中";
    return { validate: "校验", extract: "解压", copy: "复制", done: "完成", failed: "失败", cancelled: "已取消" }[task.stage] ?? task.stage;
};
