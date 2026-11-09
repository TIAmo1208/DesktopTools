import { useEffect, useLayoutEffect, useRef, useState, type PointerEvent } from "react";
import { getDesktopHostSnapshot } from "../../bridge/desktopHost";
import { beginIslandDrag, listenIslandSnapshots, saveIslandCardOrder, setIslandState } from "../../bridge/island";
import { executeMusicAction } from "../../bridge/music";
import { listenSpectrum } from "../../bridge/audio";
import type { IslandRect, IslandSnapshot, IslandStateRequest } from "../../contracts/island";
import { PluginSlot, pluginTileType } from "../components/PluginSlot";
import { useMusicState, positionAt, lyricIndex, lyricTickDelay } from "../music/musicSource";
import { usePreferences } from "../settings/PreferencesProvider";
import { islandCornerRadius, islandGeometry, moveIslandCard, visibleIslandRect } from "./islandGeometry";
import {
    ISLAND_HOVER_DELAY_MS,
    ISLAND_DRAG_DELAY_MS,
    ISLAND_NO_DRAG,
    islandDragIntent,
    islandSortTarget,
    islandDragMoved,
    islandAfterDrag,
    cancelIslandHover,
    islandHoverTarget,
} from "./islandInteraction";
import { ISLAND_MOTION_MS, ISLAND_FRAME_MS, islandMotionFrame, islandUnionFrame } from "./islandMotion";
import "./island.css";
const PAINTED_STYLE_PROPERTIES = ["position", "left", "top", "inline-size", "block-size", "max-inline-size", "min-block-size", "overflow", "border-radius", "flex"] as const; // paint 写入的内联属性，恢复时只清这些。

type IslandMode = IslandStateRequest["state"]; // 三态互斥。

/** 岛只消费共享音乐事实，计时器只推进可见投影，不读取媒体或请求歌词。 */
const IslandMusic = ({ mode, side }: { mode: IslandMode; side: boolean }) => { // side：左右贴边，hover 的播放控制改到第二排。
    const music = useMusicState();
    const [now, setNow] = useState(() => performance.now()); // 单调时钟用于播放位置投影。
    const [busy, setBusy] = useState(false); // 同一时刻只提交一个播放命令。
    const [failure, setFailure] = useState(false); // 命令失败留在固定内容槽，不撑高。
    const [failedArtwork, setFailedArtwork] = useState<string>(); // 失效图片不显示破图。
    const media = music.snapshot;
    const position = positionAt(music, now);
    const index = lyricIndex(music.lyrics, position);
    const status = !media
        ? "未在播放"
        : media.playback === "playing"
          ? "正在播放"
          : media.playback === "paused"
            ? "已暂停"
            : media.playback === "stopped"
              ? "已停止"
              : "播放状态未知";
    const fallback = [media?.title, media?.artist].filter(Boolean).join(" · ") || status;
    const line = music.lyrics[index]?.text || fallback;
    useEffect(() => {
        let timer: ReturnType<typeof setTimeout> | undefined;
        const refresh = () => {
            clearTimeout(timer);
            if (document.hidden || window.__DESKTOP_TOOLS_NATIVE__?.isRefreshPaused?.()) return;
            const current = performance.now();
            setNow(current);
            if (!music.timeline?.advancing) return;
            const projected = positionAt(music, current);
            if (projected >= music.timeline.durationMs) return;
            const delay = mode === "expanded" ? 250 : lyricTickDelay(music.lyrics, projected, music.timeline.durationMs);
            if (delay !== undefined) timer = setTimeout(refresh, delay);
        };
        refresh();
        document.addEventListener("visibilitychange", refresh);
        window.addEventListener("desktop-refresh-changed", refresh);
        return () => {
            clearTimeout(timer);
            document.removeEventListener("visibilitychange", refresh);
            window.removeEventListener("desktop-refresh-changed", refresh);
        };
    }, [music, mode]);
    const handleAction = async (action: "previous" | "play" | "pause" | "next") => {
        if (!media || busy) return;
        setBusy(true);
        try {
            await executeMusicAction(action, media.sessionId);
            setFailure(false);
        } catch (error) {
            console.warn("岛音乐控制失败", error);
            setFailure(true);
        } finally {
            setBusy(false);
        }
    };
    const controls = (
        <div className="island-music__controls">
            {(
                [
                    ["previous", "上一首", media?.canPrevious, "M10 4 3 8l7 4V4ZM2 4v8"],
                    [
                        media?.playback === "playing" ? "pause" : "play",
                        media?.playback === "playing" ? "暂停" : "播放",
                        media?.canPlayPause,
                        media?.playback === "playing" ? "M5 3v10M11 3v10" : "m5 3 8 5-8 5V3Z",
                    ],
                    ["next", "下一首", media?.canNext, "m6 4 7 4-7 4V4Zm8 0v8"],
                ] as const
            ).map(([action, label, enabled, path]) => (
                <button type="button" data-button="icon" key={action} aria-label={label} disabled={!enabled || busy} onClick={() => void handleAction(action)}>
                    <svg viewBox="0 0 16 16" aria-hidden="true">
                        <path d={path} />
                    </svg>
                </button>
            ))}
        </div>
    );
    return (
        <div className="island-music">
            <div className="island-music__header">
                <div className="island-music__artwork" aria-hidden="true">
                    {media?.artworkUrl && failedArtwork !== media.artworkUrl ? (
                        <img
                            src={media.artworkUrl}
                            alt=""
                            draggable={false}
                            onDragStart={(event) => event.preventDefault()}
                            onError={() => setFailedArtwork(media.artworkUrl)}
                        />
                    ) : (
                        <span>♪</span>
                    )}
                </div>
                {mode === "collapsed" ? (
                    <p className="island-music__line" title={line}>
                        <span className="island-music__reserve" aria-hidden="true">
                            {fallback}
                        </span>
                        <span className="island-music__current">{line}</span>
                    </p>
                ) : (
                    <div className="island-music__info">
                            <div className="island-music__metadata">
                                <p className="island-music__title" title={media?.title}>
                                    {media?.title || status}
                                </p>
                                <p className="island-music__artist" title={media?.artist}>
                                    {media?.artist || "\u00a0"}
                                </p>
                                <p className="island-music__source" title={media?.source}>
                                    {failure ? "控制失败，请重试" : media?.source || "\u00a0"}
                                </p>
                            </div>
                            {mode === "expanded" ? controls : null}
                    </div>
                )}
                {mode === "hover" && !side ? controls : null}
            </div>
            {mode === "hover" && side ? <div className="island-music__second-row">{controls}</div> : null}
            {mode === "expanded" ? (
                <>
                    <div className="island-music__timeline">
                        <progress aria-label="播放进度" max={music.timeline?.durationMs || 1} value={position} />
                        <div>
                            <time>{formatTime(position)}</time>
                            <time>{formatTime(music.timeline?.durationMs ?? 0)}</time>
                        </div>
                    </div>
                    <IslandSpectrum playing={media?.playback === "playing"} />
                    <div className="island-music__lyrics" aria-live="off">
                        {[-1, 0, 1].map((offset) => (
                            <p key={offset} data-current={offset === 0} title={music.lyrics[index + offset]?.text}>
                                {offset === 0 ? line : music.lyrics[index + offset]?.text || "\u00a0"}
                            </p>
                        ))}
                    </div>
                </>
            ) : null}
        </div>
    );
};

/** 时间格式固定一行，不为无会话另造布局。 */
const formatTime = (milliseconds: number) => `${Math.floor(milliseconds / 60000)}:${String(Math.floor(milliseconds / 1000) % 60).padStart(2, "0")}`;

/** 岛独立挂载，不装配工作区、格子外壳或设置入口。 */
export const IslandPage = () => {
    const { preferences, onPreferences, animationsEnabled } = usePreferences();
    const [snapshot, setSnapshot] = useState<IslandSnapshot>();
    const [failure, setFailure] = useState<string>();
    const [mode, setMode] = useState<IslandMode>("collapsed");
    const modeRef = useRef(mode); // 延时回调读取最新状态，不能覆盖键盘或点击后的展开。
    modeRef.current = mode;
    const [work, setWork] = useState<IslandRect>();
    const root = useRef<HTMLElement>(null); // 只测量岛自身。
    const palette = useRef<HTMLDivElement>(null); // 只读取现有主题的围栏令牌，不呈现工作区。
    useEffect(() => {
        const readPalette = () => {
            if (!root.current || !palette.current) return;
            // 先移除上次复制值，防止撤下主题后探针继承旧色形成自回路。
            for (const name of ["--fence-surface", "--fence-border", "--fence-text"]) root.current.style.removeProperty(name);
            const tokens = getComputedStyle(palette.current);
            for (const name of ["--fence-surface", "--fence-border", "--fence-text"]) {
                const value = tokens.getPropertyValue(name).trim();
                if (value) root.current.style.setProperty(name, value);
                else root.current.style.removeProperty(name);
            }
        };
        const scheme = matchMedia("(prefers-color-scheme: dark)"); // 系统变化可能只改变主题的媒体查询。
        const observer = new MutationObserver(readPalette); // 同时覆盖主题表替换与深浅模式属性更新。
        observer.observe(document.head, { childList: true, subtree: true, attributes: true, attributeFilter: ["href"] });
        observer.observe(document.documentElement, { attributes: true, attributeFilter: ["data-desktop-theme"] });
        const handleLoad = (event: Event) => {
            if (event.target instanceof HTMLLinkElement && event.target.hasAttribute("data-theme-plugin")) readPalette();
        };
        document.addEventListener("load", handleLoad, true);
        scheme.addEventListener("change", readPalette);
        readPalette();
        return () => {
            observer.disconnect();
            document.removeEventListener("load", handleLoad, true);
            scheme.removeEventListener("change", readPalette);
        };
    }, [preferences]); // 既有偏好事件由 Provider 接收，无需第二份事件订阅。
    const mounted = useRef(false); // 串行原生提交只受页面生命周期约束，不被测量 effect 重建取消。
    const hoverTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
    const dragHold = useRef<ReturnType<typeof setTimeout> | undefined>(undefined); // 按下到进入原生拖动之间的长按计时。
    const dragging = useRef(false); // 原生移动循环期间不切 hover。
    const cardDrag = useRef<string | undefined>(undefined); // 卡片排序身份，不保存布局。
    const sortDragAllowed = useRef(false); // 保留按下的真实来源，阻止控件借父卡片发起 HTML 拖动。
    const geometryKey = useRef(""); // 同一实测结果不重复 IPC。
    const currentGeometry = useRef<IslandStateRequest | undefined>(undefined); // 最近实际完成的原生区域。
    const collapsedBox = useRef<IslandStateRequest | undefined>(undefined); // 最近胶囊态区域，hover 窗口与它取并集避免指针离开。
    const skipMotionOnce = useRef(false); // 拖动后的首次测量跳过动画，避免跨边时先放大到并集再收缩。
    const finishMotion = useRef<() => Promise<void>>(async () => undefined); // 拖动前先结束形变，防止动画帧移动原生拖动中的窗口。
    const gesture = useRef<{ id: number; x: number; y: number } | undefined>(undefined); // hover 单击候选，按位移升级为拖动。
    const pointerInside = useRef(false); // 延时回调只读取最新边界结果。
    const suppressClick = useRef(false); // 原生拖动释放不得再次当成单击展开。
    const [dragRevision, setDragRevision] = useState(0); // 拖动结束即使仍为同态也重测新位置。
    const pendingOffset = useRef<number | undefined>(undefined); // 本次拖动的新偏移；权威偏好事件前先按它重测，避免回跳。
    const paintedGeometry = useRef<{ state: IslandStateRequest["state"]; width: number; height: number } | undefined>(undefined); // 最近一次已写入的窗口布局，用于忽略 paint 自己触发的 ResizeObserver。
    const [dragActive, setDragActive] = useState(false); // 原生移动循环期间的拖动反馈。
    const [motionActive, setMotionActive] = useState(false); // 并集动画期间隐藏角部补面，避免弧面脱离卡片漂浮。
    const capsuleHeight = useRef<number | undefined>(undefined); // 胶囊态卡片高度；左右贴边的 hover 在测量时直接写入，不经过 React 状态以免换边时用到旧值。
    const capsuleOrientation = useRef<"horizontal" | "vertical" | undefined>(undefined); // 胶囊高度对应的边方向；同方向换边/换形态可复用。
    const submission = useRef(Promise.resolve()); // 原生提交串行，防止快速切态乱序。
    const sorting = useRef(false); // 不让相邻两次保存用同一旧顺序。
    const recovering = useRef(false); // fail-closed 最多自动补一次常态，持续失败不循环重试。
    useEffect(() => {
        let active = true;
        mounted.current = true;
        const handleFailure = (error: unknown) => {
            if (active) setFailure(error instanceof Error ? error.message : String(error));
        };
        const subscription = listenIslandSnapshots((next) => {
            if (active) {
                setSnapshot(next);
                setFailure(undefined);
            }
        }, handleFailure);
        void subscription.catch(handleFailure);
        void getDesktopHostSnapshot()
            .then((desktop) => {
                const monitor = desktop.hosts.find((host) => host.monitor.primary)?.monitor;
                if (active && monitor)
                    setWork({
                        left: monitor.workX / monitor.scaleFactor,
                        top: monitor.workY / monitor.scaleFactor,
                        width: monitor.workWidth / monitor.scaleFactor,
                        height: monitor.workHeight / monitor.scaleFactor,
                    });
            })
            .catch(handleFailure);
        return () => {
            active = false;
            mounted.current = false;
            cancelIslandHover(hoverTimer);
            void subscription.then((stop) => stop()).catch(() => undefined);
        };
    }, []);
    const edge = preferences?.islandEdge ?? "";
    const vertical = mode === "collapsed" && (edge === "left" || edge === "right"); // 仅胶囊态在左右贴边时竖排；hover 与展开保持水平卡片。
    const instances = snapshot?.instances ?? [];
    const visible = mode === "expanded" ? instances : instances.slice(0, 1);
    const edgeKey = useRef(""); // 上次测量的贴边方向，变化时丢弃旧几何缓存。
    const formKey = useRef(""); // 上次测量的岛形态，变化时丢弃旧胶囊高度与并集缓存。
    useLayoutEffect(() => {
        if (!preferences || !work || !root.current) return;
        let active = true;
        let frame = 0;
        let motionFrame = 0;
        let wakeMotion: (() => void) | undefined;
        let animating = false;
        let forceFinish = false;
        let finish: Promise<void> = Promise.resolve();
        const savedStyles = new Set<HTMLElement>(); // 完成、取消与失败都还原被 paint 改写的内联属性。
        const saveStyle = (element: HTMLElement) => {
            savedStyles.add(element);
        };
        const restoreStyles = () => {
            for (const element of savedStyles) {
                // 只移除 paint 写入的 CSSOM 属性，保留 React/主题写在 style 上的其它自定义属性；生产 CSP 不拦截属性 setter。
                for (const property of PAINTED_STYLE_PROPERTIES) element.style.removeProperty(property);
            }
            savedStyles.clear();
        };
        const nextEdge = edge;
        const nextForm = preferences.islandForm ?? "capsule";
        if (edgeKey.current !== nextEdge || formKey.current !== nextForm) {
            // 切换贴边或形态时旧并集与旧完成几何不再适用；胶囊高度只在沿边方向变化时失效。
            const nextOrientation = nextEdge === "left" || nextEdge === "right" ? "horizontal" : "vertical";
            const orientationChanged = capsuleOrientation.current !== undefined && capsuleOrientation.current !== nextOrientation;
            edgeKey.current = nextEdge;
            formKey.current = nextForm;
            collapsedBox.current = undefined;
            geometryKey.current = "";
            currentGeometry.current = undefined;
            if (orientationChanged) capsuleHeight.current = undefined; // 上下与左右的内容高度不可复用；同方向换边/换形态保留。
            capsuleOrientation.current = nextOrientation;
            pendingOffset.current = undefined; // 旧边/旧形态的拖动落点不再适用。
        }
        const measureNow = (force = false) => {
            if (animating || dragging.current) return;
            const painted = paintedGeometry.current;
            if (!force && root.current && painted) {
                const box = root.current.getBoundingClientRect();
                // paint 写入的根尺寸会触发 ResizeObserver；它不改变内容，不能借机 restore 成自然布局。
                if (mode === painted.state && Math.abs(box.width - painted.width) < 0.6 && Math.abs(box.height - painted.height) < 0.6) return;
            }
            cancelAnimationFrame(frame);
            restoreStyles(); // 动画被打断或切态竞争时清掉上一轮内联尺寸，测量必须基于真实文档流。
            {
                if (!active || !root.current || dragging.current) return;
                const single = root.current.querySelector<HTMLElement>(".island-card");
                // 左右贴边的 hover 沿用当前胶囊高度；在测量前直接写入，避免经 React 状态延迟一帧造成高度错乱。
                if (mode === "hover" && (edge === "left" || edge === "right") && single && capsuleHeight.current !== undefined) {
                    saveStyle(single);
                    single.style.blockSize = `${capsuleHeight.current}px`;
                }
                const origin = root.current.getBoundingClientRect();
                // 刘海非展开态由根 padding 让出圆弧区；测量时换回内容坐标，几何函数再统一加回 12 DIP。
                const notchPad = preferences.islandForm === "notch" && mode !== "expanded" ? 12 : 0;
                const horizontalEdge = edge === "left" || edge === "right";
                const measured = Array.from(root.current.querySelectorAll<HTMLElement>(".island-card")).flatMap((card) => {
                    const rect = card.getBoundingClientRect();
                    return visibleIslandRect(
                        {
                            left: rect.left - origin.left - (horizontalEdge ? 0 : notchPad),
                            top: rect.top - origin.top - (horizontalEdge ? notchPad : 0),
                            width: rect.width,
                            height: rect.height,
                        },
                        origin.height,
                    );
                });
                if (!measured.length) return;
                try {
                    const radius = mode === "collapsed" ? 22 : mode === "hover" ? 16 : preferences.islandRadius;
                    const pending = pendingOffset.current;
                    // 默认空边按顶部居中；拖动返回的显式偏移必须按有效边 top 计算，不能在中点丢掉。
                    const geometryEdge = pending !== undefined && edge === "" ? "top" : edge;
                    let request = islandGeometry(mode, measured, radius, work, geometryEdge, pending ?? preferences.islandOffset, preferences.islandForm);
                    // hover 窗口与胶囊态取并集：指针停在原胶囊范围任意位置都不会离开窗口，避免左右贴边来回切态。
                    if (mode === "hover" && collapsedBox.current) {
                        request = { ...islandUnionFrame(request, collapsedBox.current, 0, edge, work), state: "hover" };
                    }
                    if (mode === "collapsed") {
                        collapsedBox.current = request;
                        capsuleHeight.current = measured[0]!.height; // 只记卡片高度；窗口盒在刘海下另含 24 的圆弧区。
                        capsuleOrientation.current = horizontalEdge ? "horizontal" : "vertical";
                    }
                    const key = JSON.stringify(request);
                    const element = root.current;
                    const cards = Array.from(element.querySelectorAll<HTMLElement>(".island-card"));
                    const from = currentGeometry.current;
                    const animate = animationsEnabled && from && from.state !== request.state && !dragging.current && !skipMotionOnce.current;
                    const paint = (value: IslandStateRequest, _elapsed: number) => {
                        paintedGeometry.current = { state: value.state, width: value.windowBox.width, height: value.windowBox.height };
                        saveStyle(element);
                        Object.assign(element.style, {
                            position: "relative",
                            inlineSize: `${value.windowBox.width}px`,
                            blockSize: `${value.windowBox.height}px`,
                            maxInlineSize: "none",
                            overflow: "clip",
                        });
                        const first = value.cards[0];
                        const arcFit = value.state !== "expanded" && first
                            ? (edge === "left" || edge === "right"
                                ? Math.abs(first.rect.top - 12) < 0.5 && Math.abs(first.rect.left) < 0.5
                                : Math.abs(first.rect.left - 12) < 0.5 && Math.abs(first.rect.top) < 0.5)
                            : true;
                        element.dataset.arcFit = arcFit ? "true" : "false";
                        cards.forEach((card, index) => {
                            const shape = value.cards[index];
                            if (!shape) return;
                            saveStyle(card);
                            Object.assign(card.style, {
                                position: "absolute",
                                left: `${shape.rect.left}px`,
                                top: `${shape.rect.top}px`,
                                inlineSize: `${shape.rect.width}px`,
                                blockSize: `${shape.rect.height}px`,
                                maxInlineSize: "none",
                                minBlockSize: "0",
                                borderRadius: islandCornerRadius(shape.radius, preferences.islandForm ?? "capsule", edge, value.state === "expanded"),
                            });
                        });
                    };
                    if (key === geometryKey.current) {
                        // 同一几何被重新测量时仍要把窗口布局写回；否则并集窗口/刘海内缩会退回自然布局。
                        if (mode === "hover") paint(request, ISLAND_MOTION_MS); // 重放当前目标并集，不能重放可能被中断的动画中间帧。
                        return;
                    }
                    geometryKey.current = key;
                    skipMotionOnce.current = false;
                    if (animate) {
                        animating = true;
                        // 内容先按最终态实测，再冻结其尺寸；动画只改变外框，不逐帧排版文字。
                        cards.forEach((card) => {
                            const content = card.querySelector<HTMLElement>(".island-card__content");
                            if (content) {
                                const box = content.getBoundingClientRect();
                                saveStyle(content);
                                Object.assign(content.style, { inlineSize: `${box.width}px`, blockSize: `${box.height}px`, flex: "none" });
                            }
                        });
                        paint(islandMotionFrame(from, request, 0, edge, work), 0);
                    }
                    const commit = async (value: IslandStateRequest) => {
                        await setIslandState(value);
                        currentGeometry.current = value;
                    };
                    finish = submission.current
                        .catch(() => undefined)
                        .then(async () => {
                            if (!active || !mounted.current) return;
                            try {
                                const startGeometry = currentGeometry.current ?? from; // 等旧提交落定后再取衔接点。
                                if (animate && startGeometry) {
                                    // 先把原生窗口一次放大到两态并集，内容保持原位置；动画只在固定窗口内移动内容。
                                    setMotionActive(true);
                                    const unionFirst = islandUnionFrame(startGeometry, request, 0, edge, work);
                                    paint(unionFirst, 0); // 先按当前窗口画出内容，再放大窗口，避免窗口先行出现未定位内容的一帧。
                                    await commit(unionFirst);
                                    await new Promise<void>((resolve) => {
                                        motionFrame = requestAnimationFrame(() => resolve());
                                    });
                                    if (!active) return;
                                    const started = performance.now();
                                    let last = -Infinity;
                                    let previousKey = "";
                                    while (active && !forceFinish) {
                                        const elapsed = await new Promise<number>((resolve) => {
                                            wakeMotion = () => resolve(ISLAND_MOTION_MS);
                                            motionFrame = requestAnimationFrame((now) => {
                                                wakeMotion = undefined;
                                                resolve(now - started);
                                            });
                                        });
                                        if (!active || elapsed >= ISLAND_MOTION_MS || forceFinish) break;
                                        if (elapsed - last < ISLAND_FRAME_MS) continue;
                                        last = elapsed;
                                        const value = islandUnionFrame(startGeometry, request, elapsed, edge, work);
                                        const frameKey = JSON.stringify(value);
                                        if (frameKey !== previousKey) {
                                            await commit(value);
                                            if (!active) break;
                                            paint(value, elapsed);
                                            previousKey = frameKey;
                                        }
                                    }
                                }
                                if (active) {
                                    // 展开态 root 是滚动容器：不写回固定内联几何，保留自然流和 scrollTop。
                                    if (request.state !== "expanded") paint(request, ISLAND_MOTION_MS);
                                    await commit(request);
                                    recovering.current = false;
                                } // 有界动画结束必须补最终态，不受去重键影响。
                            } catch (error) {
                                currentGeometry.current = undefined;
                                geometryKey.current = "";
                                console.warn("岛状态提交失败", error);
                                if (active && mounted.current) {
                                    setMode("collapsed"); // 原生仍执行 fail-closed，恢复从常态开始。
                                    if (!recovering.current) {
                                        recovering.current = true;
                                        setDragRevision((value) => value + 1);
                                    }
                                }
                            } finally {
                                animating = false;
                                if (mounted.current) setMotionActive(false); // 动画结束或中断后恢复角部补面。
                                // 成功结束回到自然布局，让常态胶囊继续随内容伸缩、展开态恢复 CSS 滚动。
                                restoreStyles();
                            }
                        });
                    submission.current = finish;
                    finishMotion.current = async () => {
                        forceFinish = true;
                        cancelAnimationFrame(motionFrame);
                        wakeMotion?.();
                        await finish;
                    };
                } catch (error) {
                    console.warn("岛几何测量失败", error);
                }
            }
        };
        const measure = (force = false) => {
            cancelAnimationFrame(frame);
            frame = requestAnimationFrame(() => measureNow(force));
        };
        const observer = new ResizeObserver(() => measure(false));
        observer.observe(root.current);
        const handleScroll = () => measure(true); // 展开态滚动必须重新计算可见卡片，不受自触发 resize 跳过影响。
        root.current.addEventListener("scroll", handleScroll);
        for (const card of root.current.querySelectorAll(".island-card")) observer.observe(card);
        measureNow(true); // 首帧绘制前完成目标实测与内容冻结，不先闪出最终布局。
        const element = root.current;
        return () => {
            active = false;
            element.removeEventListener("scroll", handleScroll);
            observer.disconnect();
            cancelAnimationFrame(frame);
            cancelAnimationFrame(motionFrame);
            wakeMotion?.();
            restoreStyles();
        };
    }, [mode, snapshot, preferences, work, edge, animationsEnabled, dragRevision]);
    useEffect(() => {
        const pending = pendingOffset.current;
        if (pending === undefined || !preferences) return;
        if ((preferences.islandEdge || "top") === (edge || "top") && Math.abs(preferences.islandOffset - pending) < 0.01) pendingOffset.current = undefined; // 权威落点已到，不再压住后续偏移。
    }, [preferences, edge]);
    const scheduleHover = (inside: boolean) => {
        cancelIslandHover(hoverTimer);
        const target = islandHoverTarget(mode, inside, dragging.current || Boolean(gesture.current) || Boolean(cardDrag.current));
        if (!target) return;
        hoverTimer.current = setTimeout(() => {
            if (
                pointerInside.current === inside &&
                islandHoverTarget(modeRef.current, inside, dragging.current || Boolean(gesture.current) || Boolean(cardDrag.current)) === target
            )
                setMode(target);
        }, ISLAND_HOVER_DELAY_MS);
    };
    const handleLeave = () => {
        pointerInside.current = false;
        cancelIslandHover(hoverTimer);
        cancelDragHold();
        if (dragging.current || gesture.current || cardDrag.current) return;
        if (mode === "expanded") setMode("collapsed");
        else scheduleHover(false);
    };
    const handleEnter = () => {
        pointerInside.current = true;
        scheduleHover(true);
    };
    /** 取消尚未到点的长按拖动，释放或移动后不得再接管鼠标。 */
    const cancelDragHold = () => {
        clearTimeout(dragHold.current);
        dragHold.current = undefined;
    };
    const startNativeDrag = () => {
        if (dragging.current) return;
        cancelDragHold();
        cancelIslandHover(hoverTimer);
        gesture.current = undefined;
        suppressClick.current = true;
        dragging.current = true;
        setDragActive(true);
        let releasedInside = false; // 失败、取消或无有效结果时保守收起，不读取 DOM hover 缓存。
        void finishMotion
            .current()
            .then(() => submission.current)
            .then(() => beginIslandDrag())
            .then((result) => {
                releasedInside = result.pointerInside;
                if (result.offset !== null) pendingOffset.current = result.offset; // 先采用原生落点，偏好事件随后到达。
            })
            .catch((error) => console.warn("岛拖动失败", error))
            .finally(() => {
                // 松手后由原生后台保存落点并广播 preferences-changed；页面只按事件重测，避免用旧 offset 先跳到旧位置。
                // 仅使用原生命令返回的释放事实，不等待可能缺失的 pointerleave 或 :hover 更新。
                cancelIslandHover(hoverTimer);
                dragging.current = false;
                setDragActive(false);
                if (!mounted.current) return;
                pointerInside.current = releasedInside;
                geometryKey.current = "";
                skipMotionOnce.current = true;
                collapsedBox.current = undefined; // 拖动已改变沿边位置，旧胶囊范围不再参与 hover 并集。
                setMode(islandAfterDrag(releasedInside));
                window.setTimeout(() => {
                    if (!mounted.current) return;
                    geometryKey.current = "";
                    setDragRevision((value) => value + 1);
                }, 120); // 偏好事件通常先到；超时兜底重测一次。
                // 原生移动循环结束后窗口已经换位，WebView2 的悬停命中仍停在旧位置；补一次指针进入让页面恢复交互。
                if (releasedInside && root.current) {
                    root.current.dispatchEvent(new PointerEvent("pointerover", { bubbles: true, pointerId: 1, pointerType: "mouse" }));
                }
            });
    };
    const handlePointerDown = (event: PointerEvent<HTMLElement>) => {
        const target = event.target as Element;
        const intent = islandDragIntent(mode, event.button, Boolean(target.closest(ISLAND_NO_DRAG)), Boolean(target.closest(".island-card")));
        sortDragAllowed.current = intent === "sort";
        if (intent !== "window" || dragging.current) return;
        cancelIslandHover(hoverTimer);
        cancelDragHold();
        suppressClick.current = false;
        gesture.current = { id: event.pointerId, x: event.screenX, y: event.screenY };
        event.currentTarget.setPointerCapture(event.pointerId);
        // 按住满延时才进入原生移动循环；期间释放或移动都按点击/普通交互处理。
        dragHold.current = setTimeout(() => {
            dragHold.current = undefined;
            if (gesture.current?.id === event.pointerId && !dragging.current) startNativeDrag();
        }, ISLAND_DRAG_DELAY_MS);
    };
    const handlePointerMove = (event: PointerEvent<HTMLElement>) => {
        const candidate = gesture.current;
        if (!candidate || candidate.id !== event.pointerId || !islandDragMoved(candidate, { x: event.screenX, y: event.screenY })) return;
        cancelDragHold(); // 先移动取消长按拖动，不再按位移升级。
    };
    const handlePointerUp = (event: PointerEvent<HTMLElement>) => {
        const candidate = gesture.current;
        if (!candidate || candidate.id !== event.pointerId) return;
        cancelDragHold();
        gesture.current = undefined;
        if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
        const hit = document.elementFromPoint(event.clientX, event.clientY);
        pointerInside.current = Boolean(hit && root.current?.contains(hit));
        if (
            !suppressClick.current &&
            !islandDragMoved(candidate, { x: event.screenX, y: event.screenY }) &&
            hit &&
            root.current?.contains(hit) &&
            !hit.closest(ISLAND_NO_DRAG)
        ) {
            cancelIslandHover(hoverTimer);
            setMode("expanded");
        } else scheduleHover(pointerInside.current);
    };
    const handleReorder = async (from: string, to: string) => {
        if (sorting.current) return;
        const order = instances.map((instance) => instance.componentId);
        const next = moveIslandCard(order, from, to);
        if (next.join("\n") === order.join("\n")) return;
        sorting.current = true;
        try {
            onPreferences(await saveIslandCardOrder(next));
        } catch (error) {
            console.warn("岛卡片顺序保存失败", error);
        } finally {
            sorting.current = false;
        }
    };
    return (
        <main
            ref={root}
            className="island"
            style={{ maxBlockSize: work?.height }}
            data-island-page="true"
            data-dragging={dragActive ? "true" : undefined}
            data-form={preferences?.islandForm ?? "capsule"}
            data-edge={edge || "top"}
            data-state={mode}
            data-vertical={vertical}
            data-fillets={preferences?.islandForm === "notch" && mode !== "expanded" ? "true" : undefined}
            data-arcs={preferences?.islandForm === "notch" && mode !== "expanded" && !motionActive ? "true" : undefined}
            onPointerEnter={handleEnter}
            onPointerLeave={handleLeave}
            onPointerDownCapture={handlePointerDown}
            onPointerMove={handlePointerMove}
            onPointerUpCapture={handlePointerUp}
            onPointerCancel={(event) => {
                gesture.current = undefined;
                if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
                scheduleHover(pointerInside.current);
            }}
            tabIndex={0}
            aria-label="灵动岛，回车展开，退出键收起"
            onKeyDown={(event) => {
                if (dragging.current || gesture.current || cardDrag.current) return;
                cancelIslandHover(hoverTimer);
                if (event.key === "Escape") setMode("collapsed");
                else if (event.key === "Enter" && event.target === event.currentTarget) setMode("expanded");
            }}
        >
            <div ref={palette} className="fence-workspace island__palette" hidden aria-hidden="true" />
            {visible.length ? (
                visible.map((instance) => {
                    const manifest = snapshot!.runtime.manifests.find((item) => item.id === instance.componentId)!;
                    const registration = snapshot!.runtime.instances.find((item) => item.instanceId === instance.instanceId);
                    return (
                        <section
                            className="island-card"
                            key={instance.instanceId}
                            aria-label={manifest.name}
                            draggable={mode === "expanded"}
                            tabIndex={mode === "expanded" ? 0 : undefined}
                            aria-description={mode === "expanded" ? "拖动卡片调整顺序，或聚焦卡片后使用上下方向键" : undefined}
                            onDragStart={(event) => {
                                if (
                                    !sortDragAllowed.current ||
                                    islandDragIntent(mode, 0, Boolean((event.target as Element).closest(ISLAND_NO_DRAG)), true) !== "sort"
                                ) {
                                    event.preventDefault();
                                    return;
                                }
                                cardDrag.current = instance.componentId;
                                event.dataTransfer.setData("text/plain", instance.componentId);
                                event.dataTransfer.effectAllowed = "move";
                            }}
                            onDragEnd={() => {
                                sortDragAllowed.current = false;
                                cardDrag.current = undefined;
                                if (root.current && !root.current.matches(":hover")) setMode("collapsed");
                            }}
                            onDragOver={(event) => {
                                if (
                                    islandSortTarget(
                                        mode,
                                        instances.map((item) => item.componentId),
                                        cardDrag.current,
                                        instance.componentId,
                                        Boolean((event.target as Element).closest(ISLAND_NO_DRAG)),
                                    )
                                )
                                    event.preventDefault();
                            }}
                            onDrop={(event) => {
                                const from = cardDrag.current;
                                const to = islandSortTarget(
                                    mode,
                                    instances.map((item) => item.componentId),
                                    from,
                                    instance.componentId,
                                    Boolean((event.target as Element).closest(ISLAND_NO_DRAG)),
                                );
                                if (!from || !to) return;
                                event.preventDefault();
                                cardDrag.current = undefined;
                                void handleReorder(from, to);
                            }}
                            onKeyDown={(event) => {
                                if (mode !== "expanded" || event.target !== event.currentTarget) return;
                                const index = instances.indexOf(instance);
                                const target = instances[index + (event.key === "ArrowUp" ? -1 : event.key === "ArrowDown" ? 1 : 0)];
                                const to =
                                    target &&
                                    islandSortTarget(
                                        mode,
                                        instances.map((item) => item.componentId),
                                        instance.componentId,
                                        target.componentId,
                                        false,
                                    );
                                if (to) {
                                    event.preventDefault();
                                    void handleReorder(instance.componentId, to);
                                }
                            }}
                        >
                            <div className="island-card__content">
                                {instance.componentId === "official.music-control" ? (
                                    <IslandMusic mode={mode} side={edge === "left" || edge === "right"} />
                                ) : registration?.status === "failed" ? (
                                    <p role="status">组件加载失败</p>
                                ) : registration?.entry === "plugin:slot" ? (
                                    <div data-island-plugin="true" inert={mode === "collapsed" ? true : undefined}>
                                        <PluginSlot manifest={manifest} instanceId={instance.instanceId} tileType={pluginTileType(instance.state)} collapsed={mode === "collapsed"} />
                                    </div>
                                ) : (
                                    <p role="status">{manifest.name} · 组件入口不可用</p>
                                )}
                            </div>
                        </section>
                    );
                })
            ) : (
                <section className="island-card">
                    <div className="island-card__content">
                        <p role="status">{failure ? "岛加载失败" : snapshot ? "暂无岛卡" : "正在加载"}</p>
                    </div>
                </section>
            )}
        </main>
    );
};

// ponytail: 只消费既有采集；后端支持消费者引用计数单例后，再让岛按需参与采集生命周期。
/** 后端已有频谱时绘制真实电平；无采集、暂停与失败时保持固定高度基线。 */
const IslandSpectrum = ({ playing }: { playing: boolean }) => {
    const [bins, setBins] = useState<number[]>([]);
    useEffect(() => {
        if (!playing) {
            setBins([]);
            return;
        }
        let active = true;
        const subscription = listenSpectrum(
            (values) => {
                if (active) setBins(Array.from(values));
            },
            () => {
                if (active) setBins([]);
            },
        );
        void subscription.catch(() => {
            if (active) setBins([]);
        });
        return () => {
            active = false;
            void subscription.then((stop) => stop()).catch(() => undefined);
        };
    }, [playing]);
    return (
        <div className="island-music__spectrum" aria-hidden="true">
            {Array.from({ length: 24 }, (_, index) => (
                <i key={index} style={{ blockSize: `${Math.max(1, ((bins[Math.floor((index * bins.length) / 24)] ?? 0) / 255) * 18)}px` }} />
            ))}
        </div>
    );
};
