// 任务的持久事实与时间投影。时间参数可注入；不读取 DOM，不在模块内启动计时器。
const levels = ['high', 'medium', 'low'];
const defaults = { defaultRemindMinutes: 60, defaultSeverity: 'medium', collapsedCount: 3 };
const isObject = value => value !== null && typeof value === 'object' && !Array.isArray(value);
const isTime = value => value === null || (Number.isSafeInteger(value) && Number.isFinite(new Date(value).getTime()));
const text = (value, max) => typeof value === 'string' && value.trim().length > 0 && [...value].length <= max;
const validTags = values => Array.isArray(values) && values.every(tag => text(tag, 16)) && new Set(values).size === values.length;
const uniqueIds = items => new Set(items.map(item => item.id)).size === items.length;
const assert = (condition, message) => { if (!condition) throw new Error(message); };

/** 只将明确未创建的 null 视为空；损坏的数据不能作为空列表被写回。 */
export function readState(raw) {
    if (raw === null) return { version: 1, prefs: { ...defaults }, tags: [], tasks: [] };
    assert(isObject(raw) && raw.version === 1 && Array.isArray(raw.tasks) && validTags(raw.tags), '待办数据格式无效，未覆盖原数据');
    const prefs = { ...defaults, ...raw.prefs };
    assert(levels.includes(prefs.defaultSeverity) && [3, 5, 10].includes(prefs.collapsedCount) && Number.isSafeInteger(prefs.defaultRemindMinutes) && prefs.defaultRemindMinutes >= 0 && prefs.defaultRemindMinutes <= 10080, '待办偏好无效');
    const record = item => isObject(item) && text(item.id, 100) && text(item.title, 120) && levels.includes(item.severity) && isTime(item.dueAt) && validTags(item.tags) && item.tags.every(tag => raw.tags.includes(tag));
    assert(raw.tasks.length <= 500 && raw.tags.length <= 64, '任务或标签数量已达上限');
    for (const task of raw.tasks) {
        assert(record(task) && task.tags.length <= 8 && typeof task.content === 'string' && [...task.content].length <= 4000 && isTime(task.remindAt) && isTime(task.completedAt) && task.createdAt !== null && isTime(task.createdAt) && task.updatedAt !== null && isTime(task.updatedAt), '任务字段无效');
        assert(Array.isArray(task.subtasks) && task.subtasks.length <= 50 && task.subtasks.every(child => record(child) && child.tags.length <= 4 && typeof child.done === 'boolean') && uniqueIds(task.subtasks), '子任务字段或标识无效');
    }
    assert(uniqueIds(raw.tasks), '任务标识重复');
    return structuredClone({ ...raw, prefs });
}

/** 草稿创建时分配身份；首次保存前不会进入列表。 */
export function newTask(state, now = Date.now()) {
    return { id: crypto.randomUUID(), title: '', content: '', severity: state.prefs.defaultSeverity, tags: [], subtasks: [], dueAt: null, remindAt: null, completedAt: null, createdAt: now, updatedAt: now };
}
export function newSubtask(parent) {
    return { id: crypto.randomUUID(), title: '', severity: parent.severity, tags: [], dueAt: null, done: false };
}

// 日期筛选与时刻判定分开，无截止时间不排在有截止时间的项前。
const activeParts = task => [task, ...task.subtasks.filter(child => !child.done)];
const overdue = (task, now) => activeParts(task).some(part => part.dueAt !== null && part.dueAt < now);
const rank = (a,b) => levels.indexOf(a.severity) - levels.indexOf(b.severity) || (a.dueAt ?? Infinity) - (b.dueAt ?? Infinity) || 0;
const sameDay = (at, now) => at !== null && new Date(at).toDateString() === new Date(now).toDateString();
export const sortedSubtasks = task => [...task.subtasks].sort(rank);
export function selectTasks(state, filter = 'all', now = Date.now()) {
    if (filter === 'completed') return state.tasks.filter(task => task.completedAt !== null).sort((a,b) => b.completedAt - a.completedAt);
    return state.tasks.filter(task => task.completedAt === null && (filter === 'today' ? activeParts(task).some(part => sameDay(part.dueAt, now)) : filter === 'overdue' ? overdue(task, now) : true)).sort((a,b) => Number(overdue(b, now)) - Number(overdue(a, now)) || rank(a,b));
}

const reasons = ['overdue','due','reminder','high'];
const byReason = (a,b) => reasons.indexOf(a.reason) - reasons.indexOf(b.reason) || (a.at ?? Infinity) - (b.at ?? Infinity) || 0;
/** 每个未完成父任务只返回最紧急的一项，并保留触发它的子任务身份。 */
export function reminders(state, now = Date.now()) {
    const items = [];
    for (const task of state.tasks) {
        if (task.completedAt !== null) continue;
        const matches = [];
        for (const part of activeParts(task)) {
            const childId = part === task ? null : part.id;
            if (part.dueAt !== null && part.dueAt <= now) matches.push({ task, childId, reason: part.dueAt === now ? 'due' : 'overdue', at: part.dueAt });
            if (part.severity === 'high') matches.push({ task, childId, reason: 'high', at: part.dueAt });
        }
        if (task.remindAt !== null && task.remindAt <= now) matches.push({ task, childId: null, reason: 'reminder', at: task.remindAt });
        if (matches.length) items.push(matches.sort(byReason)[0]);
    }
    return items.sort((a,b) => byReason(a,b) || rank(a.task,b.task));
}
/** 活动窗口内跨过的事件，首次挂载/恢复不得传入历史时间来补播。 */
export function messageEvents(state, after, now) {
    const events = [];
    for (const task of state.tasks) {
        if (task.completedAt !== null) continue;
        for (const [at,kind,label] of [[task.remindAt,'remind','提醒'],[task.dueAt,'due','到期']]) {
            if (at !== null && at > after && at <= now) events.push({ at, key: `${kind}:${task.id}:${at}`, kind: 'info', text: `${label} · ${task.title}` });
        }
    }
    return events.sort((a,b) => a.at-b.at);
}
/** 下一次影响提醒集合的时刻（无未来边界不唤醒）。 */
export function nextBoundary(state, now = Date.now()) {
    let next = Infinity;
    for (const task of state.tasks) {
        if (task.completedAt !== null) continue;
        if (task.remindAt !== null && task.remindAt > now) next = Math.min(next,task.remindAt);
        for (const part of activeParts(task)) if (part.dueAt !== null && part.dueAt >= now) next = Math.min(next, part.dueAt === now ? now + 1 : part.dueAt);
    }
    return Number.isFinite(next) ? next : null;
}

// 从宿主读最新快照，CAS 失败只重放纯变换；没有 CAS 时不退回不安全的全量覆盖。
export async function updateState(ctx, edit) {
    for (let attempt = 0; attempt < 3; attempt++) {
        const expected = await ctx.readSettings();
        const next = readState(expected);
        const changed = readState(edit(next) ?? next);
        if (await ctx.compareAndSaveSettings(expected, changed)) return changed;
    }
    throw new Error('其他窗口也在修改待办，请重试');
}
const equal = (a,b) => JSON.stringify(a) === JSON.stringify(b);
const conflict = () => { throw new Error('该任务已在别处修改，请核对最新内容后再保存'); };
/** 逐字段合并，而非以旧草稿替换整条任务；相同改动视为幂等。 */
function mergeFields(base, draft, current, keys) {
    const merged = { ...current };
    for (const key of keys) if (!equal(base[key],draft[key])) {
        if (!equal(base[key],current[key]) && !equal(current[key],draft[key])) conflict();
        merged[key] = structuredClone(draft[key]);
    }
    return merged;
}
function mergeChildren(base, draft, current) {
    const next = [...current];
    for (const original of base) {
        const item = draft.find(child => child.id === original.id);
        const i = next.findIndex(child => child.id === original.id);
        if (!item) { if (i >= 0) { if (!equal(original,next[i])) conflict(); next.splice(i,1); } }
        else if (i < 0) { if (!equal(original,item)) conflict(); }
        else next[i] = mergeFields(original,item,next[i],['title','severity','dueAt','tags','done']);
    }
    for (const item of draft.filter(child => !base.some(old => old.id === child.id))) {
        const existing = next.find(child => child.id === item.id);
        if (existing && !equal(existing,item)) conflict();
        if (!existing) next.push(structuredClone(item));
    }
    return next;
}
/** 将保存期间继续输入的字段叠到成功快照上；同字段冲突交还编辑窗处理。 */
export function rebaseTask(sent, currentDraft, persisted) {
    const result = mergeFields(sent, currentDraft, persisted, ['title','content','severity','dueAt','remindAt','completedAt','tags']);
    result.subtasks = mergeChildren(sent.subtasks,currentDraft.subtasks,persisted.subtasks);
    return result;
}
export function saveTask(ctx, baseline, draft) {
    const edited = structuredClone(draft);
    return updateState(ctx,state => {
        const i = state.tasks.findIndex(task => task.id === edited.id);
        let next;
        if (baseline === null) { if (i >= 0) conflict(); next = structuredClone(edited); }
        else {
            if (baseline.id !== edited.id || i < 0) conflict();
            next = mergeFields(baseline,edited,state.tasks[i],['title','content','severity','dueAt','remindAt','completedAt','tags']);
            next.subtasks = mergeChildren(baseline.subtasks,edited.subtasks,state.tasks[i].subtasks);
        }
        // 只引入合并结果实际持有的标签，不从旧草稿重新添加已被移除的标签。
        for (const tag of [next,...next.subtasks].flatMap(item => item.tags)) if (!state.tags.includes(tag)) state.tags.push(tag);
        next.updatedAt = Date.now();
        if (i < 0) state.tasks.push(next); else state.tasks[i] = next;
    });
}
export function removeTask(ctx, baseline) {
    return updateState(ctx,state => {
        const i = state.tasks.findIndex(task => task.id === baseline.id);
        if (i < 0) return;
        if (!equal(state.tasks[i],baseline)) conflict();
        state.tasks.splice(i,1);
    });
}
export function setCompleted(ctx, taskId, childId, done) {
    const now=Date.now();
    return updateState(ctx,state => {
        const task=state.tasks.find(task=>task.id===taskId);
        assert(task,'任务已不存在');
        if (childId !== null) { const child=task.subtasks.find(item=>item.id===childId); assert(child,'子任务已不存在'); child.done=done; }
        else task.completedAt=done ? (task.completedAt ?? now) : null;
        task.updatedAt=now;
    });
}
export function clearCompleted(ctx, ids) {
    const selected=new Set(ids);
    return updateState(ctx,state=>{state.tasks=state.tasks.filter(task=>task.completedAt===null || !selected.has(task.id));});
}
