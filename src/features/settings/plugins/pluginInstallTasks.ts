/** 一条安装任务。只活在这次打开的设置页里。 */
export interface InstallTask {
    id: string; // 计划标识。
    name: string; // 插件名。
    status: "running" | "done" | "failed" | "cancelled"; // 当前结果。
    stage: string; // 后端阶段。
    percent: number; // 0 到 100。
    detail?: string; // 失败原因。
}

/** 内存里保留的任务条数。 */
const INSTALL_TASK_LIMIT = 20;

/** 进度事件的字段；后端事件与任务行只在这一层对齐。 */
export interface InstallProgress {
    taskId: string;
    sourceName: string;
    stage: string;
    percent: number;
}

/** 写入或更新一条任务，并丢掉超出的最旧记录。 */
export const rememberInstallTask = (tasks: readonly InstallTask[], next: InstallTask): InstallTask[] => {
    const without = tasks.filter((task) => task.id !== next.id); // 同一计划只留最新状态。
    return [...without, next].slice(-INSTALL_TASK_LIMIT);
};

/** 合并一条后端进度：已结束的行不被迟到进度改回进行中，名称保留首次已知值。 */
export const progressedInstallTask = (tasks: readonly InstallTask[], progress: InstallProgress): InstallTask[] => {
    const existing = tasks.find((task) => task.id === progress.taskId); // 同一计划已有任务行。
    if (existing && existing.status !== "running") return [...tasks];
    const status = progress.stage === "done" ? "done" : progress.stage === "failed" ? "failed" : "running"; // 阶段到结果的映射。
    return rememberInstallTask(tasks, {
        id: progress.taskId,
        name: existing?.name || progress.sourceName,
        status,
        stage: progress.stage,
        percent: progress.percent,
    });
};

/** 结束一条安装：用户已取消的行保留取消结果，失败沿用已有进度。 */
export const failedInstallTask = (tasks: readonly InstallTask[], target: { id: string; name: string; cancelled: boolean; detail: string }): InstallTask[] => {
    const existing = tasks.find((task) => task.id === target.id); // 用户可能先点了取消。
    if (existing?.status === "cancelled") return [...tasks];
    return rememberInstallTask(tasks, {
        id: target.id,
        name: target.name,
        status: target.cancelled ? "cancelled" : "failed",
        stage: target.cancelled ? "cancelled" : "failed",
        percent: existing?.percent ?? 0,
        detail: target.detail,
    });
};

/** 用户取消一条安装任务，保留已有阶段与进度。 */
export const cancelledInstallTask = (tasks: readonly InstallTask[], task: InstallTask): InstallTask[] => rememberInstallTask(tasks, { ...task, status: "cancelled", stage: "cancelled" });

/** 进行中的总进度。N 是当前这一件，M 含已完成。 */
export const installBoard = (tasks: readonly InstallTask[]) => {
    const running = tasks.filter((task) => task.status === "running"); // 还在安装的行。
    const done = tasks.filter((task) => task.status === "done").length; // 已经完成的行。
    const total = Math.max(running.length + done, running.length); // 分母至少覆盖进行中的件数。
    const percent = running.length ? Math.round(running.reduce((sum, task) => sum + task.percent, 0) / running.length) : 0;
    return { running: running.length, text: running.length ? `正在安装 ${Math.min(done + 1, total)}/${total}` : "", percent };
};

/** 进度阶段的界面文案。 */
export const installStageLabel = (stage: string) =>
    (({ validate: "校验", extract: "解压", copy: "复制", done: "完成", failed: "失败", cancelled: "已取消" }) as Record<string, string>)[stage] ?? stage;
