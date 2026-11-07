import { useEffect, useLayoutEffect, useRef, useState, type PointerEvent } from "react";
import { getDesktopHostSnapshot } from "../../bridge/desktopHost";
import { beginIslandDrag, listenIslandSnapshots, saveIslandCardOrder, setIslandState } from "../../bridge/island";
import { executeMusicAction } from "../../bridge/music";
import { listenSpectrum } from "../../bridge/audio";
import { getPreferences } from "../../bridge/settings";
import type { IslandRect, IslandSnapshot, IslandStateRequest } from "../../contracts/island";
import { PluginSlot, pluginTileType } from "../components/PluginSlot";
import { useMusicState, positionAt, lyricIndex } from "../music/musicSource";
import { usePreferences } from "../settings/PreferencesProvider";
import { islandGeometry, moveIslandCard, visibleIslandRect } from "./islandGeometry";
import {
    ISLAND_HOVER_DELAY_MS,
    ISLAND_NO_DRAG,
    islandDragIntent,
    islandSortTarget,
    islandDragMoved,
    islandAfterDrag,
    cancelIslandHover,
    islandHoverTarget,
} from "./islandInteraction";
import { ISLAND_MOTION_MS, ISLAND_FRAME_MS, islandMotionFrame } from "./islandMotion";
import "./island.css";

type IslandMode = IslandStateRequest["state"]; // 三态互斥。

/** 岛只消费共享音乐事实，计时器只推进可见投影，不读取媒体或请求歌词。 */
const IslandMusic = ({ mode }: { mode: IslandMode }) => {
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
            const next = music.lyrics[lyricIndex(music.lyrics, projected) + 1]?.startMs;
            const delay = mode === "expanded" ? 250 : next === undefined ? undefined : Math.max(16, next - projected);
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
                    <>
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
                        {mode === "hover" ? controls : null}
                    </>
                )}
            </div>
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
    const dragging = useRef(false); // 原生移动循环期间不切 hover。
    const cardDrag = useRef<string | undefined>(undefined); // 卡片排序身份，不保存布局。
    const sortDragAllowed = useRef(false); // 保留按下的真实来源，阻止控件借父卡片发起 HTML 拖动。
    const geometryKey = useRef(""); // 同一实测结果不重复 IPC。
    const currentGeometry = useRef<IslandStateRequest | undefined>(undefined); // 最近实际完成的原生区域。
    const finishMotion = useRef<() => Promise<void>>(async () => undefined); // 拖动前先结束形变，防止动画帧移动原生拖动中的窗口。
    const gesture = useRef<{ id: number; x: number; y: number } | undefined>(undefined); // hover 单击候选，按位移升级为拖动。
    const pointerInside = useRef(false); // 延时回调只读取最新边界结果。
    const suppressClick = useRef(false); // 原生拖动释放不得再次当成单击展开。
    const [dragRevision, setDragRevision] = useState(0); // 拖动结束即使仍为同态也重测新位置。
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
    const vertical = mode === "collapsed" && (edge === "left" || edge === "right");
    const instances = snapshot?.instances ?? [];
    const visible = mode === "expanded" ? instances : instances.slice(0, 1);
    useLayoutEffect(() => {
        if (!preferences || !work || !root.current) return;
        let active = true;
        let frame = 0;
        let motionFrame = 0;
        let wakeMotion: (() => void) | undefined;
        let animating = false;
        let forceFinish = false;
        let finish: Promise<void> = Promise.resolve();
        const savedStyles = new Map<HTMLElement, string | null>(); // 完成、取消与失败都还原最终态的正常文档流。
        const saveStyle = (element: HTMLElement) => {
            if (!savedStyles.has(element)) savedStyles.set(element, element.getAttribute("style"));
        };
        const restoreStyles = () => {
            for (const [element, value] of savedStyles) {
                if (value === null) element.removeAttribute("style");
                else element.setAttribute("style", value);
            }
            savedStyles.clear();
        };
        const measureNow = () => {
            if (animating || dragging.current) return;
            cancelAnimationFrame(frame);
            {
                if (!active || !root.current || dragging.current) return;
                const origin = root.current.getBoundingClientRect();
                const measured = Array.from(root.current.querySelectorAll<HTMLElement>(".island-card")).flatMap((card) => {
                    const rect = card.getBoundingClientRect();
                    return visibleIslandRect(
                        { left: rect.left - origin.left, top: rect.top - origin.top, width: rect.width, height: rect.height },
                        origin.height,
                    );
                });
                if (!measured.length) return;
                try {
                    const radius = mode === "collapsed" ? 22 : mode === "hover" ? 16 : preferences.islandRadius;
                    const request = islandGeometry(mode, measured, radius, work, edge, preferences.islandOffset);
                    const key = JSON.stringify(request);
                    if (key === geometryKey.current) return;
                    geometryKey.current = key;
                    const element = root.current;
                    const cards = Array.from(element.querySelectorAll<HTMLElement>(".island-card"));
                    const from = currentGeometry.current;
                    const animate = animationsEnabled && from && from.state !== request.state && !dragging.current;
                    const paint = (value: IslandStateRequest, elapsed: number) => {
                        saveStyle(element);
                        Object.assign(element.style, {
                            position: "relative",
                            inlineSize: `${value.windowBox.width}px`,
                            blockSize: `${value.windowBox.height}px`,
                            maxInlineSize: "none",
                            overflow: "clip",
                        });
                        const reveal = Math.min(1, elapsed / ISLAND_MOTION_MS);
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
                                borderRadius: `${shape.radius}px`,
                            });
                            const content = card.querySelector<HTMLElement>(".island-card__content");
                            if (content) {
                                saveStyle(content);
                                content.style.opacity = String(reveal);
                                content.style.filter = `blur(${5 * (1 - reveal)}px)`;
                            }
                        });
                    };
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
                                    paint(islandMotionFrame(startGeometry, request, 0, edge, work), 0);
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
                                        const value = islandMotionFrame(startGeometry, request, elapsed, edge, work);
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
                                restoreStyles();
                                animating = false;
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
        const measure = () => {
            cancelAnimationFrame(frame);
            frame = requestAnimationFrame(measureNow);
        };
        const observer = new ResizeObserver(measure);
        observer.observe(root.current);
        root.current.addEventListener("scroll", measure);
        for (const card of root.current.querySelectorAll(".island-card")) observer.observe(card);
        measureNow(); // 首帧绘制前完成目标实测与内容冻结，不先闪出最终布局。
        const element = root.current;
        return () => {
            active = false;
            element.removeEventListener("scroll", measure);
            observer.disconnect();
            cancelAnimationFrame(frame);
            cancelAnimationFrame(motionFrame);
            wakeMotion?.();
            restoreStyles();
        };
    }, [mode, snapshot, preferences, work, edge, animationsEnabled, dragRevision]);
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
        if (dragging.current || gesture.current || cardDrag.current) return;
        if (mode === "expanded") setMode("collapsed");
        else scheduleHover(false);
    };
    const handleEnter = () => {
        pointerInside.current = true;
        scheduleHover(true);
    };
    const startNativeDrag = () => {
        if (dragging.current) return;
        cancelIslandHover(hoverTimer);
        gesture.current = undefined;
        suppressClick.current = true;
        dragging.current = true;
        let releasedInside = false; // 失败、取消或无有效结果时保守收起，不读取 DOM hover 缓存。
        void finishMotion
            .current()
            .then(() => submission.current)
            .then(() => beginIslandDrag())
            .then((result) => {
                releasedInside = result.pointerInside;
            })
            .catch((error) => console.warn("岛拖动失败", error))
            .finally(async () => {
                // 原生返回前保存落点；重读一次权威偏好，不能按开始时的 offset 把岛拉回。
                try {
                    const next = await getPreferences();
                    if (mounted.current) onPreferences(next);
                } catch (error) {
                    console.warn("岛拖动后偏好读取失败", error);
                }
                // 仅使用原生命令返回的释放事实，不等待可能缺失的 pointerleave 或 :hover 更新。
                cancelIslandHover(hoverTimer);
                dragging.current = false;
                if (!mounted.current) return;
                pointerInside.current = releasedInside;
                geometryKey.current = "";
                setMode(islandAfterDrag(releasedInside));
                setDragRevision((value) => value + 1);
            });
    };
    const handlePointerDown = (event: PointerEvent<HTMLElement>) => {
        const target = event.target as Element;
        const intent = islandDragIntent(mode, event.button, Boolean(target.closest(ISLAND_NO_DRAG)), Boolean(target.closest(".island-card")));
        sortDragAllowed.current = intent === "sort";
        if (intent !== "window" || dragging.current) return;
        cancelIslandHover(hoverTimer);
        suppressClick.current = false;
        if (mode === "hover") {
            gesture.current = { id: event.pointerId, x: event.screenX, y: event.screenY };
            event.currentTarget.setPointerCapture(event.pointerId);
        } else startNativeDrag();
    };
    const handlePointerMove = (event: PointerEvent<HTMLElement>) => {
        const candidate = gesture.current;
        if (!candidate || candidate.id !== event.pointerId || !islandDragMoved(candidate, { x: event.screenX, y: event.screenY })) return;
        event.currentTarget.releasePointerCapture(event.pointerId);
        startNativeDrag();
    };
    const handlePointerUp = (event: PointerEvent<HTMLElement>) => {
        const candidate = gesture.current;
        if (!candidate || candidate.id !== event.pointerId) return;
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
            data-state={mode}
            data-vertical={vertical}
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
                                    <IslandMusic mode={mode} />
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
