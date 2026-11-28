import { clearCompleted, messageEvents, nextBoundary, readState, reminders, selectTasks, setCompleted, sortedSubtasks } from './model.js';
import { renderWindow, settingsEntries } from './editor.js';

export { renderWindow, settingsEntries };

const filters = { all: '全部', today: '今天', overdue: '逾期', completed: '已完成' };
const severityNames = { high: '高', medium: '中', low: '低' };
const minute = 60_000;

/** 每个挂载只有一个到下一次变化的一次性计时器。只有浮岛 renderTile 传入消息投递口。 */
export function observeClock(doc, onPaint, publish, visibleTimes) {
    let state, timer, baseline, disposed = false;
    function paint(now) {
        if (disposed || doc.hidden || !state) return;
        onPaint(now);
        let boundary = nextBoundary(state, now) ?? Infinity;
        // 本地日历与可见的相对逾期文案也会随时间改变，不以整分钟轮询任务集合。
        if (visibleTimes && state.tasks.length) {
            const midnight = new Date(now);
            midnight.setHours(24, 0, 0, 0);
            boundary = Math.min(boundary, +midnight, visibleTimes(now));
        }
        if (!Number.isFinite(boundary)) return;
        timer = setTimeout(() => {
            if (disposed || doc.hidden) return;
            const at = Date.now();
            if (publish && at > baseline) {
                for (const event of messageEvents(state, baseline, at)) {
                    Promise.resolve(publish(event)).catch(error => console.warn('待办提醒投递失败', error));
                }
            }
            baseline = Math.max(at, baseline);
            paint(at);
        }, Math.max(1, Math.min(2 ** 31 - 1, boundary - now)));
    }
    function reset() {
        clearTimeout(timer);
        baseline = Date.now(); // 首次、恢复和设置更新只采纳当前状态，不补播历史。
        paint(baseline);
    }
    doc.addEventListener('visibilitychange', reset);
    return {
        update(value) { state = value; reset(); },
        dispose() { disposed = true; clearTimeout(timer); doc.removeEventListener('visibilitychange', reset); },
    };
}

/** 所有任务内容用文本节点赋值，不将用户输入作为标签或选择器。 */
function element(name, cls, text) {
    const node = document.createElement(name);
    if (cls) node.className = cls;
    if (text !== undefined) node.textContent = text;
    return node;
}
function button(text, action, cls = '') {
    const node = element('button', cls, text);
    node.type = 'button';
    node.addEventListener('click', action);
    return node;
}
function label(node, text) {
    if (node.textContent !== text) node.textContent = text;
}
function shortDate(at, now) {
    if (at == null) return '';
    const date = new Date(at), today = new Date(now), tomorrow = new Date(now);
    tomorrow.setDate(today.getDate() + 1);
    const day = date.toDateString() === today.toDateString() ? '今天' : date.toDateString() === tomorrow.toDateString() ? '明天' :
        `${date.getFullYear() !== today.getFullYear() ? date.getFullYear() + '/' : ''}${date.getMonth() + 1}/${date.getDate()}`;
    return `${day} ${String(date.getHours()).padStart(2, '0')}:${String(date.getMinutes()).padStart(2, '0')}`;
}
function overdueText(at, now) {
    const minutes = Math.floor((now - at) / minute);
    return minutes < 1 ? '刚刚逾期' : minutes < 1440 ? `逾期 ${minutes} 分` : `逾期 ${Math.floor(minutes / 1440)} 天`;
}
function reasonText(entry, now) {
    if (entry.reason === 'overdue') return overdueText(entry.at, now);
    if (entry.reason === 'high') return '待处理';
    return `${shortDate(entry.at, now)} ${entry.reason === 'due' ? '到期' : '提醒'}`;
}
function tagText(tags, limit) {
    const text = tags.slice(0, limit).join(' · ');
    return tags.length > limit ? `${text} +${tags.length - limit}` : text;
}

/** 复用按任务身份索引的行，插入/重排时只移动需要变化的 DOM，保留列表与输入焦点。 */
function rowList(surface, actions) {
    const list = element('div', 'todo-rows');
    const rows = new Map();
    function update(items, now) {
        const keep = new Set();
        for (const [i, { task, child, reminder }] of items.entries()) {
            const key = JSON.stringify([task.id, child?.id ?? null]);
            let row = rows.get(key);
            if (!row) {
                row = createRow(surface, task.id, child?.id, actions);
                rows.set(key, row);
            }
            keep.add(key);
            row.update(task, child, reminder, now);
            const previous = list.children[i];
            if (previous !== row.node) list.insertBefore(row.node, previous ?? null);
        }
        for (const [key, row] of rows) if (!keep.has(key)) { row.node.remove(); rows.delete(key); }
    }
    return { node: list, update };
}

function createRow(surface, id, childId, actions) {
    const node = element('div', 'todo-row');
    node.dataset.taskId = id;
    if (childId !== undefined) node.dataset.childId = childId;
    const interactive = surface === 'desktop' || surface === 'hover';
    const dot = element('span', 'todo-dot'), title = element('span', 'todo-title'), time = element('span', 'todo-time');
    const tags = element('span', 'todo-tags'), progress = element('span', 'todo-progress'), meta = element('span', 'todo-meta');
    const feedback = element('span', 'todo-row-error');
    feedback.setAttribute('role', 'alert');
    dot.setAttribute('role', 'img');
    let input, hit, done;
    if (interactive) {
        const target = element('label', 'todo-check');
        input = element('input'); input.type = 'checkbox';
        input.addEventListener('click', event => {
            event.preventDefault(); // 保持已提交状态，宿主确认成功才勾选或移出。
            void actions.complete(id, childId ?? null, !done, input, feedback);
        });
        target.append(input);
        hit = button('', () => actions.open(id, feedback), 'todo-hit');
        hit.dataset.button = 'row-hit';
        node.append(target, hit);
    }
    meta.append(tags, progress);
    node.append(dot, title, time, meta, feedback);
    return { node, update(task, child, reminder, now) {
        const item = child ?? task;
        done = child ? child.done : task.completedAt !== null;
        node.dataset.done = String(done);
        dot.dataset.severity = item.severity;
        dot.setAttribute('aria-label', `${severityNames[item.severity]}优先级`);
        label(title, item.title); title.title = item.title;
        if (input) {
            input.checked = done;
            input.setAttribute('aria-label', `${done ? '取消完成' : '完成'}：${item.title}`);
            hit.setAttribute('aria-label', `打开待办：${task.title}`);
        }
        const overdue = !done && task.completedAt === null && item.dueAt != null && item.dueAt < now;
        const status = reminder ? reasonText(reminder, now) : overdue && !child ? overdueText(item.dueAt, now) : shortDate(item.dueAt, now);
        label(time, status || (!child && item.remindAt != null ? `${shortDate(item.remindAt, now)} 提醒` : ''));
        time.title = [item.dueAt != null && `到期：${shortDate(item.dueAt, now)}`, !child && item.remindAt != null && `提醒：${shortDate(item.remindAt, now)}`].filter(Boolean).join('\n');
        time.dataset.overdue = String(reminder ? reminder.reason === 'overdue' : overdue);
        label(tags, (surface === 'desktop' || (surface === 'card' && child)) ? tagText(item.tags, child ? 1 : 2) : '');
        tags.title = item.tags.join('、');
        const children = !child && task.subtasks;
        const count = children?.length ? `${children.filter(item => item.done).length}/${children.length}` : '';
        const late = surface === 'hover' && children ? children.filter(item => !item.done && item.dueAt != null && item.dueAt < now).length : 0;
        label(progress, count && surface !== 'card' ? `${surface === 'hover' ? '子任务 ' : ''}${count}${late ? ` · ${late} 逾期` : ''}` : '');
        meta.hidden = !tags.textContent && !progress.textContent;
    } };
}

function listItems(tasks, children) {
    return tasks.flatMap(task => [{ task }, ...(children ? sortedSubtasks(task).map(child => ({ task, child })) : [])]);
}

function mount(container, context, surface) {
    const doc = container.ownerDocument;
    if (!doc.querySelector('link[data-plugin-style="todo-list"]')) {
        const link = element('link'); link.rel = 'stylesheet';
        link.href = new URL('./list.css', import.meta.url).href;
        link.dataset.pluginStyle = 'todo-list'; doc.head.append(link);
    }
    const root = element('div', 'todo-list');
    root.dataset.surface = surface;
    container.replaceChildren(root);
    const desktop = surface === 'desktop', island = surface === 'card' || surface === 'capsule';
    let state, alive = true, revision = 0, filter = 'all', showCompleted = false, folded = context.cardCollapsed === true;
    let menuKey, disposeMenu, nextLabelAt = Infinity;
    const summary = element('div', 'todo-summary');
    const notice = element('div', 'todo-notice'); notice.setAttribute('role', 'alert');
    const toolbar = element('div', 'todo-tools'), tabs = element('div', 'todo-filters'), select = element('select', 'todo-select');
    const scroll = element('div', 'todo-scroll'), blank = element('div', 'todo-blank'), placeholder = element('span');
    const blankAction = button('', () => state ? state.tasks.length ? changeFilter('all') : open(null) : retry(), 'todo-action');
    blank.append(placeholder, blankAction);
    const rows = rowList(surface, { complete, open });
    const finished = rowList(surface, { complete, open });
    const group = button('', () => { showCompleted = !showCompleted; paint(Date.now()); }, 'todo-group');
    if (desktop) {
        select.setAttribute('aria-label', '筛选待办');
        for (const [key, text] of Object.entries(filters)) {
            const tab = button(text, () => changeFilter(key)); tab.value = key; tabs.append(tab);
            const opt = element('option', '', text); opt.value = key; select.append(opt);
        }
        select.addEventListener('change', () => changeFilter(select.value));
        const add = button('＋ 新建', () => open(null), 'todo-new'); add.setAttribute('aria-label', '新建待办');
        toolbar.append(tabs, select, add); root.append(toolbar);
    }
    if (surface !== 'capsule') {
        if (surface === 'hover') {
            const heading = element('div', 'todo-heading', '待办'); root.append(heading);
            // 宿主不能把正文的 click/Enter 或复选框的 Space 再当作拖窗、展开手势。
            for (const name of ['pointerdown', 'pointerup', 'mousedown', 'mouseup', 'click', 'dblclick', 'keydown', 'keyup']) {
                root.addEventListener(name, event => { if (event.target.closest('button,label,input')) event.stopPropagation(); });
            }
        }
        if (surface !== 'hover') root.append(summary);
        root.append(notice, scroll); scroll.append(blank, rows.node);
        if (desktop) scroll.append(group, finished.node);
    }
    function changeFilter(value) { filter = value; scroll.scrollTop = 0; paint(Date.now()); }
    async function open(id, errorNode = notice) {
        const task = id && state?.tasks.find(item => item.id === id);
        label(errorNode, '');
        try {
            await context.openWindow({ view: 'task', windowKey: id ?? 'new', title: task ? `编辑待办 · ${task.title}`.slice(0, 80) : '新建待办',
                size: { width: 720, height: 640 }, minSize: { width: 480, height: 520 }, data: id ? { taskId: id } : {} });
        } catch (error) { if (alive) label(errorNode, `无法打开待办：${error.message ?? error}`); }
    }
    async function complete(id, child, done, input, errorNode) {
        if (input.disabled) return;
        input.disabled = true; label(errorNode, '');
        const before = revision;
        try {
            const updated = await setCompleted(context, id, child, done);
            if (alive && before === revision) receive(updated);
        } catch (error) { if (alive) label(errorNode, `保存失败：${error.message ?? error}`); }
        finally { if (alive) input.disabled = false; }
    }
    async function clearDone() {
        const ids = state?.tasks.filter(task => task.completedAt !== null).map(task => task.id) ?? [];
        if (!ids.length || !doc.defaultView.confirm(`清除这 ${ids.length} 项已完成待办及其子任务？此操作不可撤销。`)) return;
        const before = revision;
        try { const updated = await clearCompleted(context, ids); if (alive && before === revision) receive(updated); }
        catch (error) { if (alive) label(notice, `清除失败：${error.message ?? error}`); }
    }
    function syncMenu(hasDone) {
        if (!desktop || hasDone === menuKey) return;
        menuKey = hasDone;
        disposeMenu = context.menu?.register([
            { id: 'new', label: '新建待办', onSelect: () => open(null) },
            { id: 'completed', label: '显示已完成', onSelect: () => changeFilter('completed') },
            { id: 'clear', label: '清除已完成', disabled: !hasDone, danger: true, separatorBefore: true, onSelect: clearDone },
        ]);
    }
    async function retry() {
        const before = revision;
        blankAction.disabled = true;
        try { const raw = await context.readSettings(); if (alive && revision === before) receive(raw); }
        catch (error) { if (alive && revision === before) receive(null, error.message ?? String(error)); }
        finally { if (alive) blankAction.disabled = false; }
    }
    function empty(text, action) {
        label(placeholder, text); label(blankAction, action ?? ''); blankAction.hidden = !action; blank.hidden = !text;
    }
    function paint(now) {
        if (!state || !alive) return;
        nextLabelAt = Infinity;
        const hits = reminders(state, now);
        if (island) context.setIslandSummary(hits.length ? `${hits.length} 项待处理` : null);
        if (surface === 'capsule') return;
        // 逾期时间只刷新当前渲染的行，到下一分钟/天变化处即停，不维持常驻秒表。
        function track(at) {
            const age = now - at, step = age < 86400000 ? minute : 86400000;
            nextLabelAt = Math.min(nextLabelAt, at + (Math.floor(age / step) + 1) * step);
        }
        if (desktop) {
            const active = selectTasks(state, 'all', now), completed = selectTasks(state, 'completed', now);
            label(summary, `${active.length} 项待办 · ${selectTasks(state, 'overdue', now).length} 项逾期`);
            select.value = filter;
            for (const tab of tabs.children) tab.setAttribute('aria-pressed', String(tab.value === filter));
            const tasks = filter === 'all' ? active : filter === 'completed' ? completed : selectTasks(state, filter, now);
            for (const task of tasks) if (task.completedAt === null && task.dueAt != null && task.dueAt < now) track(task.dueAt);
            rows.update(listItems(tasks, true), now);
            group.hidden = filter !== 'all' || !completed.length;
            label(group, `${showCompleted ? '▾' : '▸'}　已完成 · ${completed.length}`);
            group.setAttribute('aria-expanded', String(showCompleted));
            finished.update(listItems(!group.hidden && showCompleted ? completed : [], true), now);
            syncMenu(completed.length > 0);
            if (!state.tasks.length) empty('尚无待办', '新建待办');
            else if (!tasks.length && group.hidden) empty('没有符合筛选的待办', '清除筛选');
            else empty('');
        } else {
            const visible = surface === 'hover' ? hits.slice(0, 3) : folded ? hits.slice(0, state.prefs.collapsedCount) : hits;
            label(summary, folded ? `${visible.length} / ${hits.length} 项` : `${hits.length} 项提醒`);
            const items = [];
            for (const reminder of visible) {
                const { task } = reminder;
                if (reminder.reason === 'overdue') track(reminder.at);
                items.push({ task, reminder });
                if (surface === 'card' && !folded) for (const child of sortedSubtasks(task)) items.push({ task, child });
            }
            rows.update(items, now);
            empty(hits.length ? '' : '暂无待处理任务');
        }
    }
    const clock = observeClock(doc, paint, island && context.publishIslandMessage, surface === 'capsule' ? null : () => nextLabelAt);
    function receive(raw, settingsError) {
        if (!alive) return;
        ++revision;
        try {
            if (settingsError) throw new Error(settingsError);
            state = readState(raw); label(notice, ''); clock.update(state);
        } catch (error) {
            state = undefined; clock.update(null); rows.update([], 0); finished.update([], 0); group.hidden = true;
            label(summary, ''); syncMenu(false);
            if (island) context.setIslandSummary('读取失败', 'failed');
            empty(`待办读取失败：${error.message ?? error}`, desktop ? '重试' : '');
        }
    }
    const stopSettings = context.onSettingsChanged?.(receive);
    const stopFold = surface === 'card' ? context.onCardCollapsedChanged?.(value => { folded = value; paint(Date.now()); }) : null;
    receive(context.settings, context.settingsError);
    return () => { alive = false; clock.dispose(); stopSettings?.(); stopFold?.(); disposeMenu?.(); root.remove(); };
}

export function renderTile(container, context) {
    const island = typeof context.setIslandSummary === 'function';
    return mount(container, context, island ? context.collapsed ? 'capsule' : 'card' : 'desktop');
}
export function renderIslandHover(container, context) { return mount(container, context, 'hover'); }

export default { renderTile, renderIslandHover, renderWindow, settingsEntries };
