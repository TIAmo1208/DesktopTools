/** 目标文件夹已有插件时的版本比较。 */
export interface PluginInstallConflict {
    installedVersion: string; // 已安装版本。
    incomingVersion: string; // 待安装版本。
    downgrade: boolean; // 待装版本低于已装版本。
}

/** 尚未落盘的安装计划。没有冲突时可以直接执行。 */
export interface PluginInstallPlan {
    planId: string; // 执行或取消用的标识。
    name: string; // 插件显示名。
    version: string; // 待安装版本。
    targetFolder: string; // 插件根下的目标文件夹。
    sourceName: string; // 进度里的来源名。
    author?: string; // 插件作者。空则弹窗只显示 @标识。
    pluginId?: string; // 插件标识。
    conflict?: PluginInstallConflict; // 目标已存在时才有。
}

/** 安装过程中的一条进度。 */
export interface PluginInstallProgress {
    taskId: string; // 对应计划标识。
    stage: string; // validate、extract、copy、done、failed。
    percent: number; // 0 到 100。
    sourceName: string; // 来源名。
}

/** 一次选择的安装结果。计划去排队，错误逐条显示。 */
export interface PluginInstallBatch {
    plans: PluginInstallPlan[]; // 可以执行的计划。
    errors: string[]; // 后端拒绝的原因，每条单独展示。
}

/** 校验安装计划。取消时后端给空。 */
export const parsePluginInstallPlan = (value: unknown): PluginInstallPlan | undefined => {
    if (value === null || value === undefined) return undefined;
    if (typeof value !== "object") throw new Error("插件安装计划无效");
    const plan = value as Record<string, unknown>; // 等待字段校验的计划。
    if (typeof plan.planId !== "string" || !plan.planId || typeof plan.name !== "string" || !plan.name) throw new Error("插件安装计划无效");
    if (typeof plan.version !== "string" || typeof plan.targetFolder !== "string" || typeof plan.sourceName !== "string") throw new Error("插件安装计划无效");
    const conflict = plan.conflict === undefined || plan.conflict === null ? undefined : parsePluginInstallConflict(plan.conflict);
    const author = typeof plan.author === "string" && plan.author ? plan.author : undefined; // 空作者不当成名字。
    const pluginId = typeof plan.pluginId === "string" && plan.pluginId ? plan.pluginId : undefined; // 空标识不占位。
    if (plan.author !== undefined && plan.author !== "" && typeof plan.author !== "string") throw new Error("插件安装计划无效");
    if (plan.pluginId !== undefined && plan.pluginId !== "" && typeof plan.pluginId !== "string") throw new Error("插件安装计划无效");
    return {
        planId: plan.planId,
        name: plan.name,
        version: plan.version,
        targetFolder: plan.targetFolder,
        sourceName: plan.sourceName,
        author,
        pluginId,
        conflict,
    };
};

/** 校验版本冲突。 */
const parsePluginInstallConflict = (value: unknown): PluginInstallConflict => {
    if (typeof value !== "object" || value === null) throw new Error("插件安装计划无效");
    const conflict = value as Record<string, unknown>; // 等待字段校验的冲突。
    if (typeof conflict.installedVersion !== "string" || typeof conflict.incomingVersion !== "string" || typeof conflict.downgrade !== "boolean")
        throw new Error("插件安装计划无效");
    return { installedVersion: conflict.installedVersion, incomingVersion: conflict.incomingVersion, downgrade: conflict.downgrade };
};

/** 接受 { plans, errors }。空的两边都是取消。结构不对才抛错。 */
export const parsePluginInstallBatch = (value: unknown): PluginInstallBatch => {
    if (value === null || value === undefined) return { plans: [], errors: [] };
    if (typeof value !== "object" || Array.isArray(value)) throw new Error("插件安装计划无效");
    const batch = value as Record<string, unknown>; // 等待字段校验的安装结果。
    if (!Array.isArray(batch.plans) || !Array.isArray(batch.errors)) throw new Error("插件安装计划无效");
    return {
        plans: batch.plans.map((item) => {
            const plan = parsePluginInstallPlan(item); // 带 errors 的对象不会进到这里。
            if (!plan) throw new Error("插件安装计划无效");
            return plan;
        }),
        errors: batch.errors.map((item) => {
            if (typeof item !== "string" || !item.trim()) throw new Error("插件安装计划无效");
            return item;
        }),
    };
};

/** 校验一条安装进度。 */
export const parsePluginInstallProgress = (value: unknown): PluginInstallProgress => {
    if (typeof value !== "object" || value === null) throw new Error("插件安装进度无效");
    const progress = value as Record<string, unknown>; // 等待字段校验的进度。
    if (typeof progress.taskId !== "string" || typeof progress.stage !== "string" || typeof progress.sourceName !== "string")
        throw new Error("插件安装进度无效");
    if (typeof progress.percent !== "number" || progress.percent < 0 || progress.percent > 100) throw new Error("插件安装进度无效");
    return { taskId: progress.taskId, stage: progress.stage, percent: progress.percent, sourceName: progress.sourceName };
};
