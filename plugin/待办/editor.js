import { newSubtask, newTask, readState, removeTask, saveTask, updateState } from './model.js';

const levels = [['high', '高'], ['medium', '中'], ['low', '低']]; // 编辑控件用文字而非等级色点。
const pad = value => String(value).padStart(2, '0');

/** datetime-local 不带时区；只在控件边界与本地分钟相互转换。 */
function localMinute(at) {
    if (at === null) return '';
    const d = new Date(at);
    return `${String(d.getFullYear()).padStart(4, '0')}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

/** 插件文案只经 textContent/value 进入 DOM，样式限定在自己的根下。 */
function el(tag, className, text) {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (text !== undefined) node.textContent = text;
    return node;
}
function button(label, action, kind = 'secondary') {
    const node = el('button', 'todo-command', label);
    node.type = 'button';
    node.dataset.button = kind;
    node.addEventListener('click', action);
    return node;
}
function input(name, type = 'text') {
    const node = el('input');
    node.type = type;
    node.setAttribute('aria-label', name);
    return node;
}
function select(name, options) {
    const node = el('select');
    node.setAttribute('aria-label', name);
    for (const [value, label] of options) {
        const option = el('option', '', label);
        option.value = value;
        node.append(option);
    }
    return node;
}
function field(name, control) {
    const wrapper = el('div', 'todo-form__field');
    wrapper.append(el('span', 'todo-form__label', name), control);
    return wrapper;
}
function group(name) {
    const node = el('div', 'todo-segments');
    node.setAttribute('role', 'group');
    node.setAttribute('aria-label', name);
    return node;
}
function segments(name, options, change) {
    const node = group(name);
    const choices = options.map(([value, label]) => {
        const choice = button(label, () => change(value));
        node.append(choice);
        return [value, choice];
    });
    return { node, show: value => choices.forEach(([id, choice]) => choice.setAttribute('aria-pressed', String(value === id))) };
}
function errorText(error) { return error instanceof Error ? error.message : String(error); }

/** 确认与标签选择只作用于本窗；原生 dialog 管理焦点、Esc 和模态命中。 */
function dialog(root, title) {
    const node = el('dialog', 'todo-dialog');
    node.setAttribute('aria-label', title);
    node.append(el('h3', '', title));
    node.addEventListener('close', () => node.remove(), { once: true });
    root.append(node);
    return node;
}
function confirmAction(root, title, message, actionLabel, action) {
    const box = dialog(root, title);
    const cancel = button('取消', () => box.close());
    const actions = el('div', 'todo-dialog__actions');
    actions.append(cancel, button(actionLabel, () => { box.close(); void action(); }, 'danger'));
    box.append(el('p', '', message), actions);
    box.showModal();
    cancel.focus();
}

/** 已选标签是草稿的一部分，新标签由 saveTask 连同任务原子加入库。 */
function pickTags(root, values, library, limit, change) {
    const box = dialog(root, '选择标签');
    const available = el('div', 'todo-tags');
    const name = input('新标签名称');
    name.placeholder = '新标签名称';
    const status = el('span', 'todo-feedback');
    status.setAttribute('role', 'status');
    const tags = new Set([...library, ...values]);
    const selected = new Set(values);
    function toggle(tag) {
        if (selected.has(tag)) selected.delete(tag);
        else if (selected.size < limit) selected.add(tag);
        else { status.textContent = `最多选择 ${limit} 个标签`; return false; }
        status.textContent = '';
        change([...selected]);
        return true;
    }
    function addChoice(tag) {
        const choice = button(tag, () => { if (toggle(tag)) choice.setAttribute('aria-pressed', String(selected.has(tag))); });
        choice.setAttribute('aria-pressed', String(selected.has(tag)));
        available.append(choice);
    }
    function addTag() {
        const tag = name.value.trim();
        if (!tag || [...tag].length > 16) { status.textContent = '标签名称应为 1 至 16 个字符'; name.focus(); return; }
        if (tags.has(tag)) { status.textContent = '该标签已在上方，可直接选择'; return; }
        if (!toggle(tag)) return;
        tags.add(tag); addChoice(tag); name.value = '';
    }
    tags.forEach(addChoice);
    name.addEventListener('keydown', event => { if (event.key === 'Enter' && !event.isComposing) { event.preventDefault(); addTag(); } });
    const add = el('div', 'todo-tags__add');
    add.append(name, button('添加标签', addTag));
    box.append(available, add, status, button('完成选择', () => box.close()));
    box.showModal();
}

/** 首读失败不把无数据误作空库；重试成功才挂载可写内容。 */
function mountState(container, context, className, mount) {
    const root = el('section', className);
    const css = el('link');
    css.rel = 'stylesheet'; css.href = new URL('./editor.css', import.meta.url).href;
    container.append(css, root);
    let disposed = false; // 重读时窗口或设置行可能已销毁。
    let cleanup;
    function show(raw, problem) {
        if (disposed) return;
        root.replaceChildren();
        try {
            if (problem) throw new Error(problem);
            cleanup = mount(root, readState(raw));
        } catch (error) {
            const status = el('p', 'todo-feedback', `暂时无法读取待办：${errorText(error)}`);
            status.setAttribute('role', 'alert');
            const retry = button('重试', async () => {
                retry.disabled = true;
                try { show(await context.readSettings()); }
                catch (failure) { show(null, errorText(failure)); }
            });
            root.replaceChildren(status, retry);
        }
    }
    show(context.settings, context.settingsError);
    return () => { disposed = true; cleanup?.(); root.remove(); css.remove(); };
}

/** 独立窗口只接受任务身份，本地草稿不会因为设置广播重挂。 */
export function renderWindow(container, context) {
    if (context.view !== 'task') throw new Error('未知的待办窗口视图');
    const creating = context.windowKey === 'new';
    if (!creating && context.data?.taskId !== context.windowKey) throw new Error('待办窗口标识不一致');
    return mountState(container, context, 'todo-form', (root, initial) => {
        let state = initial;
        let baseline = creating ? null : state.tasks.find(task => task.id === context.windowKey);
        if (!creating && !baseline) throw new Error('该待办已不存在');
        let draft = baseline ? structuredClone(baseline) : newTask(state);
        // 重开已有时间时宁可保护原值，也不推断用户选中的提醒是自动生成的。
        let manualReminder = draft.remindAt !== null || draft.dueAt !== null;
        let revision = 0, saved = 0, timer, pending, disposed = false, deleting = false, mayLeave = false;
        const rows = new Map(); // 保存回显按 ID 复用行，不重建正在输入的控件。
        const status = el('span', 'todo-form__status', creating ? '未保存' : '已保存');
        status.setAttribute('role', 'status');
        const retry = button('重试', () => void flush());
        retry.hidden = true;
        const details = el('p', 'todo-feedback');
        details.hidden = true;
        const discard = button('放弃未保存的更改', () => confirmAction(root, '放弃未保存的更改？', '未保存的输入将丢失，已经保存的内容不受影响。', '放弃并关闭', async () => {
            mayLeave = true;
            try { await context.closeWindow(); } catch (error) { mayLeave = false; failure(error); }
        }), 'danger');
        discard.hidden = true;
        function failure(error) {
            status.textContent = '更改未保存'; retry.hidden = false;
            details.textContent = errorText(error); details.hidden = false;
        }
        function changed() {
            revision++;
            retry.hidden = details.hidden = discard.hidden = true;
            status.textContent = '待保存';
            clearTimeout(timer);
            timer = setTimeout(() => void flush(), 600);
        }
        function flush() {
            clearTimeout(timer);
            if (pending) return pending;
            if (disposed || deleting) return Promise.resolve(false);
            // 同时触发的关闭、重试、防抖只共享一个写入；有新输入则继续排空。
            pending = (async () => {
                while (saved !== revision && !disposed) {
                    if (!draft.title.trim()) throw new Error('请填写任务标题');
                    if (draft.subtasks.some(child => !child.title.trim())) throw new Error('请填写子任务标题');
                    if ([...root.querySelectorAll('input[type="datetime-local"]')].some(control => control.validity.badInput)) throw new Error('请填写完整的日期和时间');
                    const writing = revision;
                    const sent = structuredClone(draft);
                    status.textContent = '正在保存…';
                    state = await saveTask(context, baseline, sent);
                    if (disposed) return false;
                    saved = writing;
                    // 写入期间再次输入时，表单实际见过的是 sent，不是磁盘的新勾选。
                    baseline = writing === revision ? state.tasks.find(task => task.id === sent.id) : sent;
                    if (writing === revision) { draft = structuredClone(baseline); draw(); }
                }
                if (!disposed && saved) {
                    status.textContent = '已保存'; retry.hidden = details.hidden = discard.hidden = true;
                }
                return !disposed;
            })().catch(error => { if (!disposed) failure(error); return false; }).finally(() => { pending = undefined; });
            return pending;
        }
        const done = input('完成待办', 'checkbox');
        done.addEventListener('change', () => { draft.completedAt = done.checked ? Date.now() : null; showCompletion(); changed(); });
        const completion = el('label', 'todo-form__completion');
        completion.append(done, el('span', '', '完成'));
        const completedAt = el('time', 'todo-form__stamp');
        function showCompletion() {
            done.checked = draft.completedAt !== null;
            completedAt.hidden = !done.checked;
            completedAt.textContent = done.checked ? `完成于 ${localMinute(draft.completedAt).replace('T', ' ')}` : '';
        }
        const header = el('header', 'todo-form__header');
        header.append(el('h2', '', creating ? '新建待办' : '编辑待办'), completion, status, retry);
        const title = input('任务标题'); title.required = true;
        const content = el('textarea'); content.setAttribute('aria-label', '任务内容'); content.rows = 3;
        title.addEventListener('input', () => { draft.title = title.value; changed(); });
        content.addEventListener('input', () => { draft.content = content.value; changed(); });
        const severity = segments('严重等级', levels, value => { draft.severity = value; severity.show(value); changed(); });
        function tagsButton(name, get, limit) {
            const node = button('', () => pickTags(root, get().tags, state.tags, limit, tags => { get().tags = tags; show(); changed(); }));
            const show = () => {
                node.textContent = [...get().tags, '＋ 标签'].join(' · ');
                node.setAttribute('aria-label', `${name}：${get().tags.join('、') || '无'}，选择标签`);
            };
            node.classList.add('todo-tags__trigger');
            return { node, show };
        }
        const tags = tagsButton('任务标签', () => draft, 8);
        const due = input('到期时间', 'datetime-local');
        const remind = input('提醒时间', 'datetime-local');
        function timestamp(control) { return control.value ? new Date(control.value).getTime() : null; }
        due.addEventListener('input', () => {
            draft.dueAt = timestamp(due);
            // 清空到期不删除提醒；手动设定（含清空）后不再跟随到期。
            if (draft.dueAt !== null && !manualReminder) {
                draft.remindAt = draft.dueAt - state.prefs.defaultRemindMinutes * 60_000;
                remind.value = localMinute(draft.remindAt);
            }
            changed();
        });
        remind.addEventListener('input', () => { manualReminder = true; draft.remindAt = timestamp(remind); changed(); });
        const pairs = (...nodes) => { const node = el('div', 'todo-form__pair'); node.append(...nodes); return node; };
        const section = el('section', 'todo-form__children');
        const childHead = el('div', 'todo-form__subhead');
        const progress = el('span');
        childHead.append(el('h3', '', '子任务'), progress);
        const columns = el('div', 'todo-form__columns');
        columns.setAttribute('aria-hidden', 'true');
        for (const name of ['完成', '优先级', '标题', '到期时间', '标签']) columns.append(el('span', '', name));
        const list = el('div', 'todo-form__rows');
        function showProgress() { progress.textContent = `${draft.subtasks.filter(child => child.done).length} / ${draft.subtasks.length} 已完成`; }
        function makeRow(id) {
            const get = () => draft.subtasks.find(child => child.id === id);
            const row = el('div', 'todo-form__child');
            const check = input('完成子任务', 'checkbox');
            const level = select('子任务优先级', levels);
            const name = input('子任务标题'); name.placeholder = '子任务标题'; name.required = true;
            const time = input('子任务到期时间', 'datetime-local');
            const labels = tagsButton('子任务标签', get, 4);
            check.addEventListener('change', () => { get().done = check.checked; row.dataset.done = String(check.checked); showProgress(); changed(); });
            level.addEventListener('change', () => { get().severity = level.value; changed(); });
            name.addEventListener('input', () => { get().title = name.value; changed(); });
            time.addEventListener('input', () => { get().dueAt = timestamp(time); changed(); });
            row.append(check, level, name, time, labels.node);
            return { row, name, show() {
                const item = get(); check.checked = item.done; row.dataset.done = String(item.done);
                level.value = item.severity;
                if (name.value !== item.title) name.value = item.title;
                time.value = localMinute(item.dueAt); labels.show();
            } };
        }
        function drawChildren() {
            for (const [id, view] of rows) if (!draft.subtasks.some(child => child.id === id)) { view.row.remove(); rows.delete(id); }
            for (const child of draft.subtasks) {
                if (!rows.has(child.id)) { const view = makeRow(child.id); rows.set(child.id, view); list.append(view.row); }
                rows.get(child.id).show();
            }
            columns.hidden = !draft.subtasks.length;
            showProgress();
        }
        section.append(childHead, columns, list, button('添加子任务', () => {
            if (draft.subtasks.length >= 50) { details.textContent = '最多添加 50 个子任务'; details.hidden = false; return; }
            const child = newSubtask(draft); draft.subtasks.push(child);
            drawChildren(); rows.get(child.id).name.focus(); changed();
        }));
        function draw() {
            if (title.value !== draft.title) title.value = draft.title;
            if (content.value !== draft.content) content.value = draft.content;
            severity.show(draft.severity); tags.show();
            due.value = localMinute(draft.dueAt); remind.value = localMinute(draft.remindAt);
            showCompletion(); drawChildren();
        }
        root.append(header, completedAt, details, discard, field('任务标题 *', title), pairs(field('等级', severity.node), field('标签', tags.node)), pairs(field('到期时间', due), field('提醒时间', remind)), field('任务内容', content), section);
        if (!creating) root.append(button('删除待办', () => confirmAction(root, '删除待办？', `「${draft.title}」和所有子任务将被删除，不能撤销。`, '删除待办', async () => {
            clearTimeout(timer); deleting = true; root.inert = true;
            try {
                if (pending) await pending;
                await removeTask(context, baseline);
                mayLeave = true; await context.closeWindow();
            } catch (error) { details.textContent = `删除或关闭失败：${errorText(error)}`; details.hidden = false; }
            finally { deleting = false; root.inert = false; }
        }), 'danger'));
        draw();
        if (creating) title.focus();
        const unsubscribe = context.onBeforeClose(async () => {
            if (mayLeave) return true;
            if (deleting) return false;
            const ok = await flush();
            if (!ok && !disposed) discard.hidden = false;
            return ok;
        });
        return () => { disposed = true; clearTimeout(timer); unsubscribe(); };
    });
}

/** 同一设置页有独立条目；只修改当前字段，不用挂载时的任务数组覆盖最新数据。 */
function entry(id, name, scope, build) {
    return { id, name, scope, render(container, context) {
        return mountState(container, context, 'todo-settings', (root, initial) => {
            const status = el('span', 'todo-feedback'); status.setAttribute('role', 'status');
            let disposed = false, busy = false;
            const controls = el('div', 'todo-settings__controls');
            async function write(edit, show) {
                if (busy) return false;
                busy = true; controls.inert = true;
                try {
                    const next = await updateState(context, edit);
                    if (!disposed) { show?.(next); status.textContent = '已保存'; }
                    return true;
                } catch (error) { if (!disposed) status.textContent = `未保存：${errorText(error)}`; return false; }
                finally { busy = false; controls.inert = false; }
            }
            build({ initial, controls, root, status, write });
            root.append(controls, status);
            return () => { disposed = true; };
        });
    } };
}

export const settingsEntries = [
    entry('reminder', '默认提醒', 'plugin', ({ initial, controls, write }) => {
        const minutes = input('默认提前分钟数', 'number'); minutes.min = '0'; minutes.max = '10080'; minutes.step = '1';
        minutes.value = String(initial.prefs.defaultRemindMinutes);
        const label = el('label', 'todo-settings__minutes');
        label.append(el('span', '', '提前'), minutes, el('span', '', '分钟'));
        controls.append(label);
        minutes.addEventListener('change', () => { const value = minutes.valueAsNumber; void write(next => { next.prefs.defaultRemindMinutes = value; }); });
    }),
    entry('severity', '默认等级', 'plugin', ({ initial, controls, write }) => {
        const level = select('默认等级', levels); level.value = initial.prefs.defaultSeverity;
        level.addEventListener('change', () => { const value = level.value; void write(next => { next.prefs.defaultSeverity = value; }); });
        controls.append(level);
    }),
    entry('tags', '标签库', 'plugin', ({ initial, controls, root, status, write }) => {
        const tags = el('div', 'todo-tags');
        const name = input('新标签名称'); name.placeholder = '标签名称';
        async function add() {
            const value = name.value.trim();
            if (!value || [...value].length > 16) { status.textContent = '标签名称应为 1 至 16 个字符'; return; }
            await write(next => { if (!next.tags.includes(value)) next.tags.push(value); }, next => { name.value = ''; show(next); });
        }
        function show(next) {
            tags.replaceChildren(...next.tags.map(tag => {
                const chip = button(`${tag} ×`, () => confirmAction(root, `删除标签「${tag}」？`, '标签将从所有待办和子任务中移除。', '删除标签', () => write(state => {
                    state.tags = state.tags.filter(value => value !== tag);
                    for (const task of state.tasks) for (const item of [task, ...task.subtasks]) item.tags = item.tags.filter(value => value !== tag);
                }, show)));
                chip.setAttribute('aria-label', `删除标签 ${tag}`); return chip;
            }));
        }
        name.addEventListener('keydown', event => { if (event.key === 'Enter' && !event.isComposing) { event.preventDefault(); void add(); } });
        const row = el('div', 'todo-tags__add'); row.append(name, button('＋ 新建标签', add));
        controls.append(row, tags); show(initial);
    }),
    entry('collapsed-count', '收起时显示条数', 'island', ({ initial, controls, write }) => {
        const choices = segments('收起时显示条数', [3, 5, 10].map(n => [n, String(n)]), value => void write(next => { next.prefs.collapsedCount = value; }, next => choices.show(next.prefs.collapsedCount)));
        choices.show(initial.prefs.collapsedCount); controls.append(choices.node);
    }),
];
