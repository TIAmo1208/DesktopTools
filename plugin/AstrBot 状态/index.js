/**
 * AstrBot 状态
 * 1. 通过宿主能力通道 invoke_component_capability 探活，不走平铺原生命令
 * 2. ok 稳定 15s 轮询，连续失败 3 次退避到 60s，成功一次立即复位
 * 3. 组件卸载时清掉全部定时器与监听，不留后台轮询
 * 4. 就绪时长由 startTime 每次展示时现算，不是 AstrBot 进程 uptime
 */

const PLUGIN_ID = "plugin.astrbot-status"; // 清单标识，同时作为样式表键。
const CAPABILITY = "probe-service-status"; // 清单声明的唯一能力。
const STEADY_POLL_MS = 15_000; // 探测稳定时的轮询间隔。
const BACKOFF_POLL_MS = 60_000; // 连续失败后的退避间隔。
const FAILURES_BEFORE_BACKOFF = 3; // 连续失败达到该次数后换成退避间隔。
const STALE_MS = 60_000; // 最近一次判定超过该时长即视为数据过期。
const TICK_MS = 1_000; // 就绪分钟数与过期判定每秒重算一次。

// 四档结论的文案与状态点样式类；unknown 不是离线，也不上绿色。
const LEVEL_VIEW = {
    ok: { label: "AstrBot 在线", levelClass: "astrbot-status__level--ok" },
    off: { label: "离线", levelClass: "astrbot-status__level--off" },
    warn: { label: "探测被拒", levelClass: "astrbot-status__level--warn" },
    unknown: { label: "未知", levelClass: "astrbot-status__level--unknown" },
};

const styleUrl = new URL("./style.css", import.meta.url).href; // 同级样式表资产。
// 纯逻辑测试在 Node 里直接导入本模块，那里没有 document。
if (typeof document !== "undefined" && !document.querySelector(`link[data-plugin-style="${PLUGIN_ID}"]`)) {
    const link = document.createElement("link");
    link.rel = "stylesheet";
    link.href = styleUrl;
    link.setAttribute("data-plugin-style", PLUGIN_ID);
    document.head.appendChild(link);
}

/**
 * 把 startTime 换算成「已就绪 N 分」。
 * startTime 是 AstrBot 核心加载完成时刻，不是进程启动时刻，所以文案只说就绪时长。
 */
export function readyLabel(startTime, nowMs) {
    const minutes = Math.max(0, Math.floor((nowMs - startTime * 1000) / 60_000)); // 时钟回拨时压到 0。
    if (minutes < 1) return "刚刚就绪";
    if (minutes < 60) return `已就绪 ${minutes} 分`;
    if (minutes < 60 * 24) return `已就绪 ${Math.floor(minutes / 60)} 小时 ${minutes % 60} 分`;
    return `已就绪 ${Math.floor(minutes / (60 * 24))} 天 ${Math.floor((minutes % (60 * 24)) / 60)} 小时`;
}

/** 最近一次判定距今超过 60s 即数据过期。 */
export function isStale(checkedAt, nowMs) {
    return nowMs - checkedAt * 1000 > STALE_MS;
}

/** 一次结论的展示状态：过期时状态点变灰并追加「数据过期」。 */
export function viewOf(probe, nowMs) {
    const view = LEVEL_VIEW[probe.level] ?? LEVEL_VIEW.unknown; // 契约外的档位一律按未知展示。
    const stale = isStale(probe.checkedAt, nowMs); // 过期与档位无关，一律灰点。
    const detail = probe.level === "ok" && probe.startTime !== null ? readyLabel(probe.startTime, nowMs) : ""; // 只有 ok 带就绪时长。
    const text = [view.label, detail].filter(Boolean).join(" · ");
    return { text, levelClass: stale ? LEVEL_VIEW.unknown.levelClass : view.levelClass };
}

/** 连续失败满 3 次后退避到 60s；成功一次由调用方清零失败计数即复位。 */
export function pollDelay(failures) {
    return failures >= FAILURES_BEFORE_BACKOFF ? BACKOFF_POLL_MS : STEADY_POLL_MS;
}

/** 探测失败（命令被拒、实例未就绪）按 unknown 记账，不当成离线。 */
export function probeFromError(_error, nowMs) {
    return { level: "unknown", startTime: null, checkedAt: Math.floor(nowMs / 1000) };
}

/**
 * 通过宿主能力通道探测；插件名与能力名固定，实例由宿主按当前格子传入。
 * 页面桥把 POST body 原样作为命令参数对象交给 dispatch_native_command，
 * 因此这里按对象传 { instanceId, capability } 就是对桥的完整要求，不走平铺原生命令。
 */
export function createCapabilityProbe(bridge, instanceId) {
    return () => bridge.invoke("invoke_component_capability", { instanceId, capability: CAPABILITY });
}

/**
 * 记录尚未触发的延时任务，停止时按全局定时器清理。
 * 任务一旦触发即从集合移除，卸载后不会残留也不会误清其它定时器。
 */
export function createPendingTimers() {
    const timers = new Set(); // 尚未触发的定时器。
    return {
        after(callback, delay) {
            const id = setTimeout(() => {
                timers.delete(id);
                callback();
            }, delay);
            timers.add(id);
            return id;
        },
        clearAll() {
            for (const id of timers) clearTimeout(id);
            timers.clear();
        },
    };
}

/**
 * 安装一次探测循环，返回停止函数。
 * 拿到结论即按成功复位为 15s，命令失败累加后按档退避；停止后清掉未触发的定时器。
 */
export function startProbeLoop(invoke, onProbe, onFailure, timers = createPendingTimers()) {
    let failures = 0; // 连续失败次数。
    let stopped = false; // 停止后拒绝迟到结果。

    /** 跑一次探测并按当前失败次数排下一次。 */
    const run = async () => {
        if (stopped) return;
        try {
            const result = await invoke(); // 能力通道的最小结构化结果。
            if (stopped) return;
            failures = 0;
            onProbe(result.payload);
        } catch (error) {
            if (stopped) return;
            failures += 1;
            onFailure(error);
        }
        if (!stopped) timers.after(run, pollDelay(failures));
    };

    timers.after(run, 0); // 挂载后立即探测一次。
    return () => {
        stopped = true;
        timers.clearAll();
    };
}

export default {
    renderTile(container, context) {
        container.innerHTML = `
            <div class="astrbot-status">
                <div class="astrbot-status__status">
                    <span class="astrbot-status__dot astrbot-status__level--unknown" id="astrbot-status-dot"></span>
                    <span class="astrbot-status__text" id="astrbot-status-text">检测中</span>
                    <span class="astrbot-status__stale" id="astrbot-status-stale" hidden>数据过期</span>
                </div>
            </div>
        `;

        const tileEl = container.querySelector(".astrbot-status");
        const dotEl = container.querySelector("#astrbot-status-dot");
        const textEl = container.querySelector("#astrbot-status-text");
        const staleEl = container.querySelector("#astrbot-status-stale");

        let probe = null; // 最近一次结论；null 表示还没拿到。
        let probeTimer = createPendingTimers(); // 未触发的轮询定时器。
        let tickTimer = null; // 每秒重算就绪时长与过期判定。
        let visible = true; // 观测器回调前的初值，挂载即探测。
        let started = false; // 探测与秒级重算是否在跑。
        let disposed = false; // 卸载后拒绝重新启动。

        /** 按当前时间重绘，让轮询间隔内显示的时长保持准确。 */
        const render = () => {
            if (!probe) return;
            const now = Date.now(); // 每次展示都现算。
            const view = viewOf(probe, now);
            dotEl.className = `astrbot-status__dot ${view.levelClass}`;
            textEl.textContent = view.text;
            staleEl.hidden = !isStale(probe.checkedAt, now);
        };

        /** 停掉探测循环和秒级重算，卸载与不可见时共用。 */
        const stop = () => {
            probeTimer.clearAll();
            if (tickTimer !== null) clearInterval(tickTimer);
            tickTimer = null;
            started = false;
        };

        /** 开始探测与秒级重算；不可见、已启动或已卸载时不重复启动。 */
        const start = () => {
            if (started || !visible || disposed) return;
            // 能力通道按实例授权，插件不调平铺原生命令。
            const invoke = createCapabilityProbe(window.__DESKTOP_TOOLS_NATIVE__, context.instanceId);
            probeTimer = createPendingTimers();
            startProbeLoop(
                invoke,
                (payload) => {
                    probe = payload;
                    render();
                },
                (error) => {
                    console.warn("AstrBot 状态：探活失败", error);
                    probe = probeFromError(error, Date.now());
                    render();
                },
                probeTimer,
            );
            tickTimer = setInterval(render, TICK_MS);
            started = true;
        };

        // 格子滚出视口或所在窗口隐藏时暂停刷新，重新可见后立刻补一次。
        const stopObserve = new IntersectionObserver((entries) => {
            const next = entries[entries.length - 1]?.isIntersecting ?? true; // 以最新一条为准。
            if (next === visible) return;
            visible = next;
            if (visible) start();
            else stop();
        });
        if (tileEl) stopObserve.observe(tileEl);

        /** 页面隐藏时不刷新，回到前台立即补一次。 */
        const handleVisibility = () => {
            const next = document.visibilityState !== "hidden"; // 隐藏时不刷新。
            if (next === visible) return;
            visible = next;
            if (visible) start();
            else stop();
        };
        document.addEventListener("visibilitychange", handleVisibility);

        start();

        return () => {
            disposed = true;
            stop();
            stopObserve.disconnect();
            document.removeEventListener("visibilitychange", handleVisibility);
        };
    },
};
