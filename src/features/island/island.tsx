import { useEffect, useLayoutEffect, useMemo, useRef, useState, type CSSProperties, type PointerEvent, type ReactNode } from "react";
import { getCollectItems, listenCollectChanges, type CollectItem } from "../../bridge/collect";
import { getDesktopHostSnapshot } from "../../bridge/desktopHost";
import { beginIslandDrag, dismissIslandMessage, getIslandMessages, islandCursorInside, listenIslandDropActive, listenIslandMessages, listenIslandSnapshots, saveIslandCardCollapsed, saveIslandCardOrder, setIslandState } from "../../bridge/island";
import { executeMusicAction } from "../../bridge/music";
import { listenSpectrum } from "../../bridge/audio";
import type { IslandDropFact as IslandNativeDropFact, IslandMessage, IslandRect, IslandSnapshot, IslandStateRequest } from "../../contracts/island";
import pauseIcon from "../../../resources/music-pause.svg?raw";
import closeIcon from "../../../resources/editor-x.svg?raw";
import successIcon from "../../../resources/editor-check.svg?raw";
import infoIcon from "../../../resources/info.svg?raw";
import errorIcon from "../../../resources/message-error.svg?raw";
import warningIcon from "../../../resources/message-warning.svg?raw";
import chevronIcon from "../../../resources/weather-chevron-right.svg?raw";
import musicFunctionIcon from "../../../resources/music-play.svg?raw";
import collectFunctionIcon from "../../../resources/menu-collect.svg?raw";
import weatherFunctionIcon from "../../../resources/menu-weather.svg?raw";
import pluginFunctionIcon from "../../../resources/settings-plugin.svg?raw";
import { PluginSlot, pluginTileType } from "../components/PluginSlot";
import { useMusicState, musicProgressRange, positionAt } from "../music/musicSource";
import { usePreferences } from "../../app/preferences/PreferencesProvider";
import { WeatherGlyph } from "../../shared/WeatherGlyph";
import { ScrollingText } from "../../shared/ScrollingText";
import { IslandCollectBox } from "./IslandCollectBox";
import { IslandWeather, useIslandWeather } from "./IslandWeather";
import {
    islandAttachedCorner,
    islandCardMaskPath,
    islandCornerRadius,
    islandFilletPatches,
    islandSeamMask,
    islandGeometry,
    moveIslandCard,
    visibleIslandRect,
} from "./islandGeometry";
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
    islandHitContour, islandCardDistance, islandContourDistance, islandHitRegion, islandMarginStep, islandScrollGeometry, islandPointInRect, islandDebugCardCount, islandCollapseSettle,
    ISLAND_MARGIN_DELAY_MS, ISLAND_SCROLLBAR_FADE_MS,
    type IslandMarginState,
} from "./islandInteraction";
import { ISLAND_MOTION_MS, ISLAND_FRAME_MS, ISLAND_EXPAND_STAGGER_MS, ISLAND_EXPAND_MOTION_MS, islandAttachedCornerAt, islandUnionFrame } from "./islandMotion";
import { islandHoldMessages, islandMessageTime, islandMessageView } from "./islandMessages";
import { ISLAND_MESSAGE_ANCHOR_TTL_MS, ISLAND_ANCHOR_GAP, ISLAND_ANCHOR_GROW_MS, ISLAND_ANCHOR_SHRINK_MS, ISLAND_ANCHOR_SIZE, ISLAND_ANCHOR_SLOTS, ISLAND_STATUS_SPECS, islandAnchorDeadline, islandAnchorGlyph, islandAnchorId, islandAnchorValue, islandArrive, islandCapsuleFace, islandClockLabel, islandDropFace, islandExpireAnchors, islandHoverContent, islandHoverEnter, islandHoverSelect, islandHoverStep, islandLayerRank, islandMergeHiddenAnchors, islandOverlayAction, islandStatusProgress, islandStatusSpec, islandAnchorKind, type IslandAnchor, type IslandAnchorArrival, type IslandDropTarget, type IslandFunctionKind, type IslandHiddenArrival, type IslandStatus, type IslandStatusSpec } from "./islandContent";
import { ISLAND_LYRIC_OFFSETS, islandCapsuleLineLimit, islandLyricView, islandMusicTickDelay, islandPlaybackStatus, islandScrollAnimationName, islandScrollDistance, islandScrollKey, islandScrollPlan, islandScrollPlayState } from "./islandLyrics";
import { islandCollectTitle, readIslandCollectCache, writeIslandCollectCache } from "./islandCollect";
import { ISLAND_FENCE_TOKENS } from "./islandTokens";
import islandToggleCollapseIcon from "../../../resources/fence-toggle-collapse.svg?raw";
import islandToggleExpandIcon from "../../../resources/fence-toggle-expand.svg?raw";
import "../../styles/fenceTokens.css"; // 探针读取的围栏内置值，来自桌面工作区与浮岛共用的那一份。
import "./island.css";
const PAINTED_STYLE_PROPERTIES = [
    "position",
    "left",
    "top",
    "inline-size",
    "block-size",
    "max-inline-size",
    "min-block-size",
    "overflow",
    "border-radius",
    "flex",
    "opacity",
    "filter",
    "z-index",
] as const; // paint 写入的内联属性，恢复时只清这些。

type IslandMode = IslandStateRequest["state"]; // 三态互斥。

/** 岛只消费共享音乐事实，计时器只推进可见投影，不读取媒体或请求歌词。 */
const IslandMusic = ({ mode, side, motion, music, position }: { mode: "hover" | "expanded"; side: boolean; motion: boolean; music: ReturnType<typeof useMusicState>; position: number }) => {
    const [failure, setFailure] = useState(false);
    const [failedArtwork, setFailedArtwork] = useState<string>();
    const media = music.snapshot;
    const progress = musicProgressRange(music.timeline, music.lyrics, position);
    const view = islandLyricView(music.lyrics, position, media);
    const status = islandPlaybackStatus(media);
    // 展开歌词：换句时整轨一次位移，行与行不拆开；当前句的字号与颜色延后 200ms 过渡，其余行立即回落。
    const lyricsTrackRef = useRef<HTMLDivElement>(null);
    const previousLyricIndex = useRef<number | undefined>(undefined);
    useLayoutEffect(() => {
        const track = lyricsTrackRef.current;
        if (!track) {
            previousLyricIndex.current = undefined; // 收起后重开不补动画。
            return;
        }
        const previous = previousLyricIndex.current;
        previousLyricIndex.current = view.index;
        if (previous === undefined || previous === view.index || !motion) return;
        const direction = view.index > previous ? 1 : -1; // 前进向上滚，回退向下滚。
        const rows = Array.from(track.querySelectorAll<HTMLElement>("[data-line-index]"));
        const step = rows.length > 1 ? Math.abs(rows[1].offsetTop - rows[0].offsetTop) : 20; // 一行歌词的实际步进。
        // 已播放行先位移、当前与未播放行延迟 80ms；fill=backwards 让延迟段停在起始位移，不回到原位。
        const animations = rows.flatMap((row) => {
            const played = Number(row.dataset.lineIndex) < view.index;
            return [
                row.animate([{ transform: `translateY(${direction * step}px)` }, { transform: "none" }], {
                    duration: played ? 220 : 240,
                    delay: played ? 0 : 80,
                    fill: "backwards",
                    easing: "ease-out",
                }),
            ];
        });
        return () => animations.forEach((animation) => animation.cancel());
    }, [mode, view.index, motion]);
    const controls = <IslandMusicControls media={media} onFailure={() => setFailure(true)} />;
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
                    <div className="island-music__info">
                        <div className="island-music__metadata">
                            <p className="island-music__title">
                                <ScrollingText text={media?.title || status} enabled={motion} />
                            </p>
                            <p className="island-music__artist">
                                <ScrollingText text={media?.artist || "\u00a0"} enabled={motion} />
                            </p>
                            <p className="island-music__source">
                                <ScrollingText text={failure ? "控制失败，请重试" : media?.source || "\u00a0"} enabled={motion} />
                            </p>
                        </div>
                        {mode === "expanded" ? controls : null}
                    </div>
                {mode === "hover" && !side ? controls : null}
            </div>
            {mode === "hover" && side ? <div className="island-music__second-row">{controls}</div> : null}
            {mode === "expanded" ? (
                <>
                    <div className="island-music__timeline">
                        <progress aria-label="播放进度" max={progress.max} value={progress.value} />
                        <div>
                            <time>{formatTime(position)}</time>
                            <time>{progress.totalMs > 0 ? formatTime(progress.totalMs) : "--:--"}</time>
                        </div>
                    </div>
                    <IslandSpectrum playing={media?.playback === "playing"} />
                    <div className="island-music__lyrics" aria-live="off">
                        <div ref={lyricsTrackRef} className="island-music__lyrics-track">
                            {ISLAND_LYRIC_OFFSETS.map((offset, slot) => {
                                const lineIndex = view.index + offset;
                                return (
                                    <p key={lineIndex >= 0 ? `line:${lineIndex}` : `line:${lineIndex}:${slot}`} data-line-index={lineIndex} data-current={offset === 0}>
                                        {offset === 0 ? <ScrollingText text={view.slots[slot]} enabled={motion} /> : view.slots[slot]}
                                    </p>
                                );
                            })}
                        </div>
                    </div>
                </>
            ) : null}
        </div>
    );
};

/** 时间格式固定一行，不为无会话另造布局。 */
const formatTime = (milliseconds: number) => `${Math.floor(milliseconds / 60000)}:${String(Math.floor(milliseconds / 1000) % 60).padStart(2, "0")}`;

/** 消息类型与图标资源一一对应，颜色仍由正文色与主题令牌控制。 */
const ISLAND_MESSAGE_ICONS: Record<IslandMessage["type"], string> = { info: infoIcon, success: successIcon, warning: warningIcon, error: errorIcon };

/** 展开消息承载完整正文、来源、时刻与原生关闭操作。 */
const IslandMessageCard = ({ message, onDismiss }: { message: IslandMessage; onDismiss: (id: number) => void }) => (
    <div className="island-message" data-kind={message.type} data-loading={message.loading ? "true" : undefined}>
        <span className="island-message__icon" aria-hidden="true" dangerouslySetInnerHTML={{ __html: ISLAND_MESSAGE_ICONS[message.type] }} />
        <div className="island-message__info">
            <p className="island-message__body">{message.content}</p>
            <p className="island-message__meta">{message.source} · {islandMessageTime(message.receivedAtMs, Date.now())}</p>
        </div>
        <button type="button" data-button="icon" className="island-message__close" aria-label={`关闭${message.source}消息`} onClick={() => onDismiss(message.id)}>
            <span aria-hidden="true" dangerouslySetInnerHTML={{ __html: closeIcon }} />
        </button>
    </div>
);

/** 展开态消息面板：默认只显示最新一条，按键切换为最近消息列表。 */
const IslandMessagePanel = ({ messages, onDismiss }: { messages: IslandMessage[]; onDismiss: (id: number) => void }) => {
    const [open, setOpen] = useState(false); // 每次进入展开态默认折叠。
    const visible = open ? messages : messages.slice(0, 1); // 折叠时只保留最新一条。
    return (
        <div className="island-message-panel">
            <div className="island-message-panel__head">
                <span>消息</span>
                {messages.length > 1 ? (
                    <button
                        type="button"
                        className="island-message-panel__toggle"
                        data-island-no-drag="true"
                        aria-expanded={open}
                        aria-label={open ? "折叠消息" : `展开全部 ${messages.length} 条消息`}
                        data-open={open ? "true" : "false"}
                        onClick={() => setOpen((value) => !value)}
                    >
                        <span aria-hidden="true" dangerouslySetInnerHTML={{ __html: chevronIcon }} />
                    </button>
                ) : null}
            </div>
            <div className="island-message-panel__list">
                {visible.map((message) => (
                    <IslandMessageCard key={message.id} message={message} onDismiss={onDismiss} />
                ))}
            </div>
        </div>
    );
};

/** 歌词在受限的一行内按绝对播放位置滚动，不推动卡片与相邻锚点。 */
const IslandCapsuleLyric = ({ music, position, motion, vertical }: { music: ReturnType<typeof useMusicState>; position: number; motion: boolean; vertical: boolean }) => {
    const [overflow, setOverflow] = useState(0);
    const lineRef = useRef<HTMLParagraphElement>(null);
    const view = islandLyricView(music.lyrics, position, music.snapshot);
    const currentLine = music.lyrics[view.index];
    // 常态胶囊滚动计划：有词按播放进度、无词回落循环；偏移由合成层动画推进，不写布局。
    const scroll = islandScrollPlan({
        distance: overflow,
        hasLine: view.hasLine,
        lineStartMs: currentLine?.startMs,
        lineEndMs: music.lyrics[view.index + 1]?.startMs ?? music.timeline?.durationMs,
        positionMs: position,
        advancing: music.timeline?.advancing === true,
        animationsEnabled: motion,
    });
    const overflowRef = useRef(0); // 与渲染值同步的测量缓存，同一尺寸不重复写状态。
    useLayoutEffect(() => {
        const box = lineRef.current;
        const content = box?.querySelector<HTMLElement>(".island-music__measure");
        const update = () => {
            if (!box || !content) {
                overflowRef.current = 0;
                setOverflow((value) => (value === 0 ? value : 0));
                return;
            }
            // 度量节点不做变换，也不随滚动键重建。
            const next = islandScrollDistance(vertical ? content.offsetHeight : content.offsetWidth, vertical ? box.clientHeight : box.clientWidth);
            if (overflowRef.current === next) return;
            overflowRef.current = next;
            setOverflow(next);
        };
        update();
        const observer = new ResizeObserver(update); // 换句、换档位或窗口宽度变化时重测。
        if (box) observer.observe(box);
        if (content) observer.observe(content);
        return () => observer.disconnect();
    }, [view.index, view.hasLine, vertical, motion]);
    // 换句/切歌/跳转/换边时重建动画并按新偏移对齐；无词循环的暂停不重建，保持循环相位。
    const scrollKey = islandScrollKey({ sessionId: music.timeline?.sessionId ?? "", index: view.index, vertical, plan: scroll });
    const animationName = islandScrollAnimationName(scroll.kind, vertical);
    const scrollStyle: CSSProperties | undefined =
        animationName === undefined
            ? undefined
            : {
                  animationName,
                  animationDuration: `${scroll.durationMs}ms`,
                  animationDelay: scroll.kind === "progress" ? `${-scroll.delayMs}ms` : "0ms",
                  animationTimingFunction: "linear",
                  animationIterationCount: scroll.kind === "loop" ? "infinite" : 1,
                  animationFillMode: "both",
                  animationPlayState: islandScrollPlayState(scroll.paused),
              };
    const lineStyle = { "--island-lyric-distance": `${scroll.distance}px` } as CSSProperties;
    return <p ref={lineRef} className="island-music__line" data-motion={motion ? "on" : "off"} style={lineStyle}>
        <span className="island-music__measure"><span key={scrollKey} className="island-music__current" style={scrollStyle}>{view.line}</span></span>
    </p>;
};

/** 同一个共享音乐时钟在三态中切换刷新节奏。 */
const useIslandMusic = (mode: IslandMode, enabled: boolean) => {
    const music = useMusicState();
    const [now, setNow] = useState(() => performance.now());
    useEffect(() => {
        let timer: ReturnType<typeof setTimeout> | undefined;
        const refresh = () => {
            clearTimeout(timer);
            if (!enabled || document.hidden || window.__DESKTOP_TOOLS_NATIVE__?.isRefreshPaused?.()) return;
            const current = performance.now();
            setNow(current);
            // 常态逐句对齐、展开推进进度；暂停、hover 与轨道末端不安排下一次刷新。
            const lyricDelay = islandMusicTickDelay(mode, music, music.lyrics, current);
            const duration = music.timeline?.durationMs ?? 0;
            const position = positionAt(music, current);
            const nextMilestone = duration * (Math.floor(position * 4 / duration) + 1) / 4;
            const progressDelay = mode === "collapsed" && music.timeline?.advancing && duration > 0 && nextMilestone < duration
                ? Math.max(1, Math.ceil(nextMilestone - position)) : Infinity;
            const delay = Math.min(lyricDelay ?? Infinity, progressDelay);
            if (Number.isFinite(delay)) timer = setTimeout(refresh, delay);
        };
        refresh();
        document.addEventListener("visibilitychange", refresh);
        window.addEventListener("desktop-refresh-changed", refresh);
        return () => {
            clearTimeout(timer);
            document.removeEventListener("visibilitychange", refresh);
            window.removeEventListener("desktop-refresh-changed", refresh);
        };
    }, [music, mode, enabled]);
    return { music, position: positionAt(music, now) };
};

/** 同一份安全控件描述向模型提供可执行动作，并驱动按钮渲染。 */
const islandMusicCommands = (media: ReturnType<typeof useMusicState>["snapshot"]) => {
    const playing = media?.playback === "playing";
    return [
        ["previous", "上一首", media?.canPrevious, "M10 4 3 8l7 4V4ZM2 4v8"],
        [playing ? "pause" : "play", playing ? "暂停" : "播放", media?.canPlayPause, playing ? "M5 3v10M11 3v10" : "m5 3 8 5-8 5V3Z"],
        ["next", "下一首", media?.canNext, "m6 4 7 4-7 4V4Zm8 0v8"],
    ] as const;
};

/** 展开保留禁用反馈；hover 只绘制模型准入后的动作。 */
const IslandMusicControls = ({ media, onFailure, allowed }: { media: ReturnType<typeof useMusicState>["snapshot"]; onFailure?: () => void; allowed?: readonly string[] }) => {
    const [busy, setBusy] = useState(false);
    const handleAction = async (action: "previous" | "play" | "pause" | "next") => {
        if (!media || busy) return;
        setBusy(true);
        try {
            await executeMusicAction(action, media.sessionId);
        } catch (error) {
            console.warn("岛音乐控制失败", error);
            onFailure?.();
        } finally {
            setBusy(false);
        }
    };
    return <div className="island-music__controls">
        {islandMusicCommands(media).filter(([action]) => !allowed || allowed.includes(action)).map(([action, label, enabled, path]) => (
            <button type="button" data-button="icon" key={label === "暂停" || label === "播放" ? "play-pause" : action} aria-label={label} disabled={!enabled || busy} onClick={() => void handleAction(action)}>
                <svg viewBox="0 0 16 16" aria-hidden="true"><path d={path} /></svg>
            </button>
        ))}
    </div>;
};

/** 限制字数而不切开组合字符；歌词使用独立受限的滚动槽。 */
const islandTextSegmenter = new Intl.Segmenter();
const islandShortText = (text: string) => {
    const chars = [...islandTextSegmenter.segment(text.trim().replace(/\s+/g, " "))].map(({ segment }) => segment);
    const limit = /\p{Script=Han}/u.test(text) ? 18 : 36;
    return chars.length > limit ? `${chars.slice(0, limit - 1).join("")}…` : chars.join("");
};

/** SVG 实线弧，不依赖虚线长度模拟进度。 */
const islandArcPath = (progress: number) => {
    const angle = progress * Math.PI * 2;
    return `M10 1.5 A8.5 8.5 0 ${progress > 0.5 ? 1 : 0} 1 ${10 + 8.5 * Math.sin(angle)} ${10 - 8.5 * Math.cos(angle)}`;
};

/** 仅是图形资源映射；短词、状态认定与去向使用纯模型。 */
const ISLAND_ANCHOR_ICONS = { pause: pauseIcon, check: successIcon, cross: closeIcon, alert: errorIcon };
const ISLAND_ANCHOR_PATHS = {
    question: "M6.5 7a3.5 3.5 0 1 1 5 3.15C10 11 10 11.5 10 13m0 3h.01",
    "link-off": "m2 2 16 16m-6-13 1.5-1.5a3.5 3.5 0 0 1 5 5L15 8m-7 4-1.5 1.5a3.5 3.5 0 0 1-5-5L3 7",
    lock: "M5 9h10a1 1 0 0 1 1 1v6a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1v-6a1 1 0 0 1 1-1Zm2 0V6a3 3 0 0 1 6 0v3m-3 4v1",
    release: "M10 2v10m-4-4 4 4 4-4M3 12v5h14v-5",
    clock: "M10 3a7 7 0 1 0 0 14 7 7 0 0 0 0-14Zm0 3.4V10l2.6 1.6",
};

/** hover 顶部锚点的功能图标：先认出锚点属于哪个功能，状态符号改到内容行左侧。 */
const ISLAND_FUNCTION_ICONS: Record<IslandFunctionKind, string> = {
    music: musicFunctionIcon,
    collect: collectFunctionIcon,
    weather: weatherFunctionIcon,
    message: infoIcon,
    plugin: pluginFunctionIcon,
    event: infoIcon,
};

/** 功能图标只作身份标识，不表达状态；状态仍由内容行左侧的状态符号表达。 */
const IslandFunctionIcon = ({ kind }: { kind: IslandFunctionKind }) => <span className="island-anchor__icon" aria-hidden="true" dangerouslySetInnerHTML={{ __html: ISLAND_FUNCTION_ICONS[kind] }} />;

/** 在 20×20 安全盒内绘制一个状态、计数或空态符号。 */
const IslandAnchorVisual = ({ anchor, special, motion, children, entering = false, specs }: { anchor?: IslandAnchor; special?: string; motion: boolean; children?: ReactNode; entering?: boolean; specs?: readonly IslandStatusSpec[] }) => {
    const ink = useRef<HTMLSpanElement>(null);
    const insertOnMount = useRef(entering);
    useLayoutEffect(() => {
        const animate = insertOnMount.current && motion;
        insertOnMount.current = false;
        if (!animate) return;
        const insertion = ink.current?.animate([{ transform: "translateY(6px)", opacity: 0 }, { transform: "none", opacity: 1 }], { duration: ISLAND_ANCHOR_GROW_MS, easing: "ease-out" });
        return () => insertion?.cancel();
    }, [motion, anchor?.unit?.status]);
    const status = anchor?.unit?.status;
    const builtin = status && Object.hasOwn(ISLAND_STATUS_SPECS, status) ? status as IslandStatus : undefined;
    const spec = builtin ? islandStatusSpec(builtin) : specs?.find((item) => item.status === status);
    const value = islandAnchorValue(anchor?.unit?.secondary ?? "");
    const progress = value?.kind === "percent" ? value.value / 100 : 0;
    const glyph = builtin ? islandAnchorGlyph(builtin, progress) :
        spec?.visual.kind === "ring" ? { kind: "ring" as const, arc: spec.visual.source === "empty" ? 0 : islandStatusProgress("running", progress) ?? 0 } : spec?.visual;
    const specialSymbol = special && special in ISLAND_ANCHOR_PATHS ? special as keyof typeof ISLAND_ANCHOR_PATHS : undefined;
    const symbol = glyph?.kind === "glyph" ? glyph.symbol : specialSymbol;
    const asset = symbol && symbol in ISLAND_ANCHOR_ICONS ? ISLAND_ANCHOR_ICONS[symbol as keyof typeof ISLAND_ANCHOR_ICONS] : undefined;
    const path = symbol && symbol in ISLAND_ANCHOR_PATHS ? ISLAND_ANCHOR_PATHS[symbol as keyof typeof ISLAND_ANCHOR_PATHS] : undefined;
    return <span ref={ink} className="island-anchor__ink" aria-hidden="true" data-status={status} data-symbol={symbol} data-glyph={glyph?.kind}>
        {children ?? (asset ? <span className="island-anchor__icon" dangerouslySetInnerHTML={{ __html: asset }} /> : path ? (
            <svg viewBox="0 0 20 20"><path d={path} strokeWidth="1.75" /></svg>
        ) : glyph?.kind === "ring" || glyph?.kind === "sweep" ? (
            <svg viewBox="0 0 20 20">
                <circle className="island-anchor__track" cx="10" cy="10" r="8.5" strokeWidth="3" />
                {glyph.kind === "sweep" ? <path className="island-anchor__sweep" data-motion={motion} d={islandArcPath(0.25)} strokeWidth="3" /> :
                    glyph.arc >= 1 ? <circle cx="10" cy="10" r="8.5" strokeWidth="3" /> : glyph.arc > 0 ? <path d={islandArcPath(glyph.arc)} strokeWidth="3" /> : null}
            </svg>
        ) : special || anchor?.count !== undefined ? <span className="island-anchor__number">{special ?? (anchor!.count! >= 99 ? "99+" : anchor!.count)}</span> : <span className="island-anchor__dot" />)}
    </span>;
};

/** 各来源共享锚点身份；原生消息集合仍决定原生 Toast 何时离场。 */
const islandMessageArrival = (message: IslandMessage): IslandAnchorArrival => ({
    source: message.source,
    layer: "message",
    nowMs: message.receivedAtMs,
    kind: "message",
    unit: { primary: message.content, status: message.loading ? "indeterminate" : message.type === "error" ? "failed" : message.type === "success" ? "completed" : "pending" },
});

/** 前台消息互斥；原生队列离场与锚点 30 秒保留是两条不同的呈现期限。 */
const useIslandContent = (residents: readonly IslandAnchorArrival[], messages: IslandMessage[], mode: IslandMode, specs: readonly IslandStatusSpec[]) => {
    const [anchors, setAnchors] = useState<IslandAnchor[]>([]);
    const [foregroundId, setForegroundId] = useState<string>();
    const [selected, setSelected] = useState<string>();
    const [clockNow, setClockNow] = useState(Date.now);
    const observed = useRef(new Map<number, IslandMessage>());
    const read = useRef(new Set<string>());
    const previousMode = useRef(mode);
    const hidden = useRef<IslandHiddenArrival[]>([]);
    const hiddenStarted = useRef<number | undefined>(undefined);
    const lastHiddenRange = useRef<{ from: number; until: number } | undefined>(undefined);
    const [visibility, setVisibility] = useState(0);
    useEffect(() => {
        const refresh = () => {
            const now = Date.now();
            if (document.hidden || window.__DESKTOP_TOOLS_NATIVE__?.isRefreshPaused?.()) hiddenStarted.current ??= now;
            else if (hiddenStarted.current !== undefined) {
                lastHiddenRange.current = { from: hiddenStarted.current, until: now };
                hiddenStarted.current = undefined;
            }
            setVisibility((value) => value + 1);
        };
        refresh();
        document.addEventListener("visibilitychange", refresh);
        window.addEventListener("desktop-refresh-changed", refresh);
        return () => {
            document.removeEventListener("visibilitychange", refresh);
            window.removeEventListener("desktop-refresh-changed", refresh);
        };
    }, []);
    useLayoutEffect(() => {
        const now = Date.now();
        const paused = document.hidden || window.__DESKTOP_TOOLS_NATIVE__?.isRefreshPaused?.();
        const keep = new Set(residents.map((item) => islandAnchorId(item.layer, item.source)));
        let next = anchors.filter((anchor) => anchor.layer === "message" || keep.has(anchor.id));
        for (const resident of residents) {
            const prior = next.find((anchor) => anchor.id === islandAnchorId(resident.layer, resident.source));
            if (prior?.unit?.primary !== resident.unit?.primary || prior?.unit?.status !== resident.unit?.status || prior?.unit?.secondary !== resident.unit?.secondary || !prior) next = islandArrive(next, resident);
        }
        let active = next.find((anchor) => anchor.id === foregroundId);
        for (const message of [...messages].sort((a, b) => a.id - b.id)) {
            const prior = observed.current.get(message.id);
            if (prior?.content === message.content && prior.loading === message.loading && prior.type === message.type) continue;
            observed.current.set(message.id, message);
            const arrival = islandMessageArrival(message);
            const window = lastHiddenRange.current;
            if (paused || window && arrival.nowMs >= window.from && arrival.nowMs <= window.until) {
                if (!prior && now - arrival.nowMs < ISLAND_MESSAGE_ANCHOR_TTL_MS) hidden.current.push(arrival);
                continue;
            }
            const action = islandOverlayAction(active, arrival);
            next = islandArrive(next, arrival);
            if (!active || action !== "append") active = next.find((anchor) => anchor.id === islandAnchorId(arrival.layer, arrival.source));
        }
        if (!paused && hidden.current.length) {
            next = islandMergeHiddenAnchors(next, hidden.current, now);
            hidden.current = [];
        }
        // 不保存已超时且不存在于当前载荷中的投影身份。
        const currentIds = new Set(messages.map((message) => message.id));
        for (const [id, message] of observed.current) {
            const isRetainedFailure = message.type === "error" && next.some((anchor) => anchor.layer === "message" && anchor.source === message.source && anchor.updatedAtMs === message.receivedAtMs);
            if (!currentIds.has(id) && !isRetainedFailure && now - message.receivedAtMs >= ISLAND_MESSAGE_ANCHOR_TTL_MS) observed.current.delete(id);
        }
        if (mode === "expanded") {
            for (const anchor of next) if (islandAnchorDeadline(anchor) !== undefined) read.current.add(anchor.id);
        }
        if (mode === "collapsed" && previousMode.current === "expanded") {
            next = next.filter((anchor) => !read.current.has(anchor.id));
            read.current.clear();
        }
        previousMode.current = mode;
        next = islandExpireAnchors(next, now, mode);
        const eligible = next.filter((anchor) => anchor.unit && islandStatusSpec(anchor.unit.status, specs)?.disposition !== "vanish" && (
            anchor.layer !== "message" || keep.has(anchor.id) || anchor.unit.status === "failed" || messages.some((message) => message.source === anchor.source && message.receivedAtMs === anchor.updatedAtMs)
        ));
        const front = eligible.find((anchor) => anchor.id === active?.id);
        let winner = front;
        for (const anchor of eligible) if (!winner || islandLayerRank(anchor.layer) < islandLayerRank(winner.layer)) winner = anchor;
        if (anchors.length !== next.length || next.some((anchor, index) => anchor !== anchors[index])) setAnchors(next);
        setForegroundId(winner?.id);
        if (mode === "hover") setSelected((key) => islandHoverSelect(next, key ?? winner?.id));
    }, [residents, messages, mode, clockNow, visibility, specs]);
    useEffect(() => {
        const now = Date.now();
        const empty = !anchors.some((anchor) => islandStatusSpec(anchor.unit?.status ?? "", specs)?.disposition !== "vanish");
        if (empty && islandClockLabel(now) !== islandClockLabel(clockNow)) setClockNow(now);
        const deadlines = mode === "collapsed" ? anchors.flatMap((anchor) => { const deadline = islandAnchorDeadline(anchor); return deadline === undefined ? [] : [deadline]; }) : [];
        const next = Math.min(...deadlines, ...(empty ? [now + 60_000 - now % 60_000] : []));
        if (!Number.isFinite(next)) return;
        const timer = setTimeout(() => setClockNow(Date.now()), Math.max(1, next - now));
        return () => clearTimeout(timer);
    }, [anchors, clockNow, mode, specs]);
    const active = anchors.find((anchor) => anchor.id === foregroundId);
    const getMessage = (anchor: IslandAnchor) => [...observed.current.values()].find((item) => item.source === anchor.source && item.receivedAtMs === anchor.updatedAtMs);
    return { anchors, active, selected, setSelected, clockNow, getMessage, forgetMessage: (id: number) => {
        const message = observed.current.get(id);
        if (message) {
            setAnchors((items) => items.filter((anchor) => anchor.id !== islandAnchorId("message", message.source) || anchor.updatedAtMs !== message.receivedAtMs));
            observed.current.delete(id);
        }
    } };
};

/** 同来源换文在一行内滚动；换源只伸缩该锚点文字，原生盒随实测收放。 */
const IslandRelayContent = ({ anchor, motion, vertical, className, growDelay, music = false, children }: {
    anchor?: IslandAnchor; motion: boolean; vertical: boolean; className: string; growDelay: number; music?: boolean; children?: ReactNode;
}) => {
    const slot = useRef<HTMLDivElement>(null);
    const incoming = useRef<HTMLDivElement>(null);
    const outgoing = useRef<HTMLDivElement>(null);
    const previous = useRef(growDelay ? undefined : anchor);
    const interrupted = useRef(false);
    const [shown, setShown] = useState({ anchor, children });
    const [leaving, setLeaving] = useState<ReactNode>();
    useLayoutEffect(() => {
        const old = previous.current;
        previous.current = anchor;
        const node = slot.current;
        const animations: Animation[] = [];
        let active = true;
        const animate = (element: Element | null, frames: Keyframe[], duration: number, delay = 0) => {
            if (!element) return undefined;
            const value = element.animate(frames, { easing: "ease-out", fill: "both", duration, delay });
            animations.push(value);
            return value;
        };
        const axis = vertical ? "blockSize" : "inlineSize";
        const size = () => `${vertical ? node?.offsetHeight : node?.offsetWidth}px`;
        const wasInterrupted = interrupted.current;
        interrupted.current = false;
        const statusChanged = old?.id === anchor?.id && old?.unit?.status !== anchor?.unit?.status;
        if (!motion || !node || wasInterrupted || statusChanged) {
            setShown({ anchor, children });
            setLeaving(undefined);
        } else if (old?.id !== anchor?.id && old?.unit) {
            const shrink = animate(node, [{ [axis]: size(), opacity: 1 }, { [axis]: "0px", opacity: 0 }], ISLAND_ANCHOR_SHRINK_MS);
            void shrink?.finished.then(() => { if (active) setShown({ anchor, children }); shrink.cancel(); }).catch(() => undefined);
        } else {
            setShown({ anchor, children });
            if (old?.id !== anchor?.id && anchor?.unit) {
                const grow = animate(node, [{ [axis]: "0px", opacity: 0 }, { [axis]: size(), opacity: 1 }], ISLAND_ANCHOR_GROW_MS, growDelay);
                void grow?.finished.then(() => grow.cancel()).catch(() => undefined);
            } else if (anchor?.layer === "message" && old?.unit?.primary !== anchor.unit?.primary) {
                setLeaving(shown.children);
                const roll = animate(incoming.current, [{ transform: "translateY(100%)" }, { transform: "none" }], ISLAND_ANCHOR_GROW_MS);
                const leave = animate(outgoing.current, [{ transform: "none" }, { transform: "translateY(-100%)" }], ISLAND_ANCHOR_GROW_MS);
                void roll?.finished.then(() => { if (active) setLeaving(undefined); roll.cancel(); leave?.cancel(); }).catch(() => undefined);
            }
        }
        return () => {
            active = false;
            interrupted.current = animations.some((item) => item.playState === "running");
            animations.forEach((item) => item.cancel());
        };
    }, [anchor?.id, anchor?.unit?.status, anchor?.layer === "message" ? anchor.unit?.primary : undefined, motion, vertical]);
    const current = motion && !anchor && shown.anchor ? shown.children : children;
    return <div ref={slot} className={className} data-music={music || undefined}>
        <div ref={incoming}>{current}</div>
        <div ref={outgoing} className="island-anchor-label__outgoing" aria-hidden="true">{leaving}</div>
    </div>;
};

/** 岛独立挂载，不装配工作区、格子外壳或设置入口。 */
type IslandDropFact = IslandDropTarget | "reject";
/** 原生投放事实映射到同一内容模型：collect 进收集箱，reject 走「无法接收」，离开即回落。 */
const islandNativeDropFact = (fact: IslandNativeDropFact): IslandDropFact | undefined => fact.active ? (fact.kind === "collect" ? "collect-box" : "reject") : undefined;
type IslandPageProps = { contentFacts?: readonly IslandAnchorArrival[]; dropFact?: IslandDropFact; statusSpecs?: readonly IslandStatusSpec[] };
const EMPTY_CONTENT_FACTS: readonly IslandAnchorArrival[] = [];

const EMPTY_STATUS_SPECS: readonly IslandStatusSpec[] = [];
export const IslandPage = ({ contentFacts = EMPTY_CONTENT_FACTS, dropFact, statusSpecs = EMPTY_STATUS_SPECS }: IslandPageProps = {}) => {
    const { preferences, onPreferences, animationsEnabled } = usePreferences();
    const [prefersReducedMotion, setPrefersReducedMotion] = useState(() => matchMedia("(prefers-reduced-motion: reduce)").matches);
    useEffect(() => {
        const query = matchMedia("(prefers-reduced-motion: reduce)");
        const change = () => setPrefersReducedMotion(query.matches);
        query.addEventListener("change", change);
        return () => query.removeEventListener("change", change);
    }, []);
    const contentMotion = animationsEnabled && !prefersReducedMotion;
    const [snapshot, setSnapshot] = useState<IslandSnapshot>();
    const [failure, setFailure] = useState<string>();
    const [messages, setMessages] = useState<IslandMessage[]>([]); // 与原生 Toast 同一队列的只读投影。
    const [messageFailure, setMessageFailure] = useState<string>(); // 消息通道失败只降级，不阻塞岛卡。
    const heldMessages = useRef<IslandMessage[]>([]); // 阅读期暂存，回到常态清空。
    const messagesRevision = useRef(0); // 广播代次：关闭失败的补偿读取只在本轮无新事件时应用。
    const [mode, setMode] = useState<IslandMode>("collapsed");
    const [collapsedCards, setCollapsedCards] = useState<Record<string, boolean>>({}); // 卡内收起态；默认值来自偏好与收集箱默认收起。
    const [nativeDrop, setNativeDrop] = useState<IslandDropFact>(); // 原生投放进入后显示「松开加入收集箱」。
    const dropActiveRef = useRef(false); // 退回判定与离开路径读取最新值，不依赖渲染。
    const modeRef = useRef(mode); // 延时回调读取最新状态，不能覆盖键盘或点击后的展开。
    modeRef.current = mode;
    const [work, setWork] = useState<IslandRect>();
    const [monitorId, setMonitorId] = useState<string>(); // 主屏身份供浮岛天气登记活动订阅。
    const root = useRef<HTMLElement>(null); // 只测量岛自身。
    const palette = useRef<HTMLDivElement>(null); // 只读取现有主题的围栏令牌，不呈现工作区。
    // 用布局副作用在首次绘制前完成复制：island.css 不再自带令牌回落，浮岛的令牌只有样式表与偏好两处来源。
    useLayoutEffect(() => {
        const readPalette = () => {
            if (!root.current || !palette.current) return;
            // 先移除上次复制值，防止撤下主题后探针继承旧色形成自回路。
            for (const name of ISLAND_FENCE_TOKENS) root.current.style.removeProperty(name);
            const tokens = getComputedStyle(palette.current);
            for (const name of ISLAND_FENCE_TOKENS) {
                const value = tokens.getPropertyValue(name).trim();
                if (value) root.current.style.setProperty(name, value);
            }
        };
        const scheme = matchMedia("(prefers-color-scheme: dark)"); // 系统变化可能只改变主题的媒体查询。
        const observer = new MutationObserver(readPalette); // 同时覆盖主题表替换与深浅模式属性更新。
        observer.observe(document.head, { childList: true, subtree: true, attributes: true, attributeFilter: ["href"] });
        // 根内联样式同样观察：桌面强调色与浮岛圆角由偏好写到 html.style，变化时 preferences 与 data-desktop-theme 都不动，
        // 只靠上面的两处观察会让复制来的 --fence-focus-border 停在旧强调色。只观察 html 自身，探针写的是岛根，不会自回路。
        observer.observe(document.documentElement, { attributes: true, attributeFilter: ["data-desktop-theme", "style"] });
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
    const remeasure = useRef<(force?: boolean) => void>(() => {}); // 消息增减只改内容高度时主动重测，展开态根尺寸固定不会被 RO 观察到。
    const hoverTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
    const dragHold = useRef<ReturnType<typeof setTimeout> | undefined>(undefined); // 按下到进入原生拖动之间的长按计时。
    const dragging = useRef(false); // 原生移动循环期间不切 hover。

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
    const surface = useRef<HTMLDivElement>(null); // 卡片与两块外凸圆弧合并后的单层背景。
    const attachedCorner = useRef<number | undefined>(undefined); // 最近实际绘制的贴边侧带符号圆角，连续切态从这里衔接。
    const submission = useRef(Promise.resolve()); // 原生提交串行，防止快速切态乱序。
    const sorting = useRef(false); // 不让相邻两次保存用同一旧顺序。
    const targetGeometry = useRef<IslandStateRequest | undefined>(undefined); // 展开自然目标，不等同于保留union。
    const inputReady = useRef(false); // 原生拖动后等新几何提交，避免用旧窗口坐标判定。
    const layoutSettled = useRef(false);
    const inputGeometryKey = useRef(""); // expanded只在最新目标提交后评估，滚动重测不闪断条。
    const marginState = useRef<IslandMarginState>({ latch: "open" });
    const lastMarginInput = useRef<Parameters<typeof islandMarginStep>[1] | undefined>(undefined);
    const hoverKind = useRef<"enter" | "leave" | "margin" | undefined>(undefined);
    const hoverVersion = useRef(0);
    const scrollbar = useRef<HTMLDivElement>(null), scrollbarThumb = useRef<HTMLDivElement>(null);
    const scrollbarFrame = useRef<ReturnType<typeof islandScrollGeometry> | undefined>(undefined);
    const scrollbarDrag = useRef<{ id: number; y: number; top: number; range: number; travel: number; capture: HTMLElement } | undefined>(undefined);
    const collapseWait = useRef<{ token: number; target: IslandMode; timer: ReturnType<typeof setTimeout>; node: HTMLElement; handler: (event: TransitionEvent) => void } | undefined>(undefined);
    const collapseToken = useRef(0);
    const animationFlag = useRef(animationsEnabled);
    animationFlag.current = animationsEnabled;
    const inputHandlers = useRef({ move: (_: globalThis.PointerEvent) => {}, leave: (_: globalThis.PointerEvent) => {}, up: (_?: globalThis.PointerEvent) => {} });
    const debugCount = import.meta.env.DEV ? islandDebugCardCount(true, window.location.search) : 0;
    const stopHoverTimer = () => { cancelIslandHover(hoverTimer); hoverKind.current = undefined; hoverVersion.current++; };
    const setScrollbarVisible = (visible: boolean, instant = false) => {
        const node = scrollbar.current;
        if (!node) return;
        const duration = !instant && animationFlag.current ? ISLAND_SCROLLBAR_FADE_MS : 0;
        if (node.dataset.visible === String(visible) && !instant) return;
        const opacity = Number.parseFloat(getComputedStyle(node).opacity); // 提交旧样式，使CSS过渡起点确定。
        node.style.transitionDuration = `${duration}ms`;
        node.dataset.visible = String(visible);
        node.dataset.phase = duration && opacity !== Number(visible) ? (visible ? "entering" : "leaving") : (visible ? "visible" : "hidden");
        node.setAttribute("aria-hidden", String(!visible)); node.tabIndex = visible ? 0 : -1;
    };
    /** 取消“渐出完成后再收起”的等待；反向请求与卸载后迟到的回调不得再提交状态。 */
    const cancelCollapseWait = () => {
        const wait = collapseWait.current;
        if (!wait) return;
        collapseWait.current = undefined;
        collapseToken.current++;
        clearTimeout(wait.timer);
        wait.node.removeEventListener("transitionend", wait.handler);
    };
    /** 只提交状态本身；滚动条独立渐出的等待由 changeMode 负责，避免形变与渐出重叠。 */
    const commitMode = (next: IslandMode, retainLatch: boolean) => {
        stopHoverTimer();
        marginState.current = { latch: retainLatch ? "blocked" : "open" };
        lastMarginInput.current = undefined;
        if (next !== modeRef.current) {
            targetGeometry.current = undefined; layoutSettled.current = false; inputReady.current = false;
            setScrollbarVisible(false, !animationFlag.current); // 关闭动效时立即隐藏；开启动效时渐出已在前置等待里完成。
        }
        modeRef.current = next; setMode(next);
    };
    /**
     * 收起前先让滚动条独立渐出到 opacity=0，完成后再提交状态并开始卡片形变；
     * 无可见条或关闭动效时零等待，等待可取消且迟到回调不会执行收起。
     */
    const changeMode = (next: IslandMode, retainLatch = false) => {
        const waiting = collapseWait.current;
        if (waiting) {
            if (waiting.target === next) return; // 同一收起请求在途：继续等待渐出，不提前开始形变。
            cancelCollapseWait();
            if (modeRef.current === "expanded" && next === "expanded") refreshScrollbar(); // 反向请求撤回渐出，按当前溢出恢复条的可见性。
        }
        if (next === modeRef.current) {
            commitMode(next, retainLatch);
            return;
        }
        const node = scrollbar.current;
        if (modeRef.current === "expanded" && next !== "expanded" && animationFlag.current && node?.dataset.visible === "true") {
            const token = ++collapseToken.current;
            if (import.meta.env.DEV) delete node.dataset.fadeFallback; // 上一轮的兜底标记不沿用到本次收起。
            /**
             * transitionend 与兜底计时共用的收口：token 失效时既不隐藏也不提交；
             * 隐藏是无条件前置（与关闭动效一致的瞬时隐藏），不假定计时器到时条已透明。
             */
            const finish = (fallback: boolean) => {
                const settled = islandCollapseSettle(token, collapseToken.current, {
                    hide: () => {
                        setScrollbarVisible(false, true); // 与关闭动效相同的瞬时隐藏。
                        void getComputedStyle(node).opacity; // 强制样式提交，确保提交状态前条已不可见。
                    },
                    commit: () => {
                        cancelCollapseWait();
                        commitMode(next, retainLatch);
                    },
                });
                // 兜底触发只在开发构建与断言页留痕，便于下次取证判断是否走了兜底；发布构建无此分支。
                if (settled && fallback && import.meta.env.DEV && scrollbar.current) scrollbar.current.dataset.fadeFallback = "true";
            };
            const handler = (event: TransitionEvent) => {
                if (event.target !== node || event.propertyName !== "opacity") return;
                if (Number.parseFloat(getComputedStyle(node).opacity) > 0.001) return; // 忽略渐入或中间帧的迟到通知，只认确实已经不可见。
                finish(false);
            };
            // 兜底只负责到点补一次收口；真正隐藏由 finish 里同一个 hide 完成，不假定时间足够。
            collapseWait.current = { token, target: next, timer: setTimeout(() => finish(true), ISLAND_SCROLLBAR_FADE_MS + 50), node, handler };
            node.addEventListener("transitionend", handler);
            setScrollbarVisible(false); // 独立渐出从这里开始。
            return;
        }
        commitMode(next, retainLatch);
    };
    const refreshScrollbar = () => {
        if (collapseWait.current) return; // 收起渐出期间不得被滚动或重测把条重新显示出来。
        const element = root.current, frame = currentGeometry.current;
        if (!element || !frame) { setScrollbarVisible(false); return; }
        const value = islandScrollGeometry(frame, targetGeometry.current, preferences?.islandEdge ?? "", { mode: modeRef.current, settled: layoutSettled.current, scrollHeight: element.scrollHeight, clientHeight: element.clientHeight, scrollTop: element.scrollTop });
        scrollbarFrame.current = value;
        if (value.visible && value.track && value.thumb && scrollbar.current && scrollbarThumb.current) {
            const { track, thumb } = value;
            Object.assign(scrollbar.current.style, { insetInlineStart: `${track.left}px`, insetBlockStart: `${track.top}px`, inlineSize: `${track.width}px`, blockSize: `${track.height}px` });
            Object.assign(scrollbarThumb.current.style, { insetInlineStart: `${thumb.left - track.left}px`, insetBlockStart: `${thumb.top - track.top}px`, inlineSize: `${thumb.width}px`, blockSize: `${thumb.height}px` });
            scrollbar.current.setAttribute("aria-valuemin", "0"); scrollbar.current.setAttribute("aria-valuemax", String(value.maxScroll)); scrollbar.current.setAttribute("aria-valuenow", String(Math.round(element.scrollTop)));
        }
        setScrollbarVisible(value.visible);
    };
    const endScrollbarDrag = (event?: globalThis.PointerEvent) => {
        const drag = scrollbarDrag.current;
        if (!drag || event && event.pointerId !== drag.id) return;
        scrollbarDrag.current = undefined;
        if (scrollbar.current) delete scrollbar.current.dataset.dragging;
        if (drag.capture.hasPointerCapture(drag.id)) drag.capture.releasePointerCapture(drag.id);
    };
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
        const messageSubscription = listenIslandMessages(
            (next) => {
                if (active) {
                    messagesRevision.current++;
                    setMessages(next);
                    setMessageFailure(undefined);
                }
            },
            (error) => {
                if (active) setMessageFailure(error instanceof Error ? error.message : String(error)); // 消息通道失败只降级到岛卡。
            },
        );
        void messageSubscription.catch((error: unknown) => {
            if (active) setMessageFailure(error instanceof Error ? error.message : String(error));
        });
        void getDesktopHostSnapshot()
            .then((desktop) => {
                const monitor = desktop.hosts.find((host) => host.monitor.primary)?.monitor;
                if (active && monitor) setMonitorId(monitor.id);
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
            stopHoverTimer();
            void subscription.then((stop) => stop()).catch(() => undefined);
            void messageSubscription.then((stop) => stop()).catch(() => undefined);
        };
    }, []);
    useEffect(() => {
        heldMessages.current = islandHoldMessages(heldMessages.current, messages, mode); // 阅读期间消失的消息暂留，回到常态清空。
    }, [mode, messages]);
    useEffect(() => {
        let active = true; // 卸载后拒绝迟到回调。
        let stop: (() => void) | undefined;
        void listenIslandDropActive(
            (next) => {
                if (!active) return;
                const fact = islandNativeDropFact(next);
                dropActiveRef.current = Boolean(fact);
                setNativeDrop(fact);
                if (fact) {
                    stopHoverTimer(); // 投放期间不按悬停消抖回落。
                    if (modeRef.current === "collapsed") changeMode("hover");
                    return;
                }
                void islandCursorInside().then((inside) => {
                    if (!active || dropActiveRef.current) return;
                    pointerInside.current = inside;
                    if (!inside && modeRef.current !== "collapsed") changeMode("collapsed");
                }).catch(() => {
                    if (active && !dropActiveRef.current && !pointerInside.current) changeMode("collapsed");
                });
            },
        )
            .then((unlisten) => {
                if (active) stop = unlisten;
                else unlisten();
            })
            .catch((error: unknown) => console.warn("浮岛投放状态订阅失败", error));
        return () => {
            active = false;
            stop?.();
        };
    }, []);
    const edge = preferences?.islandEdge ?? "";
    const vertical = mode === "collapsed" && (edge === "left" || edge === "right"); // 仅胶囊态在左右贴边时竖排；hover 与展开保持水平卡片。
    const instances = snapshot?.instances ?? [];
    const sortable = instances; // 卡片身份同一份，不另生成排序列表。
    const visibleMessages = islandMessageView(messages, heldMessages.current, "expanded");
    const drop = dropFact ?? nativeDrop;
    dropActiveRef.current = Boolean(drop);
    const dropFace = drop ? islandDropFace(drop === "reject" ? undefined : drop) : undefined;
    const dropAnchor: IslandAnchor | undefined = dropFace ? { id: "drop", source: "拖放", layer: "drop", updatedAtMs: 0, unit: { primary: dropFace.primary, status: "" } } : undefined;
    const { music, position } = useIslandMusic(mode, instances.some((instance) => instance.componentId === "official.music-control"));
    const media = music.snapshot;
    const musicView = islandLyricView(music.lyrics, position, media);
    const musicProgress = musicProgressRange(music.timeline, music.lyrics, position);
    const musicPercent = Math.min(100, Math.max(0, Math.floor(musicProgress.value / musicProgress.max * 100)));
    const musicStatus: IslandStatus = !media ? "disconnected" : media.playback === "playing" ? (music.timeline?.durationMs ?? 0) > 0 ? "running" : "indeterminate" : media.playback === "paused" ? "paused" : "pending";
    const weatherInstance = instances.find((instance) => instance.componentId === "official.weather");
    const weather = useIslandWeather(weatherInstance?.instanceId, monitorId);
    const hasCompactCollect = mode !== "expanded" && instances.some((instance) => instance.componentId === "official.collect-box");
    const [collectPreview, setCollectPreview] = useState<CollectItem | undefined>(() => readIslandCollectCache()?.[0]);
    useEffect(() => {
        if (!hasCompactCollect) return;
        let active = true;
        let generation = 0;
        const refresh = () => {
            if (window.__DESKTOP_TOOLS_NATIVE__?.isRefreshPaused?.()) return;
            const id = ++generation;
            void getCollectItems().then((items) => {
                if (!active || generation !== id) return;
                writeIslandCollectCache(items);
                setCollectPreview(items[0]);
            }).catch(() => undefined);
        };
        refresh();
        const listener = listenCollectChanges(refresh, () => undefined).catch(() => undefined);
        window.addEventListener("desktop-refresh-changed", refresh);
        return () => { active = false; window.removeEventListener("desktop-refresh-changed", refresh); void listener.then((stop) => stop?.()); };
    }, [hasCompactCollect]);
    const collectTitle = collectPreview ? islandCollectTitle(collectPreview) : "收集箱";
    const arrivals = useMemo(() => {
        const resident = (snapshot?.instances ?? []).map((instance): IslandAnchorArrival => {
            const manifest = snapshot?.runtime.manifests.find((item) => item.id === instance.componentId);
            const registration = snapshot?.runtime.instances.find((item) => item.instanceId === instance.instanceId);
            const source = manifest?.name ?? instance.componentId;
            const status = registration?.status === "failed" ? "failed" : registration?.status === "ready" ? "pending" : "disconnected";
            const kind: IslandFunctionKind = instance.componentId === "official.music-control" ? "music" : instance.componentId === "official.collect-box" ? "collect" : instance.componentId === "official.weather" ? "weather" : "plugin";
            return { source, layer: "resident", nowMs: 0, kind, unit: instance.componentId === "official.music-control"
                ? { status: musicStatus, primary: musicView.line, secondary: musicStatus === "running" ? `${musicPercent}%` : undefined }
                : instance.componentId === "official.weather" ? { status: weather.unit.status, primary: weather.unit.primary }
                    : { status, primary: instance.componentId === "official.collect-box" ? collectTitle : source } };
        });
        return [...resident, ...contentFacts];
    }, [snapshot, musicStatus, musicView.line, musicPercent, contentFacts, weather.unit.status, weather.unit.primary, collectTitle]);
    const content = useIslandContent(arrivals, messages, mode, statusSpecs);
    const baseAnchors = content.anchors.filter((anchor) => islandStatusSpec(anchor.unit?.status ?? "", statusSpecs)?.disposition !== "vanish");
    const anchors = dropAnchor ? [dropAnchor, ...baseAnchors] : baseAnchors;
    const face = islandCapsuleFace(anchors, content.clockNow);
    const capsuleIds = face.kind === "anchors" && mode === "collapsed" ? face.visible.map((anchor) => anchor.id) : [];
    const previousCapsule = useRef<{ ids: string[]; front?: string }>({ ids: [] });
    const selected = dropAnchor ?? anchors.find((anchor) => anchor.id === islandHoverSelect(anchors, content.selected));
    const foreground = dropAnchor ?? content.active;
    const capsuleActive = islandStatusSpec(foreground?.unit?.status ?? "", statusSpecs)?.disposition === "dot" ? undefined : foreground;
    const relayFrom = previousCapsule.current.front;
    const previousIds = previousCapsule.current.ids;
    useLayoutEffect(() => { previousCapsule.current = { ids: capsuleIds, front: mode === "collapsed" ? capsuleActive?.id : undefined }; });
    const musicSource = snapshot?.runtime.manifests.find((item) => item.id === "official.music-control")?.name;
    const weatherSource = snapshot?.runtime.manifests.find((item) => item.id === "official.weather")?.name;
    const isWeather = (anchor: IslandAnchor | undefined) => anchor?.layer === "resident" && anchor.source === weatherSource;
    const weatherIcon = (anchor: IslandAnchor) => isWeather(anchor) && weather.summary?.code && weather.unit.status === "pending" ? <WeatherGlyph code={weather.summary.code} /> : undefined;
    const isMusic = (anchor: IslandAnchor | undefined) => anchor?.layer === "resident" && anchor.source === musicSource;
    const selectedMessage = selected?.layer === "message" ? visibleMessages.find((message) => message.source === selected.source && message.receivedAtMs === selected.updatedAtMs) : undefined;
    const dismissAllowed = selectedMessage && selected?.unit?.status !== "awaiting" && selected?.unit?.status !== "failed";
    const availableControls = isMusic(selected)
        ? islandMusicCommands(media).filter(([, , enabled]) => enabled).map(([action]) => action)
        : dismissAllowed ? ["dismiss"] : [];
    const hoverText = islandHoverContent(selected, availableControls);
    const hoverValue = hoverText.lines[1];
    const hoverSecondary = isMusic(selected) ? media?.artist : isWeather(selected) ? weather.detail : islandAnchorValue(hoverValue ?? "") ? hoverValue : undefined;
    const status = foreground?.unit?.status;
    const announcementValue = islandAnchorValue(foreground?.unit?.secondary ?? "");
    const milestone = status === "running" && announcementValue?.kind === "percent" ? Math.floor(announcementValue.value / 25) * 25 : undefined;
    const announcement = !foreground ? "" : foreground.layer === "drop" ? foreground.unit?.primary :
        [foreground.source, islandStatusSpec(status ?? "", statusSpecs)?.word, status === "running" ? milestone && milestone < 100 ? `${milestone}%` : undefined : foreground.unit?.primary].filter(Boolean).join(" · ");
    const announcementState = useRef({ text: "", added: 0, ids: new Set<string>() });
    const [liveText, setLiveText] = useState("");
    useEffect(() => {
        const state = announcementState.current;
        const ids = new Set(baseAnchors.map((anchor) => anchor.id));
        const added = [...ids].filter((id) => !state.ids.has(id)).length;
        state.ids = ids;
        state.added += added; // 本次内容改写伴随新增时，同批也要计入本次播报。
        if (announcement !== state.text) {
            const pending = state.added;
            state.text = announcement ?? "";
            if (announcement) state.added = 0; // 只有实际播报才消耗积攒的计数。
            setLiveText(announcement && pending ? `${announcement} · 新增 ${pending} 条` : announcement ?? "");
        }
    }, [announcement, baseAnchors.map((anchor) => anchor.id).join("\n")]);
    const hoverContentRef = useRef<HTMLDivElement>(null);
    const hoverList = useRef<HTMLDivElement>(null);
    /** 胶囊卡片宽度：hover 卡片不得窄于它，否则指针停在原胶囊范围会被判成透明边距而回退。 */
    const capsuleInline = useRef(0);
    const handleHoverKey = (event: React.KeyboardEvent<HTMLDivElement>) => {
        if (event.key === "ArrowLeft" || event.key === "ArrowRight") {
            event.preventDefault();
            const next = islandHoverStep(anchors, selected?.id, event.key === "ArrowLeft" ? -1 : 1);
            content.setSelected(next);
            Array.from(hoverList.current?.querySelectorAll<HTMLButtonElement>("button") ?? []).find((item) => item.dataset.anchor === next)?.focus();
        } else if (event.key === "Enter" && islandHoverEnter(selected)) {
            event.preventDefault();
            const detail = hoverContentRef.current;
            (detail?.querySelector<HTMLButtonElement>("button:not(:disabled)") ?? detail)?.focus();
        }
    };
    // 卡内收起态：记忆开启时读偏好，关闭记忆时回落到该卡默认；收集箱默认收起，其余默认展开。
    const collapseMemory = preferences?.islandCardCollapseMemory ?? {};
    const collapseDefault = (componentId: string) => componentId === "official.collect-box" ? (preferences?.islandCollectCollapsedDefault ?? true) : false;
    // 记忆开关只决定是否写偏好，运行期收起态始终可切换。
    const cardCollapsed = (componentId: string) => collapsedCards[componentId] ?? collapseDefault(componentId);
    useLayoutEffect(() => {
        // 只回填记忆开启的卡片；关闭记忆的卡片按默认值显示，不受旧保存值影响。
        const memory = preferences?.islandCardCollapseMemory ?? {};
        const stored = preferences?.islandCardCollapsed ?? {};
        setCollapsedCards(Object.fromEntries(Object.entries(stored).filter(([cardId]) => memory[cardId] !== false)));
    }, [preferences?.islandCardCollapsed, preferences?.islandCardCollapseMemory]);
    /** 收起/展开一张卡；先更新页面，再把整份收起表写回偏好。 */
    const handleToggleCard = (componentId: string) => {
        const next = { ...collapsedCards, [componentId]: !cardCollapsed(componentId) };
        setCollapsedCards(next);
        if (collapseMemory[componentId] === false) return; // 关闭记忆时只改当前会话，不写偏好。
        void saveIslandCardCollapsed(next).catch((error: unknown) => console.warn("浮岛卡收起状态保存失败", error));
    };
    /** 拖出结束后按原生光标判态：OLE 拖动期间页面收不到 pointerout，不能只等悬停事件。 */
    const handleCollectDragEnd = () => {
        void islandCursorInside()
            .then((inside) => {
                if (!mounted.current) return;
                pointerInside.current = inside;
                if (!inside && modeRef.current === "expanded") changeMode("collapsed");
            })
            .catch(() => undefined);
    };
    const edgeKey = useRef(""); // 上次测量的贴边方向，变化时丢弃旧几何缓存。
    const formKey = useRef(""); // 上次测量的岛形态，变化时丢弃旧胶囊高度与并集缓存。
    useLayoutEffect(() => {
        if (!preferences || !work || !root.current) return;
        let active = true;
        let frame = 0;
        let motionFrame = 0;
        let wakeMotion: (() => void) | undefined;
        let animating = false;
        let committing = false;
        let needsMeasure = false;
        let lastCompactFrame = -Infinity;
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
            edgeKey.current = nextEdge;
            formKey.current = nextForm;
            collapsedBox.current = undefined;
            geometryKey.current = "";
            currentGeometry.current = undefined;
            targetGeometry.current = undefined; inputReady.current = false; layoutSettled.current = false;
            inputGeometryKey.current = ""; stopHoverTimer(); marginState.current = { latch: "open" };
            cancelCollapseWait(); endScrollbarDrag(); setScrollbarVisible(false, true);
            pendingOffset.current = undefined; // 旧边/旧形态的拖动落点不再适用。
        }
        const measureNow = (force = false) => {
            if (dragging.current) return;
            if (animating || committing) { needsMeasure = true; return; }
            const now = performance.now();
            if (mode === "collapsed" && !force && now - lastCompactFrame < ISLAND_FRAME_MS) {
                cancelAnimationFrame(frame);
                frame = requestAnimationFrame(() => measureNow());
                return;
            }
            const painted = paintedGeometry.current;
            // 展开根是保留union，根尺寸不变不能证明卡片内容未变化；展开RO必须重测自然目标。
            if (!force && mode !== "expanded" && root.current && painted) {
                const box = root.current.getBoundingClientRect();
                // paint 写入的根尺寸会触发 ResizeObserver；它不改变内容，不能借机 restore 成自然布局。
                if (mode === painted.state && Math.abs(box.width - painted.width) < 0.6 && Math.abs(box.height - painted.height) < 0.6) return;
            }
            cancelAnimationFrame(frame);
            restoreStyles(); // 动画被打断或切态竞争时清掉上一轮内联尺寸，测量必须基于真实文档流。
            {
                if (!active || !root.current || dragging.current) return;
                // 暂时移除保留窗口的尺寸约束，只测自然内容目标，避免用旧union高度截掉新增内容。
                // 同一次同步测量内恢复变量和scrollTop，paint自触发的RO重放会由geometryKey去重，不重复提交。
                const element = root.current;
                const expandedInline = element.style.getPropertyValue("--island-expanded-inline");
                const expandedBlock = element.style.getPropertyValue("--island-expanded-block");
                const scrollTop = element.scrollTop;
                if (mode === "expanded") {
                    element.style.removeProperty("--island-expanded-inline");
                    element.style.removeProperty("--island-expanded-block");
                }
                const origin = element.getBoundingClientRect();
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
                if (mode === "expanded") {
                    if (expandedInline) element.style.setProperty("--island-expanded-inline", expandedInline);
                    if (expandedBlock) element.style.setProperty("--island-expanded-block", expandedBlock);
                    element.scrollTop = scrollTop;
                }
                if (!measured.length) return;
                try {
                    const radius = mode === "collapsed" ? 22 : mode === "hover" ? 16 : preferences.islandRadius;
                    const pending = pendingOffset.current;
                    // 默认空边按顶部居中；拖动返回的显式偏移必须按有效边 top 计算，不能在中点丢掉。
                    const geometryEdge = pending !== undefined && edge === "" ? "top" : edge;
                    let request = islandGeometry(mode, measured, radius, work, geometryEdge, pending ?? preferences.islandOffset, preferences.islandForm);
                    if (mode === "collapsed") {
                        capsuleInline.current = measured[0]?.width ?? 0;
                        const circles = Array.from(root.current.querySelectorAll<HTMLElement>(".island-anchor-row .island-anchor"));
                        request = { ...request, anchors: circles.slice(0, ISLAND_ANCHOR_SLOTS + 1).map((circle) => {
                            const bounds = circle.getBoundingClientRect();
                            return { rect: { left: bounds.left - origin.left, top: bounds.top - origin.top, width: bounds.width, height: bounds.height }, radius: bounds.width / 2 };
                        }) };
                    }
                    const natural = request;
                    // hover 窗口与胶囊态取并集：指针停在原胶囊范围任意位置都不会离开窗口，避免左右贴边来回切态。
                    if (mode === "hover" && collapsedBox.current) {
                        request = { ...islandUnionFrame(request, collapsedBox.current, 0, edge, work), state: "hover" };
                    }
                    if (mode === "collapsed") {
                        collapsedBox.current = request;
                    }
                    const key = JSON.stringify(request);
                    const element = root.current;
                    const cards = Array.from(element.querySelectorAll<HTMLElement>(".island-card"));
                    const from = currentGeometry.current;
                    const animate = animationsEnabled && from && from.state !== request.state && !dragging.current && !skipMotionOnce.current;
                    const form = preferences.islandForm ?? "capsule";
                    const filletMask = (slot: "start" | "end"): [string, string] => {
                        switch (edge) {
                            case "bottom":
                                return slot === "start" ? ["0%", "0%"] : ["100%", "0%"];
                            case "left":
                                return slot === "start" ? ["100%", "0%"] : ["100%", "100%"];
                            case "right":
                                return slot === "start" ? ["0%", "0%"] : ["0%", "100%"];
                            default:
                                return slot === "start" ? ["0%", "100%"] : ["100%", "100%"];
                        }
                    };
                    // 单层背景按真实DOM卡片生成遮罩；native cards包含透明命中gap，不能直接把它当可见表面绘制。
                    const paintSurface = (attached: number, value: IslandStateRequest, moving: boolean) => {
                        const node = surface.current;
                        if (!node || attached >= -0.01 || !cards.length) {
                            if (node) node.style.display = "none";
                            return;
                        }
                        const radius = -attached;
                        const layers: string[] = [];
                        const positions: string[] = [];
                        const sizes: string[] = [];
                        // 直接以surface为局部原点，滚动容器中它可能不等于根的边框原点。
                        node.style.display = "block";
                        const origin = node.getBoundingClientRect();
                        const seamPaths: string[] = [];
                        const seamBands: IslandRect[] = [];
                        for (const card of cards) {
                            // 相位 1 内其他卡完全隐藏，不参与表面遮罩与接缝，避免重复填充与错误补面。
                            if (Number.parseFloat(getComputedStyle(card).opacity) < 0.01) continue;
                            const box = card.getBoundingClientRect();
                            const rect = { left: box.left - origin.left, top: box.top - origin.top, width: box.width, height: box.height };
                            const style = getComputedStyle(card);
                            const corners = [style.borderTopLeftRadius, style.borderTopRightRadius, style.borderBottomRightRadius, style.borderBottomLeftRadius].map(Number.parseFloat);
                            const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${rect.width}" height="${rect.height}"><path d="${islandCardMaskPath(rect.width, rect.height, corners)}" fill="#fff"/></svg>`;
                            layers.push(`url("data:image/svg+xml,${encodeURIComponent(svg)}")`);
                            positions.push(`${rect.left}px ${rect.top}px`);
                            sizes.push(`${rect.width}px ${rect.height}px`);
                            const seam = islandSeamMask(rect, corners, value.windowBox, edge, form, attached, window.devicePixelRatio);
                            if (seam) { seamPaths.push(seam.path); seamBands.push(...seam.bands); }
                            for (const patch of islandFilletPatches(rect, value.windowBox, edge, radius)) {
                                const [x, y] = filletMask(patch.slot);
                                // 原瓦片尺寸/位置不动；中帧圆心与色标取绝对px，缺覆盖由根尺寸单路径补采样。
                                const centerX = x === "100%" ? patch.rect.width : 0;
                                const centerY = y === "100%" ? patch.rect.height : 0;
                                // 稳态连渐变文本也复用原版，避免等价圆心表达式造成最后1级光栅量化差。
                                layers.push(moving
                                    ? `radial-gradient(circle ${patch.radius}px at ${centerX}px ${centerY}px, transparent ${patch.radius - 0.5}px, #000 ${patch.radius}px)`
                                    : `radial-gradient(circle at ${x} ${y}, transparent ${patch.radius - 0.5}px, #000 ${patch.radius}px)`);
                                positions.push(`${patch.rect.left}px ${patch.rect.top}px`);
                                sizes.push(`${patch.rect.width}px ${patch.rect.height}px`);
                            }
                        }
                        if (seamPaths.length) {
                            // clipPath位于SVG内部，不是CSS mask层的clip-path；单张根尺寸瓦片没有接缝处的小瓦片边界。
                            const width = value.windowBox.width, height = value.windowBox.height;
                            const clips = seamBands.map((band) => `<rect x="${band.left}" y="${band.top}" width="${band.width}" height="${band.height}"/>`).join("");
                            const svg = `<svg xmlns="http://www.w3.org/2000/svg" data-island-seam="true" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}"><defs><clipPath id="seam-band" clipPathUnits="userSpaceOnUse">${clips}</clipPath></defs><path d="${seamPaths.join(" ")}" clip-path="url(#seam-band)" fill="#fff"/></svg>`;
                            layers.push(`url("data:image/svg+xml,${encodeURIComponent(svg)}")`);
                            positions.push("0px 0px");
                            sizes.push(`${width}px ${height}px`);
                        }
                        // 原三层外缘保持不动，所有负角帧增加一张根尺寸单path补采样（包括分数DPR稳态）；同一surface只填色一次。
                        node.style.maskImage = layers.join(", ");
                        node.style.maskPosition = positions.join(", ");
                        node.style.maskSize = sizes.join(", ");
                        node.style.maskRepeat = "no-repeat";
                        node.style.setProperty("-webkit-mask-image", layers.join(", "));
                        node.style.setProperty("-webkit-mask-position", positions.join(", "));
                        node.style.setProperty("-webkit-mask-size", sizes.join(", "));
                        node.style.setProperty("-webkit-mask-repeat", "no-repeat");
                    };
                    // 补面与描边状态独立于布局写入；常态自然流下也能单独重放，不锁死内联尺寸。
                    const paintAttached = (attached: number, value: IslandStateRequest, moving = false) => {
                        attachedCorner.current = attached;
                        element.dataset.filleted = attached < -0.01 ? "true" : "false";
                        paintSurface(attached, value, moving);
                    };
                    // 固定视口偏移只定位根节点；展开态不写回卡片内联几何，但仍需要正确 margin。
                    const applyViewportMargin = (value: IslandStateRequest) => {
                        element.style.marginInlineStart = `${value.windowBox.left - value.viewport.left}px`;
                        element.style.marginBlockStart = `${value.windowBox.top - value.viewport.top}px`;
                    };
                    const paint = (value: IslandStateRequest, elapsed: number, start?: number, staggerMs = 0) => {
                        paintedGeometry.current = { state: value.state, width: value.windowBox.width, height: value.windowBox.height };
                        saveStyle(element);
                        Object.assign(element.style, {
                            position: "relative",
                            // 固定页面视口中的窗口偏移；父窗口移动只改变裁剪，内容屏幕位置由 margin 决定。
                            marginInlineStart: `${value.windowBox.left - value.viewport.left}px`,
                            marginBlockStart: `${value.windowBox.top - value.viewport.top}px`,
                            inlineSize: `${value.windowBox.width}px`,
                            blockSize: `${value.windowBox.height}px`,
                            maxInlineSize: "none",
                            overflow: "clip",
                        });
                        // 贴边侧角从最近实际绘制的带符号圆角过渡到目标态，连续切态不会先跳回普通圆角再长出补面。
                        const end = islandAttachedCorner(request.state, form, request.cards[0]?.radius ?? 0);
                        const attached = elapsed >= ISLAND_MOTION_MS ? end : islandAttachedCornerAt(start ?? end, request, elapsed, form);
                        // 内容与形变同步淡入并去模糊：起点透明 5px，终点完全清晰；终帧与静态态清空内联值。
                        cards.forEach((card, index) => {
                            const shape = value.cards[index];
                            if (!shape) return;
                            // 展开相位 2：其他卡延迟 staggerMs 起步，内容与卡片本身同步淡入并去模糊。
                            const localElapsed = staggerMs > 0 && index > 0 ? Math.max(0, elapsed - staggerMs) : elapsed;
                            const reveal = localElapsed >= ISLAND_MOTION_MS ? 1 : Math.min(1, Math.max(0, localElapsed / ISLAND_MOTION_MS));
                            saveStyle(card);
                            Object.assign(card.style, {
                                position: "absolute",
                                left: `${shape.rect.left}px`,
                                top: `${shape.rect.top}px`,
                                inlineSize: `${shape.rect.width}px`,
                                blockSize: `${shape.rect.height}px`,
                                maxInlineSize: "none",
                                minBlockSize: "0",
                                borderRadius: islandCornerRadius(shape.radius, form, edge, attached),
                                zIndex: String(cards.length - index), // 当前卡压在其他卡之上，其他卡从它背后弹出。
                            });
                            if (staggerMs > 0 && index > 0) card.style.opacity = `${reveal}`; // 相位 1 内其他卡完全隐藏。
                            const content = card.querySelector<HTMLElement>(".island-card__content");
                            if (!content) return;
                            saveStyle(content);
                            if (reveal >= 1) {
                                content.style.removeProperty("opacity");
                                content.style.removeProperty("filter");
                            } else {
                                content.style.opacity = `${reveal}`;
                                content.style.filter = `blur(${5 * (1 - reveal)}px)`;
                            }
                        });
                        paintAttached(attached, value, elapsed < ISLAND_MOTION_MS);
                    };
                    // 展开态不写回内联几何，必须单独收回上一帧的补面与描边抑制，否则会残留贴边弧面。
                    const clearFillets = () => {
                        element.dataset.filleted = "false";
                        attachedCorner.current = undefined; // 展开态没有贴边负圆角，下一次从目标态重新推导。
                        if (surface.current) surface.current.style.display = "none";
                    };
                    if (key === geometryKey.current) {
                        // 同一几何被重新测量时仍要把窗口布局与补面写回；否则并集窗口/刘海内缩、
                        // 换边后上次提交被取消等情况会退回自然布局或残留上一贴边的补面。
                        if (request.state === "expanded") {
                            clearFillets();
                            applyViewportMargin(request);
                        } else {
                            // 稳态无需固定尺寸：hover 的原生命中可保留并集，绘制与实测仍以自然内容盒为原点。
                            applyViewportMargin(natural);
                            paintAttached(islandAttachedCorner(natural.state, form, natural.cards[0]?.radius ?? 0), natural);
                        }
                        // 去重只针对原生几何提交：内容变高但裁剪后 request 不变时，滚动指标仍可能变化，
                        // 同一 key 早退也必须刷新条的可见性、thumb/range 与命中排除矩形，且不产生重复 IPC。
                        targetGeometry.current = request; // 同尺寸下内容盒可能变化，滚动与命中判定必须用最新实测。
                        refreshScrollbar();
                        return;
                    }
                    geometryKey.current = key;
                    committing = true;
                    lastCompactFrame = now;
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
                        paint(from, 0, attachedCorner.current ?? islandAttachedCorner(from.state, form, from.cards[0]?.radius ?? 0)); // 起点帧按起点态自己的窗口与坐标，不能把目标布局画进旧窗口。
                    }
                    const commit = async (value: IslandStateRequest) => {
                        await setIslandState(value);
                        currentGeometry.current = value;
                        if (!dragging.current) inputReady.current = true;
                    };
                    finish = submission.current
                        .catch(() => undefined)
                        .then(async () => {
                            if (!active || !mounted.current) return;
                            let settled = false; // 只有成功补完最终态才允许补测，失败路径保持一次重试，不形成持续提交。
                            try {
                                const startGeometry = currentGeometry.current ?? from; // 等旧提交落定后再取衔接点。
                                const staggerMs = request.state === "expanded" && animate && request.cards.length > 1 ? ISLAND_EXPAND_STAGGER_MS : 0; // 展开时其他卡延迟弹出。
                                const motionTotal = staggerMs > 0 ? ISLAND_EXPAND_MOTION_MS : ISLAND_MOTION_MS; // 展开比其余切态多一段重叠。
                                if (animate && startGeometry) {
                                    // 先把原生窗口一次放大到两态并集，内容保持原位置；动画只在固定窗口内移动内容。
                                    const startAttached =
                                        attachedCorner.current ?? islandAttachedCorner(startGeometry.state, form, startGeometry.cards[0]?.radius ?? 0); // 从实际绘制值衔接连续切态。
                                    const unionFirst = islandUnionFrame(startGeometry, request, 0, edge, work, staggerMs);
                                    if (staggerMs > 0)
                                        for (const card of cards.slice(1)) {
                                            saveStyle(card);
                                            card.style.opacity = "0"; // 窗口放大前先藏起其他卡，避免自然流露出一帧。
                                        }
                                    await commit(unionFirst); // 先放大窗口再写并集布局，避免目标内容被旧窗口裁切出一帧。
                                    paint(unionFirst, 0, startAttached, staggerMs);
                                    await new Promise<void>((resolve) => {
                                        motionFrame = requestAnimationFrame(() => resolve());
                                    });
                                    if (!active) return;
                                    const started = performance.now();
                                    let last = -Infinity;
                                    let previousKey = "";
                                    while (active && !forceFinish) {
                                        const elapsed = await new Promise<number>((resolve) => {
                                            wakeMotion = () => resolve(motionTotal);
                                            motionFrame = requestAnimationFrame((now) => {
                                                wakeMotion = undefined;
                                                resolve(now - started);
                                            });
                                        });
                                        if (!active || elapsed >= motionTotal || forceFinish) break;
                                        if (elapsed - last < ISLAND_FRAME_MS) continue;
                                        last = elapsed;
                                        const value = islandUnionFrame(startGeometry, request, elapsed, edge, work, staggerMs);
                                        const frameKey = JSON.stringify(value);
                                        if (frameKey !== previousKey) {
                                            await commit(value);
                                            if (!active) break;
                                            paint(value, elapsed, startAttached, staggerMs);
                                            previousKey = frameKey;
                                        }
                                    }
                                }
                                if (active) {
                                    // 展开态 root 是滚动容器：不写回固定内联几何，保留自然流和 scrollTop。
                                    const grows =
                                        !from || request.windowBox.width > from.windowBox.width + 0.5 || request.windowBox.height > from.windowBox.height + 0.5;
                                    if (request.state === "expanded") {
                                        // 展开终态保留入态并集窗口，不收缩到目标盒；内容仍按目标 margin 落位。
                                        // 有切态动画时保留入态并集；同态重测（内容增减、卡内收起）直接提交自然目标，窗口随内容收缩。
                                        const expandedFrame = animate ? islandUnionFrame(startGeometry ?? from ?? request, request, motionTotal, edge, work, staggerMs) : request;
                                        await commit(expandedFrame);
                                        if (active) {
                                            clearFillets();
                                            applyViewportMargin(request);
                                            targetGeometry.current = request;
                                            // 宽度取展开目标本身而不是入态并集：目标宽度在动画前已实测，动画结束后不再被并集撑宽后重排。
                                            element.style.setProperty("--island-expanded-inline", `${request.windowBox.width}px`);
                                            element.style.setProperty("--island-expanded-block", `${expandedFrame.windowBox.height}px`);
                                            currentGeometry.current = expandedFrame;
                                            paintedGeometry.current = {
                                                state: "expanded",
                                                width: expandedFrame.windowBox.width,
                                                height: expandedFrame.windowBox.height,
                                            };
                                        }
                                    } else if (!animate && grows) {
                                        // 目标比当前窗口大时先提交窗口再绘制，否则旧窗口会裁掉新布局一帧；动画路径的窗口已固定为并集。
                                        await commit(request);
                                        if (active) paint(request, ISLAND_MOTION_MS);
                                    } else {
                                        paint(request, ISLAND_MOTION_MS);
                                        await commit(request);
                                    }
                                    recovering.current = false;
                                    settled = true;
                                } // 有界动画结束必须补最终态，不受去重键影响。
                            } catch (error) {
                                currentGeometry.current = undefined;
                                geometryKey.current = "";
                                console.warn("岛状态提交失败", error);
                                if (active && mounted.current) {
                                    changeMode("collapsed"); // 原生仍执行 fail-closed，恢复从常态开始。
                                    if (!recovering.current) {
                                        recovering.current = true;
                                        setDragRevision((value) => value + 1);
                                    }
                                }
                            } finally {
                                animating = false;
                                committing = false;
                                restoreStyles();
                                if (settled && active && request.state !== "expanded") {
                                    applyViewportMargin(natural);
                                    paintAttached(islandAttachedCorner(natural.state, form, natural.cards[0]?.radius ?? 0), natural);
                                }
                                if (settled && active && modeRef.current === mode) {
                                    inputGeometryKey.current = key;
                                    if (targetGeometry.current?.state === "expanded") {
                                        layoutSettled.current = true;
                                        refreshScrollbar(); // 最终提交并恢复文档流后才显示条。
                                    }
                                    if (needsMeasure || request.state === "expanded") {
                                        needsMeasure = false;
                                        remeasure.current(true);
                                    }
                                }
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
                    // 内容超过工作区等原因提交不了新几何时，条仍要按当前窗口与内容同步。
                    refreshScrollbar();
                }
            }
        };
        const measure = (force = false) => {
            cancelAnimationFrame(frame);
            frame = requestAnimationFrame(() => measureNow(force));
        };
        remeasure.current = measure; // 供消息集合变化等无 DOM 尺寸变化的内容更新主动调用。
        const observer = new ResizeObserver(() => measure(false));
        observer.observe(root.current);
        const handleScroll = () => { refreshScrollbar(); measure(true); }; // 展开态滚动必须重新计算可见卡片，不受自触发 resize 跳过影响。
        root.current.addEventListener("scroll", handleScroll);
        for (const card of root.current.querySelectorAll(".island-card")) observer.observe(card);
        const contentObserver = new ResizeObserver(() => measure(true));
        const compactContent = root.current.querySelector(".island-card--anchors .island-card__content");
        if (compactContent) contentObserver.observe(compactContent);
        measureNow(true); // 首帧绘制前完成目标实测与内容冻结，不先闪出最终布局。
        const element = root.current;
        return () => {
            active = false;
            remeasure.current = () => {}; // 卸载后不再调度测量帧。
            element.removeEventListener("scroll", handleScroll);
            observer.disconnect();
            contentObserver.disconnect();
            cancelAnimationFrame(frame);
            cancelAnimationFrame(motionFrame);
            wakeMotion?.();
            restoreStyles();
        };
    }, [mode, snapshot, preferences, work, edge, animationsEnabled, dragRevision]);
    useEffect(() => {
        // 只有展开态的根尺寸被并集固定；消息卡增减不会触发 RO，必须主动重测。
        remeasure.current(true);
    }, [messages, messageFailure, mode, anchors.length, drop, mode === "hover" ? selected?.id : undefined, mode === "hover" ? hoverText.lines.join("\n") : undefined]);
    useEffect(() => {
        const pending = pendingOffset.current;
        if (pending === undefined || !preferences) return;
        if ((preferences.islandEdge || "top") === (edge || "top") && Math.abs(preferences.islandOffset - pending) < 0.01) pendingOffset.current = undefined; // 权威落点已到，不再压住后续偏移。
    }, [preferences, edge]);
    const interactionBusy = () => dragging.current || Boolean(gesture.current) || Boolean(scrollbarDrag.current);
    // 常态关闭透明像素检测：门闩只在 hover 与展开态参与判定，常态进入仍是原 180ms 消抖。
    const hoverLatched = () => modeRef.current !== "collapsed" && marginState.current.latch !== "open";
    const scheduleHover = (inside: boolean) => {
        const target = islandHoverTarget(modeRef.current, inside, interactionBusy(), hoverLatched());
        const kind = inside ? "enter" : "leave";
        if (target && hoverKind.current === kind && hoverTimer.current !== undefined) return;
        stopHoverTimer();
        if (!target) return;
        hoverKind.current = kind;
        const version = hoverVersion.current;
        hoverTimer.current = setTimeout(() => {
            if (version !== hoverVersion.current || !mounted.current) return;
            hoverTimer.current = undefined; hoverKind.current = undefined;
            if (pointerInside.current === inside && islandHoverTarget(modeRef.current, inside, interactionBusy(), hoverLatched()) === target) changeMode(target);
        }, ISLAND_HOVER_DELAY_MS);
    };
    const leaveWindow = () => {
        pointerInside.current = false;
        if (hoverKind.current !== "leave") stopHoverTimer();
        cancelDragHold(); marginState.current.since = undefined;
        if (interactionBusy()) return;
        marginState.current = islandMarginStep(marginState.current, { mode: modeRef.current, region: "outside", now: performance.now(), busy: false, outlineDistance: Infinity, coreDistance: Infinity, leave: true, detect: modeRef.current !== "collapsed" }).state;
        if (dropActiveRef.current) return; // 投放拖动期间页面指针本就不可信，保持当前态等原生投放事件收口。
        if (modeRef.current === "expanded") changeMode("collapsed"); else scheduleHover(false);
    };
    const handleLeave = (event: PointerEvent<HTMLElement>) => {
        // 根之外仍可能是union内的body透明区，交给pointermove判定；只把真正离开文档当传统离岛。
        if (event.relatedTarget instanceof Node && document.documentElement.contains(event.relatedTarget)) return;
        leaveWindow();
    };
    const handleEnter = () => {
        pointerInside.current = true;
        scheduleHover(true);
    };
    // 文档级事件覆盖union余量（它可能在main根之外），但只在pointermove上评估透明区域。
    inputHandlers.current = {
        move: (event) => {
            const drag = scrollbarDrag.current;
            if (drag) {
                if (event.pointerId === drag.id && root.current && drag.travel > 0) root.current.scrollTop = drag.top + (event.clientY - drag.y) * drag.range / drag.travel;
                return;
            }
            if (event.pointerType === "touch") return;
            if (interactionBusy() || !inputReady.current) {
                marginState.current.since = undefined;
                if (hoverKind.current === "margin") stopHoverTimer();
                return;
            }
            const element = root.current, frame = currentGeometry.current;
            if (!element || !frame) return;
            const point = { x: event.clientX, y: event.clientY };
            const layout = islandScrollGeometry(frame, targetGeometry.current, preferences?.islandEdge ?? "", { mode: modeRef.current, settled: layoutSettled.current, scrollHeight: element.scrollHeight, clientHeight: element.clientHeight, scrollTop: element.scrollTop });
            if (!islandPointInRect(point, layout.window)) { leaveWindow(); return; }
            pointerInside.current = true;
            // 常态透明边距不承载本次切态语义：透明判定会与既有 180ms 进入、拖动释放重判冲突，
            // 因此常态不评估透明判定、不写门闩，只保留既有 180ms 进入；逻辑与单测保留以便恢复。
            if (modeRef.current === "collapsed") {
                marginState.current.since = undefined;
                if (hoverKind.current === "margin") stopHoverTimer();
                scheduleHover(true);
                return;
            }
            if (modeRef.current === "expanded" && (!layoutSettled.current || inputGeometryKey.current !== geometryKey.current)) return;
            const cards = Array.from(element.querySelectorAll<HTMLElement>(".island-card")).map((card) => {
                const box = card.getBoundingClientRect(), style = getComputedStyle(card);
                return { rect: { left: box.left, top: box.top, width: box.width, height: box.height }, radii: [style.borderTopLeftRadius, style.borderTopRightRadius, style.borderBottomRightRadius, style.borderBottomLeftRadius].map(Number.parseFloat) };
            });
            if (!cards.length) return;
            const contour = islandHitContour(cards, layout.window, preferences?.islandEdge ?? "", preferences?.islandForm ?? "capsule", attachedCorner.current ?? 0);
            const region = islandHitRegion(modeRef.current, point, layout, contour);
            const input = { mode: modeRef.current, region, now: performance.now(), busy: false, outlineDistance: islandContourDistance(point, contour), coreDistance: islandCardDistance(point, cards[0]!) };
            lastMarginInput.current = input;
            const next = islandMarginStep(marginState.current, input);
            marginState.current = next.state;
            if (next.collapse) { if (!dropActiveRef.current) changeMode("collapsed", modeRef.current === "hover"); return; }
            // hover 的透明边距与展开的额外带共用同一条 120ms 连续停留计时：进带开始计时，回内容盒取消。
            const delayRegion: "transparent" | "extra-band" = modeRef.current === "hover" ? "transparent" : "extra-band";
            if (region === delayRegion && marginState.current.since !== undefined) {
                if (hoverKind.current === "margin" && hoverTimer.current !== undefined) return;
                stopHoverTimer(); hoverKind.current = "margin";
                const version = hoverVersion.current;
                const deadline = marginState.current.since + ISLAND_MARGIN_DELAY_MS;
                hoverTimer.current = setTimeout(() => {
                    hoverTimer.current = undefined; hoverKind.current = undefined;
                    if (version !== hoverVersion.current || !mounted.current || !lastMarginInput.current || marginState.current.since === undefined) return;
                    // setTimeout已达到deadline；受控断言时performance.now可暂停，仍不依赖rAF推进。
                    const result = islandMarginStep(marginState.current, { ...lastMarginInput.current, mode: modeRef.current, now: Math.max(performance.now(), deadline), busy: interactionBusy() || !inputReady.current });
                    marginState.current = result.state;
                    if (result.collapse && !dropActiveRef.current) changeMode("collapsed", modeRef.current === "hover");
                }, Math.max(0, Math.ceil(deadline - performance.now())));
                return;
            }
            if (hoverKind.current === "margin") stopHoverTimer(); // 离开延时区：撤销未到点的退回计时。
            if (modeRef.current === "hover") scheduleHover(region === "content");
        },
        leave: () => leaveWindow(),
        up: (event) => endScrollbarDrag(event),
    };
    useEffect(() => {
        const move = (event: globalThis.PointerEvent) => inputHandlers.current.move(event);
        const leave = (event: globalThis.PointerEvent) => { if (!event.relatedTarget) inputHandlers.current.leave(event); };
        const up = (event: globalThis.PointerEvent) => inputHandlers.current.up(event);
        document.addEventListener("pointermove", move, true); document.addEventListener("pointerout", leave, true);
        document.addEventListener("pointerup", up, true); document.addEventListener("pointercancel", up, true);
        return () => {
            document.removeEventListener("pointermove", move, true); document.removeEventListener("pointerout", leave, true);
            document.removeEventListener("pointerup", up, true); document.removeEventListener("pointercancel", up, true);
            stopHoverTimer(); endScrollbarDrag(); cancelCollapseWait();
        };
    }, []);
    const handleScrollbarDown = (event: PointerEvent<HTMLDivElement>) => {
        const value = scrollbarFrame.current, element = root.current;
        if (event.button !== 0 || modeRef.current !== "expanded" || !value?.visible || !value.track || !value.thumb || !element) return;
        event.stopPropagation(); event.preventDefault(); stopHoverTimer(); marginState.current.since = undefined;
        if (!islandPointInRect({ x: event.clientX, y: event.clientY }, value.thumb) && value.travel > 0) element.scrollTop = (event.clientY - value.track.top - value.thumb.height / 2) * value.maxScroll / value.travel;
        scrollbarDrag.current = { id: event.pointerId, y: event.clientY, top: element.scrollTop, range: value.maxScroll, travel: value.travel, capture: event.currentTarget };
        event.currentTarget.dataset.dragging = "true";
        event.currentTarget.setPointerCapture(event.pointerId);
    };
    /** 取消尚未到点的长按拖动，释放或移动后不得再接管鼠标。 */
    const cancelDragHold = () => {
        clearTimeout(dragHold.current);
        dragHold.current = undefined;
    };
    const startNativeDrag = () => {
        if (dragging.current) return;
        cancelDragHold();
        stopHoverTimer();
        gesture.current = undefined;
        suppressClick.current = true;
        dragging.current = true;
        inputReady.current = false; marginState.current = { latch: "open" }; endScrollbarDrag();
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
                stopHoverTimer();
                dragging.current = false;
                setDragActive(false);
                if (!mounted.current) return;
                pointerInside.current = releasedInside;
                geometryKey.current = "";
                skipMotionOnce.current = true;
                collapsedBox.current = undefined; // 拖动已改变沿边位置，旧胶囊范围不再参与 hover 并集。
                changeMode(islandAfterDrag(releasedInside));
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
        if (intent !== "window" || dragging.current) return;
        marginState.current.since = undefined;
        stopHoverTimer();
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
            stopHoverTimer();
            changeMode("expanded");
        } else scheduleHover(pointerInside.current);
    };
    const handleReorder = async (from: string, to: string) => {
        if (sorting.current) return;
        const order = sortable.map((instance) => instance.componentId);
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
    /** 关闭一条消息：先本地移除避免闪烁，后端走与原生关闭按钮相同的离场路径。 */
    const handleDismissMessage = (id: number) => {
        content.forgetMessage(id);
        heldMessages.current = heldMessages.current.filter((message) => message.id !== id);
        setMessages((current) => current.filter((message) => message.id !== id));
        void dismissIslandMessage(id).catch((error: unknown) => {
            console.warn("岛消息关闭失败", error);
            const revision = messagesRevision.current; // 补偿读取前记录代次。
            void getIslandMessages()
                .then((next) => {
                    if (messagesRevision.current === revision) setMessages(next); // 期间有新广播时以广播为准。
                })
                .catch(() => undefined); // 消息仍在队列时恢复展示，避免本地与后端分叉。
        });
    };
    return (
        <main
            ref={root}
            id="island-scroll-content"
            className="island"
            style={{ maxBlockSize: work?.height, "--island-anchor-size": `${ISLAND_ANCHOR_SIZE}px`, "--island-capsule-line-max": `${islandCapsuleLineLimit(vertical, media?.playback === "playing")}px`, "--island-anchor-gap": `${ISLAND_ANCHOR_GAP}px`, "--island-anchor-grow": `${ISLAND_ANCHOR_GROW_MS}ms`, "--island-anchor-shrink": `${ISLAND_ANCHOR_SHRINK_MS}ms` } as CSSProperties}
            data-island-page="true"
            data-dragging={dragActive ? "true" : undefined}
            data-form={preferences?.islandForm ?? "capsule"}
            data-edge={edge || "top"}
            data-state={mode}
            data-animations={animationsEnabled ? "true" : undefined}
            data-vertical={vertical}
            data-scrollbar={edge === "right" ? "start" : "end"}
            data-fillets={preferences?.islandForm === "notch" && mode !== "expanded" ? "true" : undefined}
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
            tabIndex={mode === "expanded" ? 0 : -1}
            aria-label="浮岛，单击展开，退出键收起"
            onKeyDown={(event) => {
                if (event.defaultPrevented || interactionBusy() || dropActiveRef.current) return;
                stopHoverTimer();
                if (event.key === "Escape") changeMode("collapsed");
                else if (event.key === "Enter" && event.target === event.currentTarget) changeMode("expanded");
            }}
        >
            <div ref={palette} className="fence-workspace island__palette" hidden aria-hidden="true" />
            {/* 常驻播报节点：消息卡随内容重建，直接挂 aria-live 时常见读屏器不播报。 */}
            <p className="island__announcer" role="status" aria-live="polite">
                {liveText}
            </p>
            <div ref={surface} className="island__surface" aria-hidden="true" />
            {mode === "expanded" && dropAnchor ? (
                <section className="island-card" data-island-no-drag="true">
                    <div className="island-card__content island-anchor-expanded">
                        <span className="island-anchor" data-anchor="drop"><IslandAnchorVisual anchor={dropAnchor} special="release" motion={false} /></span>
                        <span className="island-anchor__text">{dropFace?.primary}</span>
                    </div>
                </section>
            ) : null}
            {mode !== "expanded" ? (
                <section className="island-card island-card--anchors" aria-label={mode === "collapsed" ? "浮岛概览" : "浮岛摘要"} style={mode === "hover" && capsuleInline.current > 0 ? { minInlineSize: `${capsuleInline.current}px` } : undefined}>
                    <div className="island-card__content">
                        {mode === "collapsed" ? (
                            <div className="island-anchor-row" data-layout="capsule">
                                {face.kind === "clock" ? <div className="island-anchor-item"><span className="island-anchor" data-anchor="clock"><IslandAnchorVisual special="clock" motion={false} /></span><span className="island-anchor__text island-anchor__clock">{face.text}</span></div> : <>
                                    {face.visible.map((anchor) => {
                                        const active = capsuleActive?.id === anchor.id ? anchor : undefined;
                                        const musicAnchor = isMusic(anchor);
                                        const lyric = musicAnchor && musicView.hasLine && anchor.unit?.primary === musicView.line;
                                        const inserting = contentMotion && previousIds.length > 0 && !previousIds.includes(anchor.id) && !drop;
                                        return <div className="island-anchor-item" key={anchor.id}>
                                            <span className="island-anchor" data-anchor={anchor.id} data-inserting={inserting || undefined}><IslandAnchorVisual anchor={anchor} specs={statusSpecs} special={anchor === dropAnchor ? "release" : undefined} motion={contentMotion} entering={inserting}>{weatherIcon(anchor)}</IslandAnchorVisual></span>
                                            <IslandRelayContent className="island-anchor-label" anchor={active} motion={contentMotion && !drop} vertical={vertical} music={musicAnchor} growDelay={Math.max(relayFrom && relayFrom !== active?.id ? ISLAND_ANCHOR_SHRINK_MS : 0, inserting ? ISLAND_ANCHOR_GROW_MS : 0)}>
                                                {active?.unit ? lyric ? <IslandCapsuleLyric music={music} position={position} motion={contentMotion} vertical={vertical} /> : <span className="island-anchor__text">{islandShortText(active.unit.primary)}{islandAnchorValue(active.unit.secondary ?? "") ? <span className="island-anchor__value">{active.unit.secondary}</span> : null}</span> : null}
                                            </IslandRelayContent>
                                        </div>;
                                    })}
                                    {face.overflow ? <div className="island-anchor-item" key="overflow">
                                        <span className="island-anchor" data-anchor="overflow"><IslandAnchorVisual special={`＋${face.overflow > 99 ? "99+" : face.overflow}`} motion={false} /></span>
                                        {capsuleActive?.unit && !face.visible.some((anchor) => anchor.id === capsuleActive.id) ? <span className="island-anchor__text">{islandShortText(capsuleActive.unit.primary)}</span> : null}
                                    </div> : null}
                                </>}
                            </div>
                        ) : <>
                            <div ref={hoverList} className="island-anchor-row" data-layout="hover" role="group" aria-label="浮岛锚点" onKeyDown={handleHoverKey}>
                                {anchors.length ? anchors.map((anchor) => <button
                                    type="button" className="island-anchor" data-anchor={anchor.id} key={anchor.id} tabIndex={anchor.id === selected?.id ? 0 : -1}
                                    aria-label={`${anchor.source} ${islandStatusSpec(anchor.unit?.status ?? "", statusSpecs)?.word ?? ""}`.trim()} aria-pressed={anchor.id === selected?.id}
                                    onPointerEnter={(event) => { if (event.pointerType !== "touch" && !drop) content.setSelected(anchor.id); }}
                                    onClick={() => { if (!drop) content.setSelected(anchor.id); }}
                                >{anchor === dropAnchor ? <IslandAnchorVisual anchor={anchor} special="release" motion={contentMotion} /> : <IslandFunctionIcon kind={islandAnchorKind(anchor)} />}</button>) : <div className="island-anchor-item"><span className="island-anchor" data-anchor="clock"><IslandAnchorVisual special="clock" motion={false} /></span><span className="island-anchor__text island-anchor__clock">{islandClockLabel(content.clockNow)}</span></div>}
                            </div>
                            {hoverText.expandable ? <div className="island-anchor-detail" ref={hoverContentRef} tabIndex={-1} role="group" aria-label={`${selected?.source}内容`}>
                                <span className="island-anchor island-anchor--detail" aria-hidden="true"><IslandAnchorVisual anchor={selected} specs={statusSpecs} motion={false} /></span>
                                <div className="island-anchor-detail__lines">
                                    <span className="island-anchor__text">{islandShortText(hoverText.lines[0] ?? "")}</span>
                                    {selectedMessage ? <span className="island-anchor__text">{selectedMessage.source} · {islandMessageTime(selectedMessage.receivedAtMs, content.clockNow)}</span> :
                                        hoverSecondary ? <span className="island-anchor__text">{islandShortText(hoverSecondary)}</span> : null}
                                </div>
                                {isMusic(selected) && hoverText.controls.length ? <IslandMusicControls media={media} allowed={hoverText.controls} /> : null}
                                {selectedMessage && hoverText.controls.includes("dismiss") ? <button type="button" className="island-message__close" data-button="icon" aria-label={`关闭${selectedMessage.source}消息`} onClick={() => handleDismissMessage(selectedMessage.id)}><span aria-hidden="true" dangerouslySetInnerHTML={{ __html: closeIcon }} /></button> : null}
                            </div> : null}
                        </>}
                    </div>
                </section>
            ) : visibleMessages.length || instances.length || baseAnchors.length ? (
                <>
                    {visibleMessages.length ? (
                        <section className="island-card" aria-label="消息" data-island-message="true">
                            <div className="island-card__content"><IslandMessagePanel messages={visibleMessages} onDismiss={handleDismissMessage} /></div>
                        </section>
                    ) : null}
                    {instances.map((instance) => {
                    const manifest = snapshot!.runtime.manifests.find((item) => item.id === instance.componentId)!;
                    const registration = snapshot!.runtime.instances.find((item) => item.instanceId === instance.instanceId);
                    return (
                        <section
                            className="island-card"
                            key={instance.instanceId}
                            aria-label={manifest.name}
                            tabIndex={0}
                            aria-description="聚焦卡片后使用上下方向键调整顺序"
                            onKeyDown={(event) => {
                                if (event.target !== event.currentTarget) return;
                                const index = sortable.indexOf(instance);
                                const target = sortable[index + (event.key === "ArrowUp" ? -1 : event.key === "ArrowDown" ? 1 : 0)];
                                const to =
                                    target &&
                                    islandSortTarget(
                                        mode,
                                        sortable.map((item) => item.componentId),
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
                            {(
                                <button
                                    type="button"
                                    className="island-card__toggle"
                                    data-island-no-drag="true"
                                    title={cardCollapsed(instance.componentId) ? "展开卡片" : "收起卡片"}
                                    aria-label={cardCollapsed(instance.componentId) ? "展开卡片" : "收起卡片"}
                                    aria-expanded={!cardCollapsed(instance.componentId)}
                                    onPointerDown={(event) => event.stopPropagation()}
                                    onClick={(event) => {
                                        event.stopPropagation();
                                        handleToggleCard(instance.componentId);
                                    }}
                                >
                                    <span
                                        aria-hidden="true"
                                        dangerouslySetInnerHTML={{ __html: cardCollapsed(instance.componentId) ? islandToggleExpandIcon : islandToggleCollapseIcon }}
                                    />
                                </button>
                            )}
                            <div className="island-card__content">
                                {instance.componentId === "official.music-control" ? (
                                    <IslandMusic mode={cardCollapsed(instance.componentId) ? "hover" : "expanded"} side={edge === "left" || edge === "right"} motion={animationsEnabled} music={music} position={position} />
                                ) : instance.componentId === "official.collect-box" ? (
                                    <IslandCollectBox
                                        mode={mode}
                                        collapsed={cardCollapsed(instance.componentId)}
                                        collapsedCount={preferences?.islandCollectCollapsedCount ?? 3}
                                        onDragEnd={handleCollectDragEnd}
                                    />
                                ) : instance.componentId === "official.weather" ? (
                                    <IslandWeather mode={cardCollapsed(instance.componentId) ? "hover" : "expanded"} weather={weather} />
                                ) : registration?.status === "failed" ? (
                                    <p role="status">组件加载失败</p>
                                ) : registration?.entry === "plugin:slot" ? (
                                    <div data-island-plugin="true">
                                        <PluginSlot
                                            manifest={manifest}
                                            instanceId={instance.instanceId}
                                            tileType={pluginTileType(instance.state)}
                                            collapsed={false}
                                        />
                                    </div>
                                ) : (
                                    <p role="status">{manifest.name} · 组件入口不可用</p>
                                )}
                            </div>
                        </section>
                    );
                    })}
                    {baseAnchors.filter((anchor) => anchor.layer !== "resident" && (anchor.layer !== "message" || !visibleMessages.some((message) => message.source === anchor.source && message.receivedAtMs === anchor.updatedAtMs))).map((anchor) => {
                        const message = anchor.layer === "message" ? content.getMessage(anchor) : undefined;
                        return <section className="island-card" key={anchor.id} aria-label={anchor.source}>
                            <div className="island-card__content island-anchor-expanded">
                                {message ? <IslandMessageCard message={message} onDismiss={handleDismissMessage} /> : <>
                                    <span className="island-anchor" data-anchor={anchor.id}><IslandAnchorVisual anchor={anchor} specs={statusSpecs} motion={contentMotion} /></span>
                                    {anchor.unit?.primary ?? islandHoverContent(anchor).lines[0]}
                                </>}
                            </div>
                        </section>;
                    })}
                </>
            ) : (
                <section className="island-card">
                    <div className="island-card__content">
                        <p role="status">{failure ? "岛加载失败" : snapshot ? "暂无岛卡" : "正在加载"}</p>
                    </div>
                </section>
            )}
            {mode === "expanded" && messageFailure ? (
                <section className="island-card" data-island-no-drag="true" aria-label="消息通道">
                    <div className="island-card__content">
                        <p role="status">消息通道不可用</p>
                    </div>
                </section>
            ) : null}
            {import.meta.env.DEV && mode === "expanded" && Array.from({ length: debugCount }, (_, index) => (
                <section className="island-card" data-island-debug="true" data-island-no-drag="true" key={`island-debug-${index}`} aria-label={`开发临时卡片${index + 1}`}>
                    <div className="island-card__content"><div style={{ inlineSize: 192, minBlockSize: 112 }}>
                        <strong>开发临时卡片 {index + 1}</strong><p>仅用于溢出与多卡调试，不保存。</p>
                    </div></div>
                </section>
            ))}
            <div ref={scrollbar} className="island__scrollbar" data-island-no-drag="true" role="scrollbar" aria-label="展开内容滚动条" aria-controls="island-scroll-content" aria-orientation="vertical" tabIndex={-1}
                onPointerDown={handleScrollbarDown}
                onLostPointerCapture={() => endScrollbarDrag()}
                onTransitionEnd={(event) => {
                    if (event.target !== event.currentTarget || event.propertyName !== "opacity") return;
                    const visible = event.currentTarget.dataset.visible === "true", opacity = Number.parseFloat(getComputedStyle(event.currentTarget).opacity);
                    if (visible ? opacity > 0.999 : opacity < 0.001) event.currentTarget.dataset.phase = visible ? "visible" : "hidden";
                }}
                onKeyDown={(event) => {
                    const element = root.current;
                    if (!element || scrollbar.current?.dataset.visible !== "true") return;
                    const next = event.key === "ArrowDown" ? element.scrollTop + 40 : event.key === "ArrowUp" ? element.scrollTop - 40 : event.key === "PageDown" ? element.scrollTop + element.clientHeight : event.key === "PageUp" ? element.scrollTop - element.clientHeight : event.key === "Home" ? 0 : event.key === "End" ? element.scrollHeight : undefined;
                    if (next === undefined) return;
                    event.preventDefault(); event.stopPropagation(); element.scrollTop = next;
                }}>
                <div ref={scrollbarThumb} className="island__scrollbar-thumb" />
            </div>
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
