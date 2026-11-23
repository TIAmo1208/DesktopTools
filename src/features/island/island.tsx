import { Fragment, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type CSSProperties, type PointerEvent, type ReactNode } from "react";
import { getCollectItems, listenCollectChanges } from "../../bridge/collect";
import { getDesktopHostSnapshot } from "../../bridge/desktopHost";
import { beginIslandDrag, dismissIslandMessage, getIslandMessages, islandCursorInside, listenIslandDropActive, listenIslandMessages, listenIslandPluginMessages, listenIslandSnapshots, saveIslandCardCollapsed, saveIslandCardOrder, setIslandState } from "../../bridge/island";
import { executeMusicAction, seekMusicPosition } from "../../bridge/music";
import { listenSpectrum } from "../../bridge/audio";
import type { IslandDropFact as IslandNativeDropFact, IslandMessage, IslandRect, IslandSnapshot, IslandStateRequest } from "../../contracts/island";
import { ISLAND_MESSAGE_CARD_ID, islandMessageCardEnabled, islandMessageCardInitialization, type ApplicationPreferences } from "../../contracts/settings";
import { savePreferences } from "../../bridge/settings";
import { openPluginUri } from "../../bridge/components";
import pauseIcon from "../../../resources/music-pause.svg?raw";
import closeIcon from "../../../resources/editor-x.svg?raw";
import successIcon from "../../../resources/editor-check.svg?raw";
import infoIcon from "../../../resources/info.svg?raw";
import errorIcon from "../../../resources/message-error.svg?raw";
import warningIcon from "../../../resources/message-warning.svg?raw";
import musicNoteIcon from "../../../resources/island-music-note.svg?raw";
import bellIcon from "../../../resources/bell.svg?raw";
import pluginFunctionIcon from "../../../resources/settings-plugin.svg?raw";
import { PluginSlot, pluginTileType, subscribePluginHost } from "../components/PluginSlot";
import { useMusicState, musicProgressRange, positionAt } from "../music/musicSource";
import { usePreferences } from "../../app/preferences/PreferencesProvider";
import { ScrollingText } from "../../shared/ScrollingText";
import { IslandCollectBox } from "./IslandCollectBox";
import { IslandWeather, useIslandWeather } from "./IslandWeather";
import { islandWeatherOutlook } from "./islandWeatherOutlook";
import { WeatherGlyph } from "../../shared/WeatherGlyph";
import {
    islandAttachedCorner,
    islandCardMaskPath,
    islandCornerRadius,
    islandFilletPatches,
    islandSeamMask,
    islandGeometry,
    moveIslandVisibleCard,
    visibleIslandRect,
    islandWindowKey,
    islandContentOffset,
    islandExpandedColumn,
    ISLAND_EXPANDED_COLUMN_WIDTH,
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
    islandExpandedBlank, beginIslandExpandedGrace, islandExpandedGraceActive, islandLeaveSuppressed, islandHitContour, islandCardDistance, islandContourDistance, islandHitRegion, islandMarginStep, islandScrollGeometry, islandPointInRect, islandDebugCardCount, islandCollapseSettle,
    ISLAND_MARGIN_DELAY_MS, ISLAND_SCROLLBAR_FADE_MS, islandExpandedDwellStep,
    type IslandExpandedDwell, type IslandMarginState, islandExpandedDwellBlocked,
} from "./islandInteraction";
import { ISLAND_MOTION_MS, ISLAND_FRAME_MS, ISLAND_EXPAND_STAGGER_MS, ISLAND_EXPAND_MOTION_MS, ISLAND_HOVER_DETAIL_EXIT_MS, ISLAND_HOVER_DETAIL_ENTER_MS, islandAttachedCornerAt, beginIslandMask, holdsIslandMask, releaseIslandMask, unmaskIslandTransition, islandHoverTravel, islandUnionFrame, beginIslandNativeSwap, canReleaseIslandMask, finishIslandNativeSwap, islandGeometryCommitDecision, islandViewportChanged, awaitIslandViewport, ISLAND_VIEWPORT_WAIT_MS, islandGeometryKey, islandFailedGeometryGate, islandWakingFrame, islandFrameOrTimeout, islandRenderGate, revokeIslandReservation, ISLAND_NO_FRAME_TIMEOUT_MS, deferIslandMaskRelease, takeIslandMaskReleases, canResyncIslandGeometry, islandRetrySubmit, resetIslandGeometryBaseline, type IslandMaskLease, type IslandNativeSwap, type IslandGeometrySeat, type IslandSubmitReservation, type IslandDeferredRelease } from "./islandMotion";
import { ISLAND_MESSAGE_LIMIT, ISLAND_MESSAGE_TIMING, ISLAND_NOTIFICATION_ANCHOR_ID, islandCapsuleGlyph, islandDismissNotification, islandHoldMessages, islandKeepNotifications, islandLocalNotificationArrival, islandMergeNotifications, islandMessageArrival, islandMessageFocusOrder, islandMessagePanelView, islandMessageTime, islandNotificationAnchor, islandNotificationContentDeadline, islandNotificationCountLabel, islandNotificationDeadline, islandNotificationTone, islandNotificationVisual, type IslandCapsuleGlyph, type IslandLocalNotification, type IslandMessageTiming, type IslandNotification } from "./islandMessages";
import { createIslandContentResizeObserver, islandHoverBody, islandHoverInlineSize, ISLAND_NOTICE_LAYER, islandCollectArrival, islandDisplayAnchors, islandMusicAnchorVisible, islandWeatherNotice, ISLAND_ANCHOR_GAP, ISLAND_ANCHOR_GROW_MS, ISLAND_ANCHOR_SHRINK_MS, ISLAND_ANCHOR_SIZE, ISLAND_ANCHOR_SLOTS, ISLAND_MESSAGE_HANDOFF_MS, ISLAND_STATUS_SPECS, islandAnchorGlyph, islandAnchorId, observeIslandPluginLayout, islandAnchorValue, islandArrive, islandCapsuleFace, islandClockLabel, islandDropFace, islandHoverEnter, islandHoverInitialSelection, islandHoverSelect, islandHoverSessionWidth, islandHoverStep, islandLayerRank, islandStatusProgress, islandStatusSpec, islandAnchorKind, type IslandAnchor, type IslandAnchorArrival, type IslandDropTarget, type IslandFunctionKind, type IslandStatus, type IslandStatusSpec } from "./islandContent";
import { ISLAND_LYRIC_OFFSETS, islandCapsuleLineLimit, islandLyricView, islandMusicTickDelay, islandPlaybackStatus, islandScrollAnimationName, islandScrollDistance, islandScrollKey, islandScrollPlan, islandScrollPlayState } from "./islandLyrics";
import { islandCollectTitle, readIslandCollectCache, writeIslandCollectCache } from "./islandCollect";
import { islandPluginAnchorStatus, islandPluginIconSource, islandPluginMark, islandPluginMessageDeliverable, islandPluginMountSites, islandPluginPrimary, islandPluginSummaries, islandPluginSummaryCallbacks, type IslandPluginSummaryState } from "./islandPluginCard";
import { islandCardCollapseDefault, islandMessageSlotIndex as messageSlotIndexFor } from "./islandMessageCard";
import { animateIslandFold, beginIslandFold, beginIslandFoldSave, endIslandFoldSave, islandCardDropTarget, islandFoldTargetHeight, isIslandFoldSaveCurrent, settleIslandFold, type IslandFoldState } from "./islandCardFold";
import { islandNavigationLabel, islandNavigationUri, openIslandNavigation } from "./islandNavigation";
import { readPluginApi } from "../../bridge/pluginScripts";
import { pluginAssetUrl } from "../../bridge/pluginAssets";
import { publishMessage } from "../notifications/messageNotification";
import { ISLAND_FENCE_TOKENS } from "./islandTokens";
import islandToggleCollapseIcon from "../../../resources/fence-toggle-collapse.svg?raw";
import islandToggleExpandIcon from "../../../resources/fence-toggle-expand.svg?raw";
import "../../styles/fenceTokens.css"; // 探针读取的围栏内置值，来自桌面工作区与浮岛共用的那一份。
import "./island.css";
const ISLAND_ANCHOR_EASING = "ease-out"; // 胶囊与 hover 接力共用的退缓。
// 原生物理状态必须跨 effect 共享：A 在途时 B 不能用自己私有的计数提前揭开遮蔽。
const nativeSwap: IslandNativeSwap = { pending: 0, appliedToken: null, appliedGeometry: undefined };
const deferredMaskReleases = new Map<number, IslandDeferredRelease>(); // pending 未清空时挂起的解除：连 keepOwner/alreadySynced 一起保留，旧回包不重画。
const maskFailTimers = new Map<number, ReturnType<typeof setTimeout>>(); // native 失败时 800ms 兜底解除。

/** 临时诊断：记录展开/收起时显示层坐标、尺寸与视口；设置 window.__ISLAND_TRACE__ 后写入 __ISLAND_TRACE_LOG__。 */
/** gated 诊断是否开启；构造 detail 之前先问一次，关闭时不为诊断做任何布局读取。 */
const islandTracing = (): boolean => typeof window !== "undefined" && (window as unknown as { __ISLAND_TRACE__?: boolean }).__ISLAND_TRACE__ === true;

/** 惰性诊断：trace 关闭时完全不构造 detail（不读 getBoundingClientRect）。 */
const islandTraceLazy = (stage: string, detail: () => Record<string, unknown>) => {
    if (!islandTracing()) return;
    islandTrace(stage, detail());
};

const islandTrace = (stage: string, detail: Record<string, unknown>) => {
    if (typeof window === "undefined") return;
    const target = window as unknown as { __ISLAND_TRACE__?: boolean; __ISLAND_TRACE_LOG__?: Array<Record<string, unknown>> };
    if (!islandTracing()) return;
    const root = document.querySelector<HTMLElement>(".island");
    const rect = root?.getBoundingClientRect();
    target.__ISLAND_TRACE_LOG__ ??= [];
    target.__ISLAND_TRACE_LOG__.push({
        t: Math.round(performance.now()),
        stage,
        inner: [window.innerWidth, window.innerHeight],
        doc: [document.documentElement.clientWidth, document.documentElement.clientHeight],
        root: rect ? { x: Math.round(rect.x), y: Math.round(rect.y), w: Math.round(rect.width), h: Math.round(rect.height), margin: root?.style.margin, visibility: root?.style.visibility } : null,
        ...detail,
    });
    if (target.__ISLAND_TRACE_LOG__.length > 400) target.__ISLAND_TRACE_LOG__.splice(0, 200);
};
if (typeof window !== "undefined") window.addEventListener("resize", () => islandTrace("viewport-resize", {}));
let islandNativeFailClosed = false; // 原生补偿失败后的 fail-closed 标记，下一次状态变化重新提交。
const PAINTED_STYLE_PROPERTIES = [
    "position",
    "left",
    "top",
    "inline-size",
    "block-size",
    "max-inline-size",
    "min-inline-size",
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
    // 拖动进度：拖动或按键期间只改本地草稿，提交后等提交之后的新样本回来再交回真实投影。
    const [seekDraft, setSeekDraft] = useState<number>();
    const [seekPending, setSeekPending] = useState<{ id: number; value: number; from: number; fromTimeline: ReturnType<typeof useMusicState>["timeline"] }>();
    const seekRequest = useRef(0); // 请求代次：旧请求的失败、超时都不清掉更新的目标。
    const seekTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
    // 优先用系统给出的可跳转区间；没有区间时退回时间轴时长，再退回歌词兜底量程，保证进度条可交互。
    const seekRange = media?.seekMinMs !== undefined && media?.seekMaxMs !== undefined && media.seekMaxMs > media.seekMinMs
        ? { min: media.seekMinMs, max: media.seekMaxMs } : undefined;
    const seekMin = seekRange ? seekRange.min : 0;
    const seekMax = seekRange ? seekRange.max : music.timeline && music.timeline.durationMs > 0 ? music.timeline.durationMs : progress.totalMs;
    // 预先按能力位筛选：不可控的播放源直接给只读进度，不提供悬停与手型，也不发跳转。
    // 仍保留回读兜底：个别播放器声明可跳转却忽略位置，发现后本会话改回只读。
    const [seekUnsupported, setSeekUnsupported] = useState<string>();
    const seekable = Boolean(media) && media?.canSeek === true && seekMax > seekMin && seekUnsupported !== media?.sessionId;
    const displayPosition = seekDraft ?? seekPending?.value ?? position;
    const progressPercent = progress.max > 0 ? Math.min(100, Math.max(0, (progress.value / progress.max) * 100)) : 0; // 只读进度的填充比例。
    const latestPosition = useRef(position); // 回读判定读取最新投影，不依赖闭包里的旧值。
    latestPosition.current = position;
    const seekProbe = useRef<number | undefined>(undefined); // 正在等待回读的提交代次。
    const seekProbeTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
    useEffect(() => () => {
        clearTimeout(seekTimer.current);
        clearTimeout(seekProbeTimer.current);
    }, []);
    useEffect(() => {
        // 换歌、换会话时丢弃草稿与待确认值，不把旧请求的进度留在新曲目上。
        clearTimeout(seekTimer.current);
        clearTimeout(seekProbeTimer.current);
        seekProbe.current = undefined;
        setSeekDraft(undefined);
        setSeekPending(undefined);
    }, [music.timeline?.sessionId, music.timeline?.title]);
    useEffect(() => {
        const pending = seekPending;
        if (!pending || !music.timeline) return;
        if (music.timeline === pending.fromTimeline) return; // 提交前就存在的样本不能当作确认。
        const moved = Math.abs(music.timeline.positionMs - pending.from) > 50; // 播放器位置真的动过。
        if (moved && Math.abs(music.timeline.positionMs - pending.value) < 1200) setSeekPending((current) => (current?.id === pending.id ? undefined : current)); // 系统已落到目标附近。
    }, [music.timeline, seekPending]);
    /** 松手或键盘提交时只发一次跳转；结果未确认时按超时回到系统投影。 */
    const commitSeek = (value: number) => {
        const session = media?.sessionId;
        setSeekDraft(undefined);
        if (!session || !seekable) return;
        const id = ++seekRequest.current;
        setSeekPending({ id, value, from: music.timeline?.positionMs ?? position, fromTimeline: music.timeline });
        clearTimeout(seekTimer.current);
        seekTimer.current = setTimeout(() => setSeekPending((current) => (current?.id === id ? undefined : current)), 3000);
        const origin = music.timeline?.positionMs ?? position;
        void seekMusicPosition(session, Math.round(value)).catch((error: unknown) => {
            console.warn("岛音乐进度跳转失败", error);
            if (seekRequest.current !== id) return; // 旧请求的失败不影响更新的目标。
            setSeekPending((current) => (current?.id === id ? undefined : current));
            setFailure(true);
        });
        clearTimeout(seekProbeTimer.current);
        seekProbe.current = id;
        seekProbeTimer.current = setTimeout(() => {
            if (seekProbe.current !== id || seekRequest.current !== id || seekUnsupported === session) return;
            // 大幅拖动却没有落到目标附近，说明播放器接受了调用但忽略了位置，本会话改回只读并说明一次。
            if (Math.abs(value - origin) <= 2_500 || Math.abs(latestPosition.current - value) <= 2_500) return;
            setSeekUnsupported(session);
            setSeekPending((current) => (current?.id === id ? undefined : current));
            publishMessage({ type: "warning", content: "当前播放器不接受进度跳转，已改回只读进度", source: "音乐" });
        }, 1_800);
    };
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
                        {seekable ? (
                            // 播放器声明可跳转时用滑块：轨道与只读进度同一套视觉，只有悬停/按下才有可拖反馈。
                            <input
                                className="island-music__seek"
                                type="range"
                                aria-label="播放进度"
                                min={Math.round(seekMin)}
                                max={Math.round(seekMax)}
                                step={1000}
                                value={Math.round(Math.min(Math.max(seekMin, displayPosition), seekMax))}
                                onChange={(event) => setSeekDraft(Number(event.currentTarget.value))}
                                onPointerUp={(event) => commitSeek(Number(event.currentTarget.value))}
                                onKeyUp={(event) => {
                                    if (event.key.startsWith("Arrow") || ["Home", "End", "PageUp", "PageDown"].includes(event.key)) commitSeek(Number(event.currentTarget.value));
                                }}
                            />
                        ) : (
                            // 播放源不可控时不给 hover：同一套轨道与滑块外观，但静态、无手型、不响应悬停。
                            <div
                                className="island-music__progress"
                                role="progressbar"
                                aria-label="播放进度"
                                aria-valuemin={0}
                                aria-valuemax={Math.round(progress.max)}
                                aria-valuenow={Math.round(progress.value)}
                                title="此播放器不支持拖动进度"
                            >
                                <span className="island-music__progress-fill" style={{ inlineSize: `${progressPercent}%` }} />
                                <span className="island-music__progress-knob" style={{ insetInlineStart: `clamp(0px, calc(${progressPercent}% - 5px), calc(100% - 10px))` }} />
                            </div>
                        )}
                        <div className="island-music__stamps">
                            <time>{formatTime(displayPosition)}</time>
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
type IslandMessageDismiss = (id: number, keyboardOrigin?: HTMLButtonElement) => void;

const IslandMessageCard = ({ message, onDismiss }: { message: IslandMessage; onDismiss: IslandMessageDismiss }) => (
    <div className="island-message" data-kind={message.type} data-loading={message.loading ? "true" : undefined}>
        <span className="island-message__icon" data-loading={message.loading ? "true" : undefined} aria-hidden="true">
            {message.loading ? <span className="island-message__loading" />
                : message.iconUrl ? <img className="island-message__logo" src={message.iconUrl} alt="" draggable={false} />
                : <span dangerouslySetInnerHTML={{ __html: ISLAND_MESSAGE_ICONS[message.type] }} />}
        </span>
        <div className="island-message__info">
            <p className="island-message__body">{message.content}</p>
            <p className="island-message__meta">
                <span className="island-message__source">{message.source}</span><span aria-hidden="true">·</span>
                <time>{islandMessageTime(message.receivedAtMs, Date.now())}</time>
            </p>
        </div>
        <button type="button" data-button="icon" className="island-message__close" data-island-message-id={message.id} aria-label={`关闭${message.source}消息`}
            onClick={(event) => {
                // 原生键盘激活的 click 没有点击次数；只有当前聚焦的按钮才请求续焦。
                const node = event.currentTarget;
                onDismiss(message.id, event.detail === 0 && document.activeElement === node ? node : undefined);
            }}>
            <span aria-hidden="true" dangerouslySetInnerHTML={{ __html: closeIcon }} />
        </button>
    </div>
);

/** 展开态消息面板：收起态只显示最新一条；开关走宿主统一收起记忆。 */
const IslandMessagePanel = ({
    messages,
    onDismiss,
    collapsed,
}: {
    messages: IslandMessage[];
    onDismiss: IslandMessageDismiss;
    collapsed: boolean; // 宿主统一收起态：消息卡缺省收起，用户显式操作后记忆。
}) => {
    const visible = collapsed ? messages.slice(0, 1) : messages; // 收起时只保留最新一条。
    return (
        <div className="island-message-panel">
            <div className="island-message-panel__head">
                <span>消息</span>
                {messages.length > 1 ? <span className="island-message-panel__count">{messages.length} 条</span> : null}
            </div>
            <div className="island-message-panel__list">
                {visible.map((message) => (
                    // 折叠只有一个槽位：后台换入更新消息时保留原按钮与焦点。
                    <IslandMessageCard key={collapsed ? "latest" : message.id} message={message} onDismiss={onDismiss} />
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
    // 换句的上下交接与盒内横向滚动是两套独立动画：交接只动外层，滚动仍只动当前行。
    const previousLine = useRef({ index: view.index, line: view.line });
    const [leaving, setLeaving] = useState<{ index: number; text: string }>();
    const incomingLayer = useRef<HTMLSpanElement>(null);
    const outgoingLayer = useRef<HTMLSpanElement>(null);
    useLayoutEffect(() => {
        const previous = previousLine.current;
        previousLine.current = { index: view.index, line: view.line };
        if (previous.index === view.index || previous.line === view.line) return;
        if (!motion) {
            setLeaving(undefined);
            return;
        }
        setLeaving({ index: previous.index, text: previous.line });
    }, [view.index, view.line, motion]);
    useEffect(() => {
        if (!leaving) return;
        const entering = incomingLayer.current?.animate(
            [{ transform: "translateY(100%)", opacity: 0 }, { transform: "none", opacity: 1 }],
            { duration: ISLAND_MESSAGE_HANDOFF_MS, easing: ISLAND_ANCHOR_EASING, fill: "both" },
        );
        const exiting = outgoingLayer.current?.animate(
            [{ transform: "none", opacity: 1 }, { transform: "translateY(-100%)", opacity: 0 }],
            { duration: ISLAND_MESSAGE_HANDOFF_MS, easing: ISLAND_ANCHOR_EASING, fill: "both" },
        );
        const timer = setTimeout(() => setLeaving(undefined), ISLAND_MESSAGE_HANDOFF_MS + 20);
        return () => {
            clearTimeout(timer);
            entering?.cancel();
            exiting?.cancel();
        };
    }, [leaving]);
    return <p ref={lineRef} className="island-music__line" data-motion={motion ? "on" : "off"} style={lineStyle}>
        <span ref={incomingLayer} className="island-music__handoff"><span className="island-music__measure"><span key={scrollKey} className="island-music__current" style={scrollStyle}>{view.line}</span></span></span>
        {leaving ? <span ref={outgoingLayer} className="island-music__outgoing" aria-hidden="true">{leaving.text}</span> : null}
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

/** 三个控制位置固定；媒体不支持的操作沿用原生禁用反馈。 */
const IslandMusicControls = ({ media, onFailure }: { media: ReturnType<typeof useMusicState>["snapshot"]; onFailure?: () => void }) => {
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
        {islandMusicCommands(media).map(([action, label, enabled, path]) => (
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
    music: musicNoteIcon,
    message: bellIcon,
    plugin: pluginFunctionIcon,
    event: infoIcon,
};

/** 功能图标只作身份标识，不表达状态；状态仍由内容行左侧的状态符号表达。 */
const IslandFunctionIcon = ({ kind }: { kind: IslandFunctionKind }) => <span className="island-anchor__icon" data-function={kind} aria-hidden="true" dangerouslySetInnerHTML={{ __html: ISLAND_FUNCTION_ICONS[kind] }} />;

/** 状态符号：按既有状态表绘制锁、暂停、失败、断连与等待，不带身份 Logo。 */
const IslandStatusSymbol = ({ status }: { status: import("./islandContent").IslandStatus }) => {
    const glyph = islandAnchorGlyph(status);
    const symbol = glyph.kind === "glyph" ? glyph.symbol : undefined;
    const asset = symbol && symbol in ISLAND_ANCHOR_ICONS ? ISLAND_ANCHOR_ICONS[symbol as keyof typeof ISLAND_ANCHOR_ICONS] : undefined;
    const path = symbol && symbol in ISLAND_ANCHOR_PATHS ? ISLAND_ANCHOR_PATHS[symbol as keyof typeof ISLAND_ANCHOR_PATHS] : undefined;
    return asset ? <span className="island-anchor__icon" aria-hidden="true" dangerouslySetInnerHTML={{ __html: asset }} />
        : path ? <svg viewBox="0 0 20 20" aria-hidden="true"><path d={path} strokeWidth="1.75" /></svg>
            : <span className="island-anchor__dot" aria-hidden="true" />;
};

/** 插件身份位：正常连接用身份图标，业务异常与陈旧一律用状态符号。 */
const PluginIdentityMark = ({ status, iconPath, size = 20 }: { status: import("./islandContent").IslandStatus; iconPath?: string; size?: 16 | 20 }) =>
    islandPluginMark(status) === "logo" ? <PluginIdentityIcon iconPath={iconPath} size={size} /> : <IslandStatusSymbol status={status} />;

/** 插件身份图标：清单给了 iconPath 就用 <img> 渲染官方原色；缺失或加载失败回落通用插件图标。 */
const PluginIdentityIcon = ({ iconPath, size = 20 }: { iconPath?: string; size?: 16 | 20 }) => {
    const [failedUrl, setFailedUrl] = useState<string>();
    const source = islandPluginIconSource(iconPath, failedUrl, pluginAssetUrl);
    if (source.kind === "fallback") return <IslandFunctionIcon kind="plugin" />; // 不出现破图或空位。
    return (
        <img
            className="island-plugin-identity"
            data-size={size}
            src={source.url}
            alt=""
            aria-hidden="true"
            draggable={false}
            onError={() => setFailedUrl(source.url)}
        />
    );
};

/** 媒体图只用于胶囊与详情。加载失败时回到同一功能音符。 */
const IslandMediaInk = ({ artwork, hover = false }: { artwork?: string; hover?: boolean }) => {
    const [failed, setFailed] = useState<string>();
    return <span className={hover ? "island-anchor__media island-music-hover__artwork" : "island-anchor__media"} aria-hidden="true">
        {artwork && artwork !== failed ? <img src={artwork} alt="" draggable={false} onError={() => setFailed(artwork)} /> : <IslandFunctionIcon kind="music" />}
    </span>;
};

/** 仅有实际进度时画环；集成的中央图标不遮挡路径。 */
const IslandProgressInk = ({ sweep, progress = 0, motion }: { sweep?: boolean; progress?: number; motion: boolean }) => (
    <svg className="island-anchor__progress" viewBox="0 0 20 20">
        <circle className="island-anchor__track" cx="10" cy="10" r="8.5" strokeWidth="3" />
        {sweep ? <path className="island-anchor__sweep" data-motion={motion} d={islandArcPath(0.25)} strokeWidth="3" /> :
            progress >= 1 ? <circle cx="10" cy="10" r="8.5" strokeWidth="3" /> : progress > 0 ? <path d={islandArcPath(progress)} strokeWidth="3" /> : null}
    </svg>
);

/** 胶囊消息锚点：普通铃铛、类型图标、确定进度的进度环、无进度用统一 loading 图形。 */
const IslandMessageGlyph = ({ glyph, motion }: { glyph?: IslandCapsuleGlyph; motion: boolean }) => {
    if (glyph?.kind === "logo") return <img className="island-anchor__logo" src={glyph.url} alt="" draggable={false} />;
    if (glyph?.kind === "type") return <span className="island-anchor__icon" aria-hidden="true" dangerouslySetInnerHTML={{ __html: ISLAND_MESSAGE_ICONS[glyph.type] }} />;
    if (glyph?.kind === "progress") return <IslandProgressInk progress={glyph.progress} motion={motion} />;
    if (glyph?.kind === "loading") return <span className="island-anchor__loading" data-motion={motion ? "on" : "off"} aria-hidden="true" />;
    return <IslandFunctionIcon kind="message" />;
};

/** hover 通知左侧只画条目真实图形；测宽样本仅保留有效图标的尺寸。 */
export const IslandNotificationRowIcon = ({ notice, measuring = false, outlookCode }: { notice: IslandNotification; measuring?: boolean; outlookCode?: number }) => {
    const visual = outlookCode === undefined ? islandNotificationVisual(notice) : { kind: "weather" as const, code: String(outlookCode) };
    if (notice.iconUrl && outlookCode === undefined) {
        // 系统通知条目用应用 Logo，其余条目仍按类型图标；测宽样本保留 Logo 尺寸。
        return <span className="island-notification-row__icon" data-visual="app" aria-hidden="true">
            {measuring ? null : <img className="island-notification-row__logo" src={notice.iconUrl} alt="" draggable={false} />}
        </span>;
    }
    if (!visual) return null;
    return <span className="island-notification-row__icon" data-visual={visual.kind} aria-hidden="true">
        {measuring ? null : visual.kind === "progress" ? <IslandProgressInk progress={visual.progress} motion={false} />
            : visual.kind === "weather" ? <WeatherGlyph code={visual.code} />
            : visual.kind === "type" ? <span dangerouslySetInnerHTML={{ __html: ISLAND_MESSAGE_ICONS[visual.type] }} /> : null}
    </span>;
};

/** 展开卡统一卡头：拖卡句柄与右上最多两个动作位，消息/官方/插件卡共用。 */
const IslandCardToolbar = ({
    cardId,
    name,
    collapsed,
    pending,
    action,
    sorting,
    onToggle,
    onHandlePointerDown,
    onHandlePointerMove,
    onHandlePointerUp,
    onHandleCancel,
    onHandleKeyDown,
}: {
    cardId: string;
    name: string;
    collapsed: boolean;
    pending: boolean;
    action?: { label: string; disabled: boolean; onClick: () => void };
    sorting: boolean;
    onToggle: (card: HTMLElement | null) => void;
    onHandlePointerDown: (event: React.PointerEvent<HTMLButtonElement>) => void;
    onHandlePointerMove: (event: React.PointerEvent<HTMLButtonElement>) => void;
    onHandlePointerUp: (event: React.PointerEvent<HTMLButtonElement>) => void;
    onHandleCancel: (event: React.PointerEvent<HTMLButtonElement>) => void;
    onHandleKeyDown: (event: React.KeyboardEvent<HTMLButtonElement>) => void;
}) => (
    <div className="island-card__toolbar" data-island-no-drag="true">
        <button
            type="button"
            className="island-card__handle"
            data-island-card-handle={cardId}
            aria-label={`调整${name}卡顺序`}
            aria-description="按住可拖动排序，或使用上下方向键"
            disabled={pending} // 展开/收缩动画在途时禁用拖动，避免叠加。
            data-sorting={sorting ? "true" : undefined}
            onPointerDown={onHandlePointerDown}
            onPointerMove={onHandlePointerMove}
            onPointerUp={onHandlePointerUp}
            onPointerCancel={onHandleCancel}
            onLostPointerCapture={onHandleCancel}
            onKeyDown={onHandleKeyDown}
        >
            <span aria-hidden="true" />
        </button>
        <div className="island-card__actions">
            {action ? (
                <button type="button" className="island-card__action" disabled={action.disabled || pending} data-island-no-drag="true" onClick={action.onClick}>
                    {action.label}
                </button>
            ) : null}
            <button
                type="button"
                className="island-card__toggle"
                data-island-no-drag="true"
                title={collapsed ? "展开卡片" : "收起卡片"}
                aria-label={collapsed ? "展开卡片" : "收起卡片"}
                aria-expanded={!collapsed}
                aria-busy={pending || undefined}
                onPointerDown={(event) => event.stopPropagation()}
                onClick={(event) => {
                    event.stopPropagation();
                    onToggle(event.currentTarget.closest<HTMLElement>(".island-card"));
                }}
            >
                <span aria-hidden="true" dangerouslySetInnerHTML={{ __html: collapsed ? islandToggleExpandIcon : islandToggleCollapseIcon }} />
            </button>
        </div>
    </div>
);

/** hover 通知列表：只有当前指针/焦点所在条目滚动，其余保持单行省略。 */
const IslandNotificationList = ({
    rows,
    measuring,
    hoverNow,
    animationsEnabled,
    onDismiss,
    onOpen,
}: {
    rows: readonly IslandNotification[];
    measuring: boolean;
    hoverNow: number;
    animationsEnabled: boolean;
    onDismiss?: (notice: IslandNotification) => void;
    onOpen: (componentId: string | undefined, uri: string | undefined) => void;
}) => {
    const [activeKey, setActiveKey] = useState<string>();
    const active = activeKey && rows.some((notice) => notice.key === activeKey) ? activeKey : rows[0]?.key; // 换行后回落到第一条。
    return <div className="island-notification-list">
        {rows.map((notice) => {
            const text = `${notice.source} · ${notice.content}`;
            const outlook = notice.weatherOutlookFacts && islandWeatherOutlook(hoverNow, notice.weatherOutlookFacts);
            const navigable = islandNavigationUri(notice.uri) !== undefined && notice.componentId !== undefined; // 只对声明了去向的条目启用。
            return <div className="island-notification-row" key={notice.key} onPointerEnter={() => setActiveKey(notice.key)} onFocus={() => setActiveKey(notice.key)}>
                <IslandNotificationRowIcon notice={notice} measuring={measuring} outlookCode={outlook?.code} />
                <div
                    className="island-notification-row__copy"
                    data-island-no-drag={navigable ? "true" : undefined}
                    role={navigable ? "button" : undefined}
                    tabIndex={navigable ? 0 : undefined}
                    aria-label={navigable ? islandNavigationLabel(text, notice.uri!) : undefined}
                    onClick={navigable ? (event) => { event.stopPropagation(); onOpen(notice.componentId, notice.uri); } : undefined}
                    onKeyDown={navigable ? (event) => {
                        if (event.key !== "Enter" && event.key !== " ") return;
                        event.preventDefault();
                        event.stopPropagation();
                        if (event.repeat) return; // 已阻断默认与冒泡，长按只触发一次打开。
                        onOpen(notice.componentId, notice.uri);
                    } : undefined}
                >
                    <span className="island-anchor__text"><ScrollingText text={text} title={text} enabled={!measuring && animationsEnabled && notice.key === active} /></span>
                    {outlook ? <span className="island-notification-row__outlook island-anchor__text" title={outlook.text}>{outlook.text}</span> : null}
                </div>
                <time> · {islandMessageTime(notice.receivedAtMs, hoverNow)}</time>
                <button type="button" className="island-message__close" data-button="icon" aria-label={`关闭消息：${islandShortText(text)}`} onClick={onDismiss ? () => onDismiss(notice) : undefined}><span aria-hidden="true" dangerouslySetInnerHTML={{ __html: closeIcon }} /></button>
            </div>;
        })}
    </div>;
};

/** 在 20×20 安全盒内绘制状态或功能墨层，媒体裁切例外占满整枚圆。 */
const IslandAnchorVisual = ({ anchor, special, motion, artwork, entering = false, specs, messageTone, messageGlyph }: { anchor?: IslandAnchor; special?: string; motion: boolean; artwork?: string; entering?: boolean; specs?: readonly IslandStatusSpec[]; messageTone?: ReturnType<typeof islandNotificationTone>; messageGlyph?: IslandCapsuleGlyph }) => {
    const ink = useRef<HTMLSpanElement>(null);
    const insertOnMount = useRef(entering);
    useLayoutEffect(() => {
        const animate = insertOnMount.current && motion;
        insertOnMount.current = false;
        if (!animate) return;
        const insertion = ink.current?.animate([{ transform: "translateY(6px)", opacity: 0 }, { transform: "none", opacity: 1 }], { duration: ISLAND_ANCHOR_GROW_MS, easing: ISLAND_ANCHOR_EASING });
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
    const kind = anchor && islandAnchorKind(anchor);
    const custom = kind === "music" || kind === "message";
    return <span ref={ink} className="island-anchor__ink" aria-hidden="true" data-status={status} data-symbol={custom ? undefined : symbol} data-glyph={custom ? undefined : glyph?.kind} data-kind={custom ? kind : undefined} data-message-tone={messageTone}>
        {kind === "music" ? <IslandMediaInk artwork={artwork} /> : kind === "message" ? <IslandMessageGlyph glyph={messageGlyph} motion={motion} /> : kind === "plugin" && status !== undefined && islandPluginMark(status) === "logo" ? <PluginIdentityIcon iconPath={anchor?.iconPath} /> : asset ? <span className="island-anchor__icon" dangerouslySetInnerHTML={{ __html: asset }} /> : path ? (
            <svg viewBox="0 0 20 20"><path d={path} strokeWidth="1.75" /></svg>
        ) : glyph?.kind === "ring" || glyph?.kind === "sweep" ? (
            <IslandProgressInk sweep={glyph.kind === "sweep"} progress={glyph.kind === "ring" ? glyph.arc : undefined} motion={motion} />
        ) : special ? <span className="island-anchor__number">{special}</span> : <span className="island-anchor__dot" />}
    </span>;
};

/** 隐藏或桥暂停其一仍成立，就不能把补读当作恢复。 */
const islandNotificationsPaused = () => document.hidden || window.__DESKTOP_TOOLS_NATIVE__?.isRefreshPaused?.() === true;

/** 收集原生与浮岛本地通知，只为合并锚点维护条目；不向原生发送新的 Toast。 */
const useIslandNotifications = (messages: IslandMessage[], mode: IslandMode, clockNow: number, timing: IslandMessageTiming) => {
    const [rows, setRows] = useState<readonly IslandNotification[]>([]);
    const observed = useRef(new Map<number, IslandMessage>());
    const previousMode = useRef(mode);
    const pending = useRef<readonly IslandNotification[]>([]); // 隐藏期间只收集，恢复时一次并入，不逐条补播。
    const sequence = useRef(0);
    const [visibility, setVisibility] = useState(0);
    useEffect(() => {
        const refresh = () => setVisibility((value) => value + 1);
        document.addEventListener("visibilitychange", refresh);
        window.addEventListener("desktop-refresh-changed", refresh);
        return () => {
            document.removeEventListener("visibilitychange", refresh);
            window.removeEventListener("desktop-refresh-changed", refresh);
        };
    }, []);
    useLayoutEffect(() => {
        const now = Date.now();
        const incoming = [...messages].sort((left, right) => left.id - right.id).filter((message) => {
            const prior = observed.current.get(message.id);
            return !prior || prior.content !== message.content || prior.loading !== message.loading || prior.type !== message.type || prior.source !== message.source;
        }).map(islandMessageArrival);
        observed.current = new Map(messages.map((message) => [message.id, message]));
        const hidden = islandNotificationsPaused();
        if (hidden) pending.current = islandMergeNotifications(pending.current, incoming);
        const recovered = hidden ? [] : islandKeepNotifications(pending.current, now, "collapsed", timing);
        if (!hidden) pending.current = [];
        const read = mode === "collapsed" && previousMode.current === "expanded";
        previousMode.current = mode;
        setRows((current) => {
            // 离开展开态清除已读的原生普通通知；本地通知不进入展开消息卡。
            const unread = read ? current.filter((notice) => notice.nativeId === undefined || islandNotificationDeadline(notice, timing) === undefined) : current;
            const merged = !hidden && (recovered.length || incoming.length)
                ? islandMergeNotifications(unread, [...recovered].reverse().concat(incoming))
                : unread;
            return islandKeepNotifications(merged, now, mode, timing);
        });
    }, [messages, mode, clockNow, visibility, timing]);
    const push = useCallback((fact: IslandLocalNotification, receivedAtMs = Date.now()) => {
        const notice = islandLocalNotificationArrival(++sequence.current, fact, receivedAtMs);
        if (islandNotificationsPaused()) pending.current = islandMergeNotifications(pending.current, [notice]);
        else setRows((current) => islandMergeNotifications(current, [notice]));
    }, []);
    const dismiss = useCallback((key: string) => {
        pending.current = islandDismissNotification(pending.current, key);
        setRows((current) => islandDismissNotification(current, key));
    }, []);
    const allowRetry = (id: number) => observed.current.delete(id);
    // 正文期结束后锚点只保留图标：投影按当前时刻裁剪内容单元，胶囊正文自然让位。
    const anchor = useMemo(() => islandNotificationAnchor(rows, { nowMs: clockNow, timing }), [rows, clockNow, timing]);
    return { rows, anchor, push, dismiss, allowRetry };
};

/** 维持锚点插入顺序；胶囊、hover、选择与播报都消费同一份显示投影。 */
const useIslandContent = (residents: readonly IslandAnchorArrival[], notification: IslandAnchor | undefined, mode: IslandMode, specs: readonly IslandStatusSpec[], preferences: ApplicationPreferences | undefined, capsuleFront: string | undefined) => {
    const [anchors, setAnchors] = useState<IslandAnchor[]>([]);
    const [foregroundId, setForegroundId] = useState<string>();
    const [selected, setSelected] = useState<string>();
    const previousMode = useRef(mode); // 只在从已呈现的胶囊切入 hover 时重设初选。
    useLayoutEffect(() => {
        const keep = new Set(residents.map((item) => islandAnchorId(item.layer, item.source)));
        setAnchors((current) => {
            let next = current.flatMap((anchor) => anchor.id === ISLAND_NOTIFICATION_ANCHOR_ID ? notification ? [notification] : [] : keep.has(anchor.id) ? [anchor] : []);
            for (const resident of residents) {
                const prior = next.find((anchor) => anchor.id === islandAnchorId(resident.layer, resident.source));
                if (!prior || prior.unit?.primary !== resident.unit?.primary || prior.unit?.status !== resident.unit?.status || prior.unit?.secondary !== resident.unit?.secondary || prior.kind !== resident.kind) next = islandArrive(next, resident);
            }
            if (notification && !next.some((anchor) => anchor.id === notification.id)) next.push(notification);
            return current.length === next.length && next.every((anchor, index) => anchor === current[index]) ? current : next;
        });
    }, [residents, notification]);
    const live = anchors.filter((anchor) => islandStatusSpec(anchor.unit?.status ?? "", specs)?.disposition !== "vanish");
    const visible = islandDisplayAnchors(live, preferences);
    const eligible = visible.filter((anchor) => anchor.unit);
    let winner = eligible.find((anchor) => anchor.id === foregroundId);
    for (const anchor of eligible) if (!winner || islandLayerRank(anchor.layer) < islandLayerRank(winner.layer)) winner = anchor;
    const preferred = selected ?? winner?.id;
    const selectedId = mode === "hover" && previousMode.current === "collapsed"
        ? islandHoverInitialSelection(visible, capsuleFront, preferred)
        : islandHoverSelect(visible, preferred);
    useLayoutEffect(() => {
        previousMode.current = mode;
        setForegroundId(winner?.id);
        if (mode === "hover") setSelected(selectedId);
    }, [mode, winner?.id, selectedId]);
    return { live, visible, active: winner, selected: selectedId, setSelected };
};

/** hover 一次只挂一份详情。离场仍占原位，结束后只换一次自然布局，再播放入场。 */
const IslandHoverRelay = ({ anchors, selected, motion, renderDetail }: { anchors: readonly IslandAnchor[]; selected?: IslandAnchor; motion: boolean; renderDetail: (anchor: IslandAnchor | undefined) => ReactNode }) => {
    const layer = useRef<HTMLDivElement>(null);
    const [shownId, setShownId] = useState(selected?.id);
    const targetId = selected?.id;
    const travel = motion && shownId !== targetId ? islandHoverTravel(anchors.map((anchor) => anchor.id), shownId, targetId) : undefined;
    const displayed = travel ? anchors.find((anchor) => anchor.id === shownId) : selected;
    const enter = useRef<{ id: string | undefined; x: number } | undefined>(undefined); // 只有已完成的离场可排入下一段。
    const exitX = travel?.exitX;
    const enterX = travel?.enterX;
    useLayoutEffect(() => {
        const node = layer.current;
        if (exitX !== undefined && enterX !== undefined && node?.firstElementChild) {
            const exit = node.animate([{ transform: "none", opacity: 1 }, { transform: `translateX(${exitX}px)`, opacity: 0 }],
                { duration: ISLAND_HOVER_DETAIL_EXIT_MS, easing: ISLAND_ANCHOR_EASING, fill: "forwards" });
            let active = true;
            void exit.finished.then(() => {
                if (!active) return;
                enter.current = { id: targetId, x: enterX };
                setShownId(targetId);
            }).catch(() => undefined);
            return () => { active = false; exit.cancel(); };
        }
        if (shownId !== targetId) setShownId(targetId);
        const arrival = enter.current;
        enter.current = undefined;
        if (motion && arrival && arrival.id === targetId && node?.firstElementChild) {
            const entry = node.animate([{ transform: `translateX(${arrival.x}px)`, opacity: 0 }, { transform: "none", opacity: 1 }],
                { duration: ISLAND_HOVER_DETAIL_ENTER_MS, easing: ISLAND_ANCHOR_EASING, fill: "backwards" });
            return () => entry.cancel();
        }
    }, [motion, shownId, targetId, exitX, enterX]);
    return <div ref={layer} className="island-hover-relay" inert={Boolean(travel) || undefined} aria-hidden={travel ? true : undefined}>
        {renderDetail(displayed)}
    </div>;
};

/** 同来源换文在一行内滚动；换源只伸缩该锚点文字，原生盒随实测收放。 */
/** 胶囊内容接力：换锚点收放尺寸，消息换文沿歌词同一阅读轴一次性交接。 */
const IslandRelayContent = ({ anchor, motion, vertical, className, growDelay, music = false, handoff, children }: {
    anchor?: IslandAnchor; motion: boolean; vertical: boolean; className: string; growDelay: number; music?: boolean; handoff?: string; children?: ReactNode;
}) => {
    const slot = useRef<HTMLDivElement>(null);
    const incoming = useRef<HTMLDivElement>(null);
    const outgoing = useRef<HTMLDivElement>(null);
    const previous = useRef(growDelay ? undefined : anchor);
    const previousHandoff = useRef(handoff); // 上一条真正显示的正文身份；同一条目的进度刷新不重播交接。
    const interrupted = useRef(false);
    const [shown, setShown] = useState({ anchor, children });
    const [leaving, setLeaving] = useState<ReactNode>();
    useLayoutEffect(() => {
        const old = previous.current;
        const oldHandoff = previousHandoff.current;
        previous.current = anchor;
        previousHandoff.current = handoff;
        const node = slot.current;
        const animations: Animation[] = [];
        let active = true;
        const animate = (element: Element | null, frames: Keyframe[], duration: number, delay = 0, easing = ISLAND_ANCHOR_EASING) => {
            if (!element) return undefined;
            const value = element.animate(frames, { easing, fill: "both", duration, delay });
            animations.push(value);
            return value;
        };
        // 竖排 writing-mode 下 inline 轴就是物理高度：两种方向都按 inlineSize 收缩，避免缩错轴。
        const axis = "inlineSize";
        const size = () => `${vertical ? node?.offsetHeight : node?.offsetWidth}px`;
        const wasInterrupted = interrupted.current;
        interrupted.current = false;
        const statusChanged = old?.layer !== "message" && old?.id === anchor?.id && old?.unit?.status !== anchor?.unit?.status;
        // 正文期结束只留锚点：与换锚点同样先收尺寸，避免胶囊宽度瞬跳。
        const collapsedToAnchor = old?.unit !== undefined && anchor?.unit === undefined;
        // 消息换文只认条目身份与结果修订，正文相同或进度刷新都不重播交接。
        const handoffChanged = handoff !== undefined && oldHandoff !== undefined && handoff !== oldHandoff;
        // 连续换文时旧动画已被取消：仍按新目标重播一次交接，避免第二条消息丢掉动画。
        const replayHandoff = wasInterrupted && handoffChanged;
        if (!motion || !node || (wasInterrupted && !replayHandoff) || statusChanged) {
            setShown({ anchor, children });
            setLeaving(undefined);
        } else if ((old?.id !== anchor?.id || collapsedToAnchor) && old?.unit) {
            const shrink = animate(node, [{ [axis]: size(), opacity: 1 }, { [axis]: "0px", opacity: 0 }], ISLAND_ANCHOR_SHRINK_MS);
            void shrink?.finished.then(() => { if (active) setShown({ anchor, children }); shrink.cancel(); }).catch(() => undefined);
        } else {
            setShown({ anchor, children });
            if (old?.id !== anchor?.id && anchor?.unit) {
                const grow = animate(node, [{ [axis]: "0px", opacity: 0 }, { [axis]: size(), opacity: 1 }], ISLAND_ANCHOR_GROW_MS, growDelay);
                void grow?.finished.then(() => grow.cancel()).catch(() => undefined);
            } else if (handoffChanged && anchor?.unit) {
                // 与文档约定的换文节奏一致：上一行立刻上移 100% 渐出，下一行从下方 100% 渐入，各 300ms 重叠。
                setLeaving(shown.children);
                const roll = animate(incoming.current, [{ transform: "translateY(100%)", opacity: 0 }, { transform: "none", opacity: 1 }], ISLAND_MESSAGE_HANDOFF_MS, 0);
                const leave = animate(outgoing.current, [{ transform: "none", opacity: 1 }, { transform: "translateY(-100%)", opacity: 0 }], ISLAND_MESSAGE_HANDOFF_MS, 0);
                void roll?.finished.then(() => { if (active) setLeaving(undefined); roll.cancel(); leave?.cancel(); }).catch(() => undefined);
            }
        }
        return () => {
            active = false;
            interrupted.current = animations.some((item) => item.playState === "running");
            animations.forEach((item) => item.cancel());
        };
    }, [anchor?.id, anchor?.unit?.status, handoff, motion, vertical]);
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
/** 断言页可注入内容与投放事实；生产入口由原生投影和实际功能事件驱动。 */
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
    const contentMotion = animationsEnabled && !prefersReducedMotion; // 既有胶囊滚动与通用状态图形口径；hover 接力和顶部过渡统一使用解析后的开关。
    const [snapshot, setSnapshot] = useState<IslandSnapshot>();
    const [failure, setFailure] = useState<string>();
    const [messages, setMessages] = useState<IslandMessage[]>([]); // 与原生 Toast 同一队列的只读投影。
    const [messageFailure, setMessageFailure] = useState<string>(); // 消息通道失败只降级，不阻塞岛卡。
    const heldMessages = useRef<IslandMessage[]>([]); // 阅读期暂存，回到常态清空。
    const messagesRevision = useRef(0); // 新快照或本地关闭都使尚未返回的旧补读失效。
    const [mode, setMode] = useState<IslandMode>("collapsed");
    const [collapsedCards, setCollapsedCards] = useState<Record<string, boolean>>({});
    const [foldStates, setFoldStates] = useState<Record<string, IslandFoldState>>({}); // 每卡展开/收缩事务，供 pending 样式与序号淘汰。
    const cardHeights = useRef<Record<string, number>>({}); // 最近一次有效自然高，测量失败时回落。 // 卡内收起态；默认值来自偏好与收集箱默认收起。
    const collapsedCardsRef = useRef(collapsedCards); // 连续点击读取最新本地状态，不等偏好回执。
    const transientFolds = useRef<Record<string, boolean>>({}); // 不记忆的卡片只在本会话保留选择。
    const foldSave = useRef({ queue: Promise.resolve(), revision: 0, inflight: 0 }); // 在途保存数；旧回执结束时也能正确归零。
    const foldOrigins = useRef(new Map<HTMLElement, { id: string; height: number; seq: number }>()); // 点击前的可见高度与事务序号，反向点击从途中接续。
    const foldSeqs = useRef<Record<string, number>>({}); // 每卡最新事务序号，连续点击只保留最新目标。
    const cardSorting = useRef(false); // 卡头长按拖动期间禁止透明边距回退与根点击。
    const transitionMasked = useRef(false); // 视口切换提交期间整层遮蔽，避免旧坐标画进新视口。
    const cardSortGesture = useRef<{ id: string; x: number; y: number; pointerId: number; timer: ReturnType<typeof setTimeout>; button: HTMLButtonElement } | undefined>(undefined);
    const [sortingCard, setSortingCard] = useState<string>(); // 正在拖动的卡。
    const [dropCard, setDropCard] = useState<string>(); // 当前落点预览。
    const foldAnimations = useRef(new Map<HTMLElement, Animation>());
    const [nativeDrop, setNativeDrop] = useState<IslandDropFact>(); // 原生投放进入后显示「松开加入收集箱」。
    const dropActiveRef = useRef(false); // 退回判定与离开路径读取最新值，不依赖渲染。
    const modeRef = useRef(mode); // 延时回调读取最新状态，不能覆盖键盘或点击后的展开。
    modeRef.current = mode;
    const [work, setWork] = useState<IslandRect>();
    const [monitorId, setMonitorId] = useState<string>(); // 主屏身份供浮岛天气登记活动订阅。
    const root = useRef<HTMLElement>(null); // 只测量岛自身。
    const messageFocus = useRef<{ origin: HTMLButtonElement; candidates: number[] } | undefined>(undefined); // 最后一条关闭也可能卸载消息卡，续焦由岛根完成。
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
    const seatBaseline = useRef<IslandGeometrySeat | undefined>(undefined); // 最近一次几何提交的贴边身份基准，随提交前移而非随 effect 固化。
    const currentGeometry = useRef<IslandStateRequest | undefined>(undefined); // 最近实际完成的原生区域。
    const hadPaintedFrame = useRef(false); // 是否已经绘制过画面：首次挂载为 false，换边清几何后仍为 true。
    const maskPainted = useRef(false); // 本次「可见→隐藏」是否已经画到屏幕上：同一遮蔽周期内不重复等，避免重测互相推迟。
    const pendingSubmit = useRef<IslandSubmitReservation | undefined>(undefined); // 本任务预留但尚未首发的几何提交。
    const failedGeometry = useRef(""); // 上一条重试耗尽、从未落地的目标几何键：同一几何不再重发，等外部变化或几何改变。
    const collapsedBox = useRef<IslandStateRequest | undefined>(undefined); // 最近胶囊态区域，hover 窗口与它取并集避免指针离开。
    const skipMotionOnce = useRef(false); // 拖动后的首次测量跳过动画，避免跨边时先放大到并集再收缩。
    const finishMotion = useRef<() => Promise<void>>(async () => undefined); // 拖动前先结束形变，防止动画帧移动原生拖动中的窗口。
    const gesture = useRef<{ id: number; x: number; y: number } | undefined>(undefined); // hover 单击候选，按位移升级为拖动。
    const tapCandidate = useRef<{ id: number; x: number; y: number; mode: IslandMode } | undefined>(undefined); // 与拖拽解耦的展开点击候选。
    const pointerInside = useRef(false); // 延时回调只读取最新边界结果。
    const lastPointerPoint = useRef<{ x: number; y: number } | undefined>(undefined); // 最近一次 client 坐标，用于 grace 位移判断。
    const lastPointerScreen = useRef<{ x: number; y: number } | undefined>(undefined); // 最近一次屏幕坐标，仅事件坐标缺失时兜底。
    const geometryResizeAt = useRef(0); // 窗口尺寸/几何切换时刻，300ms 内忽略 leave。
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
    const maskLease = useRef<IslandMaskLease>({ seq: 0, owner: undefined, masked: false }); // 换视口遮蔽租约。
    /** 新任务领取遮蔽租约并使旧 token 失效；hide 为假时只占租约，不改可见性。 */
    const beginMask = (hide = true): number => {
        const token = beginIslandMask(maskLease.current);
        transitionMasked.current = hide;
        if (!hide) maskLease.current.masked = false; // 没有真正遮蔽，唯一结算入口按未遮蔽直接释放租约。
        inputReady.current = false;
        const element = root.current;
        if (element && hide) {
            if (element.style.visibility !== "hidden") maskPainted.current = false; // 只有真正从可见变隐藏才需要重新等一次绘制。 // 只有真正从可见变隐藏才需要重新等一次绘制。
            element.style.visibility = "hidden"; // 连同首卡面与所有坐标内容一起遮蔽。
            element.style.pointerEvents = "none";
        }
        return token;
    };
    /** 仅当仍是当前 token 时解除视觉遮蔽，但保留所有权供本任务后续动画绘制。 */
    const unmaskTransition = (token: number, resynced = false): boolean => {
        if (!canReleaseIslandMask(nativeSwap, String(token), resynced)) return false;
        if (!unmaskIslandTransition(maskLease.current, token)) return false;
        transitionMasked.current = false;
        const element = root.current;
        if (element) {
            element.style.removeProperty("visibility");
            element.style.removeProperty("pointer-events");
            islandTrace("unmask", { token });
        }
        inputReady.current = true;
        remeasure.current(true);
        return true;
    };
    /** 幂等释放租约：旧 token 不得解除新任务的遮蔽或清掉新所有权。 */
    const releaseMask = (token: number, resynced = false): boolean => {
        if (!canReleaseIslandMask(nativeSwap, String(token), resynced)) return false; // native 仍在途时保留遮蔽。
        if (!releaseIslandMask(maskLease.current, token)) return false;
        transitionMasked.current = false;
        const element = root.current;
        if (element) {
            element.style.removeProperty("visibility");
            element.style.removeProperty("pointer-events");
            islandTrace("unmask", { token });
        }
        inputReady.current = true;
        remeasure.current(true);
        return true;
    };
    const ownsMask = (token: number): boolean => holdsIslandMask(maskLease.current, token);
    const layoutSettled = useRef(false);
    const inputGeometryKey = useRef(""); // expanded只在最新目标提交后评估，滚动重测不闪断条。
    const marginState = useRef<IslandMarginState>({ latch: "open" });
    const lastMarginInput = useRef<Parameters<typeof islandMarginStep>[1] | undefined>(undefined); // hover 只保留最后一次完整轮廓判定。
    const expandedDwell = useRef<IslandExpandedDwell>("idle"); // 展开只记录是否离开过内容，列内移动不重新计时。
    const pendingEscape = useRef(false); // 遮蔽期间按下的 Esc 不丢：解除遮蔽后立即收起，保留既有交互语义。
    const expandedGrace = useRef<ReturnType<typeof beginIslandExpandedGrace> | undefined>(undefined); // 触发展开的那一次交互先不吃空白回退。
    const expandedDwellTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
    const expandedDwellPoint = useRef<{ x: number; y: number } | undefined>(undefined);
    const expandedDwellClosing = useRef(false); // 区分空白触发的滚动条渐出与 Escape 等主动收起。
    const expandedReentryGate = useRef(false); // 自动收起后须先离开胶囊，再允许旧指针重新进入。
    const expandedReentryGateTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined); // gate 只保留极短时间。
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
    /** 清理展开回退计时；可保留内容侧状态作为下一次边界的起点。 */
    const resetExpandedDwell = (content = false) => {
        clearTimeout(expandedDwellTimer.current);
        expandedDwellTimer.current = undefined;
        expandedDwellPoint.current = undefined;
        expandedDwell.current = content ? "content" : "idle";
    };
    /** 只提交状态本身；滚动条独立渐出的等待由 changeMode 负责，避免形变与渐出重叠。 */
    const commitMode = (next: IslandMode, retainLatch: boolean) => {
        stopHoverTimer();
        clearTimeout(expandedReentryGateTimer.current);
        expandedReentryGateTimer.current = undefined;
        marginState.current = { latch: next === "collapsed" || !retainLatch ? "open" : "blocked" }; // 回胶囊必须清 latch，允许再次 hover。
        lastMarginInput.current = undefined;
        if (next !== "collapsed") expandedReentryGate.current = false;
        else if (expandedDwellClosing.current) {
            expandedReentryGate.current = true;
            expandedReentryGateTimer.current = setTimeout(() => {
                expandedReentryGate.current = false;
                expandedReentryGateTimer.current = undefined;
            }, 600); // 只在刚收起后的极短窗口阻止旧指针立即回 hover。
        } else expandedReentryGate.current = false;
        resetExpandedDwell();
        expandedDwellClosing.current = false;
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
        const value = islandScrollGeometry(frame, targetGeometry.current, preferences?.islandEdge ?? "", { mode: modeRef.current, settled: layoutSettled.current, scrollHeight: element.scrollHeight, clientHeight: element.clientHeight, scrollTop: element.scrollTop }, work);
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
        const acceptMessages = (next: IslandMessage[]) => {
            if (!active) return;
            messagesRevision.current++;
            setMessages(next);
            setMessageFailure(undefined);
        };
        const handleMessageFailure = (error: unknown) => {
            if (active) setMessageFailure(error instanceof Error ? error.message : String(error)); // 消息通道失败只降级到岛卡。
        };
        const messageSubscription = listenIslandMessages(acceptMessages, handleMessageFailure);
        void messageSubscription.catch(handleMessageFailure);
        let wasPaused = islandNotificationsPaused();
        let visibilityRevision = 0;
        const handleNotificationResume = () => {
            const paused = islandNotificationsPaused();
            if (paused === wasPaused) return; // 两种信号可以连续报告同一次恢复，只补读一次。
            wasPaused = paused;
            const visibilityTicket = ++visibilityRevision;
            if (paused) return;
            const messageTicket = messagesRevision.current;
            const current = () => active && visibilityTicket === visibilityRevision && messageTicket === messagesRevision.current;
            void getIslandMessages().then((next) => {
                if (current()) acceptMessages(next);
            }).catch((error: unknown) => {
                if (current()) handleMessageFailure(error);
            });
        };
        // 恢复桥可能吞掉暂停期间的消息广播；只在真实恢复边沿补读，不轮询。
        document.addEventListener("visibilitychange", handleNotificationResume);
        window.addEventListener("desktop-refresh-changed", handleNotificationResume);
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
            document.removeEventListener("visibilitychange", handleNotificationResume);
            window.removeEventListener("desktop-refresh-changed", handleNotificationResume);
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
                    if (modeRef.current === "collapsed") changeMode("hover", false);
                    return;
                }
                void islandCursorInside().then((inside) => {
                    if (!active || dropActiveRef.current) return;
                    pointerInside.current = inside;
                    if (!inside && modeRef.current !== "collapsed") changeMode("collapsed", false);
                }).catch(() => {
                    if (active && !dropActiveRef.current && !pointerInside.current) changeMode("collapsed", false);
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
    const visibleMessages = islandMessagePanelView(messages, heldMessages.current);
    // 消息卡是否真的出现在展开态；空槽不参与键盘焦点，但顺序记录仍保留。
    const messageCardRendered = islandMessageCardEnabled(preferences) && (visibleMessages.length > 0 || Boolean(messageFailure));
    // 排序身份取完整保存顺序里的可见卡，消息槽不会因实例集合重建被删掉。
    const sortableIds = (preferences?.islandCardOrder ?? instances.map((instance) => instance.componentId)).filter((id) => {
        if (id === ISLAND_MESSAGE_CARD_ID) return messageCardRendered;
        return instances.some((instance) => instance.componentId === id);
    });
    useLayoutEffect(() => {
        const request = messageFocus.current;
        messageFocus.current = undefined; // 后续消息广播或失败补读都不再改焦点。
        if (!request || mode !== "expanded" || !root.current) return;
        const focused = document.activeElement;
        if (focused !== request.origin && focused !== document.body) return;
        const panel = root.current.querySelector("[data-island-message='true']");
        for (const id of request.candidates) {
            const next = panel?.querySelector<HTMLButtonElement>(`button[data-island-message-id="${id}"]`);
            if (next) { next.focus(); return; }
        }
        const toggle = panel?.querySelector<HTMLButtonElement>(".island-card__toggle"); // 收起统一在卡头。
        if (toggle) { toggle.focus(); return; }
        // 消息卡前的投放卡无控件；其余卡保持现有 DOM / Tab 次序，不额外滚动通知列表。
        const next = Array.from(root.current.querySelectorAll<HTMLElement>(".island-card:not([data-island-message]) :is(button, a[href], input, select, textarea, [tabindex])")).find(
            (node) => node.tabIndex >= 0 && !node.matches(":disabled") && !node.closest("[hidden], [inert]") && node.getClientRects().length > 0 && getComputedStyle(node).visibility === "visible",
        );
        (next ?? root.current).focus();
    });
    const drop = dropFact ?? nativeDrop;
    dropActiveRef.current = Boolean(drop);
    const dropFace = drop ? islandDropFace(drop === "reject" ? undefined : drop) : undefined;
    const dropAnchor: IslandAnchor | undefined = dropFace ? { id: "drop", source: "拖放", layer: "drop", updatedAtMs: 0, unit: { primary: dropFace.primary, status: "" } } : undefined;
    const { music, position } = useIslandMusic(mode, instances.some((instance) => instance.componentId === "official.music-control"));
    const media = music.snapshot;
    const musicView = islandLyricView(music.lyrics, position, media);
    const musicCapsuleText = musicView.hasLine ? musicView.line : media?.title || islandPlaybackStatus(media); // 胶囊无同步词时不拼接艺人。
    const musicProgress = musicProgressRange(music.timeline, music.lyrics, position);
    const musicPercent = Math.min(100, Math.max(0, Math.floor(musicProgress.value / musicProgress.max * 100)));
    const musicStatus: IslandStatus = !media ? "disconnected" : media.playback === "playing" ? (music.timeline?.durationMs ?? 0) > 0 ? "running" : "indeterminate" : media.playback === "paused" ? "paused" : "pending";
    const weatherInstance = instances.find((instance) => instance.componentId === "official.weather");
    const weather = useIslandWeather(weatherInstance?.instanceId, monitorId);
    const collectEnabled = instances.some((instance) => instance.componentId === "official.collect-box");
    const [clockNow, setClockNow] = useState(Date.now);
    const [pluginSummaries, setPluginSummaries] = useState<Readonly<Record<string, IslandPluginSummaryState>>>({}); // 插件卡摘要与业务状态：缺条目为未调用，text=null 为显式隐藏。
    const [pluginEpoch, setPluginEpoch] = useState(0); // 插件脚本注册/失败后重算 hover 挂载位。
    const [navigationFeedback, setNavigationFeedback] = useState<string>(); // 声明式导航失败的页内提示。
    const navigationRequest = useRef(0); // 只允许最后一次导航写入反馈，迟到结果不覆盖新状态。
    useEffect(() => {
        navigationRequest.current += 1; // 转态即让旧请求失效。
        setNavigationFeedback(undefined);
    }, [mode]);
    const [islandPaused, setIslandPaused] = useState(islandNotificationsPaused); // 锁屏、浮岛隐藏或全屏隐藏时不挂载插件卡。
    const islandPausedRef = useRef(islandPaused); // 事件回调同步读取暂停状态。
    islandPausedRef.current = islandPaused;
    useEffect(() => subscribePluginHost(() => setPluginEpoch((value) => value + 1)), []); // 插件脚本注册或失败后重算挂载位。
    useEffect(() => {
        const sync = () => setIslandPaused(islandNotificationsPaused());
        sync();
        document.addEventListener("visibilitychange", sync);
        window.addEventListener("desktop-refresh-changed", sync);
        return () => {
            document.removeEventListener("visibilitychange", sync);
            window.removeEventListener("desktop-refresh-changed", sync);
        };
    }, []);
    // 只对「已启用且声明 island 的插件卡」建立常驻宿主；官方卡不参与。
    const pluginCards = useMemo(
        () =>
            instances.filter((instance) => {
                const manifest = snapshot?.runtime.manifests.find((item) => item.id === instance.componentId);
                const registration = snapshot?.runtime.instances.find((item) => item.instanceId === instance.instanceId);
                return instance.componentId.startsWith("plugin.") && manifest?.island === true && registration?.status !== "failed";
            }),
        [instances, snapshot],
    );
    const updatePluginSummary = useCallback((componentId: string, text: string | null, status?: string, navigation?: { uri: string }) => {
        setPluginSummaries((current) => islandPluginSummaries(current, componentId, text, status, navigation));
    }, []);
    // 每个 componentId 一条稳定回调：摘要更新只重画文本，不改变插件上下文的函数身份。
    const summaryCallback = useMemo(() => islandPluginSummaryCallbacks(updatePluginSummary), [updatePluginSummary]);
    // 「实际可运行」：浮岛启用且未因锁屏/隐藏暂停；关闭时不挂载也不接收新提醒。
    const islandRunning = !islandPaused && preferences?.islandEnabled === true;
    const islandRunningRef = useRef(islandRunning);
    islandRunningRef.current = islandRunning;
    // 消息通知卡首次进入同一份 islandCardOrder：落盘成功才算初始化，失败下次重试。
    useEffect(() => {
        const initialization = islandMessageCardInitialization(preferences);
        if (!initialization || !preferences) return;
        let active = true;
        void savePreferences({ ...preferences, ...initialization })
            .then((saved) => {
                if (active) onPreferences(saved);
            })
            .catch((error: unknown) => console.warn("消息通知卡初始化保存失败，下次重试", error));
        return () => {
            active = false;
        };
    }, [preferences, onPreferences]);
    // 两段时长是消息通道的固定口径：正文与锚点各从到达时刻起算，互不串接。
    const notifications = useIslandNotifications(messages, mode, clockNow, ISLAND_MESSAGE_TIMING);
    const pushLocalNotification = notifications.push;
    useEffect(() => {
        if (!collectEnabled) return;
        let active = true;
        let generation = 0;
        let previous = readIslandCollectCache();
        const refresh = (fromChange: boolean) => {
            if (window.__DESKTOP_TOOLS_NATIVE__?.isRefreshPaused?.()) return;
            const id = ++generation;
            void getCollectItems().then((items) => {
                if (!active || generation !== id) return;
                // 重命名、置顶、删除和首次读取不产生收录通知。
                const arrival = fromChange ? islandCollectArrival(previous, items, islandCollectTitle) : undefined;
                previous = items;
                writeIslandCollectCache(items);
                if (arrival?.activity === "completed") pushLocalNotification({ source: "collect", content: arrival.title });
            }).catch(() => undefined);
        };
        const sync = () => refresh(false);
        sync();
        const listener = listenCollectChanges((deletedName) => refresh(!deletedName), () => undefined).catch(() => undefined);
        window.addEventListener("desktop-refresh-changed", sync);
        return () => { active = false; window.removeEventListener("desktop-refresh-changed", sync); void listener.then((stop) => stop?.()); };
    }, [collectEnabled, pushLocalNotification]);
    const weatherSummary = weather.summary && weather.unit.status === "pending" ? `${weather.unit.primary}\n${weather.summary.code ?? ""}` : undefined;
    const weatherPrevious = useRef<string | undefined>(undefined);
    useLayoutEffect(() => {
        if (islandWeatherNotice(weatherPrevious.current, weatherSummary)) pushLocalNotification({ source: "weather", content: weather.unit.primary, weatherCode: weather.summary?.code, weatherOutlookFacts: weather.outlookFacts });
        weatherPrevious.current = weatherSummary;
    }, [weatherSummary, weather.unit.primary, weather.summary?.code, weather.outlookFacts, pushLocalNotification]);
    // 插件消息并入既有本地条目管线：同一 key 去重、按到达时间排序，不弹 Toast、不新建队列。
    useEffect(() => {
        let active = true; // 卸载后拒绝迟到消息。
        let stop: (() => void) | undefined;
        void listenIslandPluginMessages(
            (message) => {
                if (!active || !islandPluginMessageDeliverable(!islandRunningRef.current)) return; // 暂停或关闭期间丢弃，不进队列、不补播。
                pushLocalNotification({ source: "plugin", pluginSource: message.source, pluginKey: message.key, pluginKind: message.kind, content: message.text, pluginUri: message.uri }, message.unixMillis);
            },
            (error) => console.warn("插件消息无效，已忽略", error),
        )
            .then((unlisten) => {
                if (active) stop = unlisten;
                else unlisten();
            })
            .catch((error: unknown) => console.warn("插件消息订阅失败", error));
        return () => {
            active = false;
            stop?.();
        };
    }, [pushLocalNotification]);
    const arrivals = useMemo(() => {
        const enabled = (snapshot?.instances ?? []).flatMap((instance): IslandAnchorArrival[] => {
            const manifest = snapshot?.runtime.manifests.find((item) => item.id === instance.componentId);
            const registration = snapshot?.runtime.instances.find((item) => item.instanceId === instance.instanceId);
            const source = manifest?.name ?? instance.componentId;
            if (instance.componentId === "official.music-control") return islandMusicAnchorVisible({ enabled: true, playback: media?.playback })
                ? [{ source, layer: ISLAND_NOTICE_LAYER, nowMs: 0, kind: "music", unit: { status: musicStatus, primary: musicCapsuleText, secondary: musicStatus === "running" ? `${musicPercent}%` : undefined } }] : [];
            if (instance.componentId === "official.weather" || instance.componentId === "official.collect-box") return [];
            const registrationStatus = registration?.status === "failed" ? "failed" : registration?.status === "ready" ? "pending" : "disconnected";
            const summary = pluginSummaries[instance.componentId];
            if (summary?.text === null) return []; // 插件显式隐藏锚点：不回落清单名，也不影响卡片与轮询。
            // 业务状态优先用插件同帧给出的 status，兼容旧插件时映射固定短词，最后回落注册状态。
            const status = islandPluginAnchorStatus(summary?.status, summary?.text, registrationStatus);
            // 插件卡在主信息行显示自己设置的摘要；未调用时回落清单名称。
            return [{ source, layer: "resident", nowMs: 0, kind: "plugin", componentId: instance.componentId, iconPath: manifest?.iconPath, uri: summary?.navigation?.uri, unit: { status, primary: islandPluginPrimary(summary, source) } }];
        });
        return [...enabled, ...contentFacts];
    }, [snapshot, media?.playback, musicStatus, musicCapsuleText, musicPercent, contentFacts, pluginSummaries]);
    const previousCapsule = useRef<{ ids: string[]; front?: string }>({ ids: [] });
    const content = useIslandContent(arrivals, notifications.anchor, mode, statusSpecs, preferences, previousCapsule.current.front);
    const baseAnchors = content.visible;
    const anchors = dropAnchor ? [dropAnchor, ...baseAnchors] : baseAnchors;
    const expandedContent = content.live.filter((anchor) => anchor.layer !== "resident" && anchor.id !== ISLAND_NOTIFICATION_ANCHOR_ID && anchor.kind !== "music");
    const empty = !baseAnchors.length;
    // 与时钟共用分钟对齐的唤醒；只在现有 hover 天气列表可能显示展望时保持节奏。
    const minuteClock = empty || mode === "hover" && baseAnchors.some((anchor) => anchor.id === ISLAND_NOTIFICATION_ANCHOR_ID)
        && notifications.rows.slice(0, ISLAND_MESSAGE_LIMIT).some((notice) => notice.weatherOutlookFacts?.hourly?.hours?.length);
    useEffect(() => {
        const now = Date.now();
        if (empty && islandClockLabel(now) !== islandClockLabel(clockNow)) setClockNow(now);
        if (mode === "collapsed")
        {
            // 回到常态时 clockNow 可能还是离开前的时刻：正文截止已过就先推进一次，再安排未来截止。
            const stale = notifications.rows.some((notice) => {
                const deadline = islandNotificationContentDeadline(notice);
                return deadline > clockNow && deadline <= now;
            });
            if (stale)
            {
                setClockNow(now);
                return;
            }
        }
        // 只安排未来的截止点：已经过去的正文截止不能再唤醒，否则会退化成 1ms 空转。
        const deadlines = mode === "collapsed" ? notifications.rows.flatMap((notice) => {
            const content = islandNotificationContentDeadline(notice);
            const retention = islandNotificationDeadline(notice) ?? Infinity;
            return [content, retention].filter((deadline) => deadline > now);
        }) : [];
        const next = Math.min(...deadlines, ...(minuteClock ? [now + 60_000 - now % 60_000] : []));
        if (!Number.isFinite(next)) return;
        const timer = setTimeout(() => setClockNow(Date.now()), Math.max(1, next - now));
        return () => clearTimeout(timer);
    }, [empty, minuteClock, notifications.rows, clockNow, mode]);
    const face = islandCapsuleFace(anchors, clockNow);
    // 胶囊消息图形取最新条目；换文身份取实际投影的锚点快照，保证与正在显示的正文同源。
    const capsuleMessageGlyph = islandCapsuleGlyph(notifications.rows[0]);
    const capsuleIds = face.kind === "anchors" && mode === "collapsed" ? face.visible.map((anchor) => anchor.id) : [];
    const selected = dropAnchor ?? anchors.find((anchor) => anchor.id === islandHoverSelect(anchors, content.selected));
    // hover 详情里真正的插件挂载点：与 renderHoverDetail 的匹配规则一致，并只在插件提供 hover 外壳时占用。
    const hoverPluginInstance = useMemo(() => {
        if (mode !== "hover" || !selected || !islandRunning) return undefined; // 暂停时不占 hover 挂载位。
        const plugin = instances.find((instance) => snapshot?.runtime.manifests.find((manifest) => manifest.id === instance.componentId)?.name === selected.source);
        const manifest = plugin && snapshot?.runtime.manifests.find((item) => item.id === plugin.componentId);
        return plugin && manifest && readPluginApi(manifest.id)?.renderIslandHover ? plugin : undefined;
    }, [mode, selected, islandRunning, instances, snapshot, pluginEpoch]); // pluginEpoch：脚本注册后重算。
    // 同一实例任一时刻只挂一处：收起为隐藏宿主，hover 给选中项，展开进卡片；暂停时全不挂。
    const pluginSites = islandPluginMountSites({
        running: islandRunning,
        mode,
        instanceIds: pluginCards.map((instance) => instance.instanceId),
    });
    const foreground = dropAnchor ?? content.active;
    const capsuleActive = foreground?.id !== ISLAND_NOTIFICATION_ANCHOR_ID && islandStatusSpec(foreground?.unit?.status ?? "", statusSpecs)?.disposition === "dot" ? undefined : foreground;
    // 换文身份跟随实际投影的锚点快照：条目身份取 noticeKey（同毫秒到达的不同条目也能区分），
    // 与正在显示的正文同一次提交；同一条目的进度刷新只改正文文本、不改身份，因此不重播交接。
    const messageHandoff = capsuleActive?.noticeKey ? `${capsuleActive.noticeKey}:${capsuleActive.unit?.status ?? ""}` : undefined;
    const relayFrom = previousCapsule.current.front;
    const previousIds = previousCapsule.current.ids;
    useLayoutEffect(() => { previousCapsule.current = { ids: capsuleIds, front: mode === "collapsed" ? capsuleActive?.id : undefined }; });
    const isMusic = (anchor: IslandAnchor | undefined) => anchor?.kind === "music";
    const status = foreground?.unit?.status;
    const announcementValue = islandAnchorValue(foreground?.unit?.secondary ?? "");
    const milestone = status === "running" && announcementValue?.kind === "percent" ? Math.floor(announcementValue.value / 25) * 25 : undefined;
    const notice = foreground?.id === ISLAND_NOTIFICATION_ANCHOR_ID ? notifications.rows[0] : undefined;
    const announcement = notice ? [notice.source, islandStatusSpec(notice.status)?.word, notice.content].filter(Boolean).join(" · ") :
        !foreground || foreground.id === ISLAND_NOTIFICATION_ANCHOR_ID ? "" : foreground.layer === "drop" ? foreground.unit?.primary :
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
    const focusedAnchor = useRef<HTMLButtonElement>(null); // 只在已聚焦锚点被撤下时接续方向键焦点。
    useLayoutEffect(() => {
        const previous = focusedAnchor.current;
        if (mode !== "hover") focusedAnchor.current = null;
        else if (previous && !previous.isConnected && document.activeElement === document.body) {
            focusedAnchor.current = null;
            hoverList.current?.querySelector<HTMLButtonElement>('[aria-pressed="true"]')?.focus();
        }
    }, [mode, selected?.id]);
    /** hover 的非交互测宽组与本次停留期间的最大宽度，退出后重新测。 */
    const hoverWidthSamples = useRef<HTMLDivElement>(null);
    const hoverSessionInline = useRef(0);
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
    // 卡内收起态：记忆开启时读偏好，关闭记忆时回落到该卡默认；收集箱默认收起，消息卡默认收起到最近一条，其余默认展开。
    const collapseMemory = preferences?.islandCardCollapseMemory ?? {};
    const collapseDefault = (componentId: string) => islandCardCollapseDefault(componentId, preferences?.islandCollectCollapsedDefault ?? true);
    // 记忆开关只决定是否写偏好，运行期收起态始终可切换。
    const cardCollapsed = (componentId: string) => collapsedCards[componentId] ?? collapseDefault(componentId);
    useLayoutEffect(() => {
        if (foldSave.current.inflight > 0) return; // 乱序回来的旧偏好不能把下一次点击的乐观状态覆盖掉。
        const memory = preferences?.islandCardCollapseMemory ?? {};
        const stored = preferences?.islandCardCollapsed ?? {};
        const next = Object.fromEntries(Object.entries(stored).filter(([id]) => memory[id] !== false));
        for (const [id, value] of Object.entries(transientFolds.current)) {
            if (memory[id] === false) next[id] = value;
            else delete transientFolds.current[id];
        }
        collapsedCardsRef.current = next;
        setCollapsedCards(next);
    }, [preferences?.islandCardCollapsed, preferences?.islandCardCollapseMemory]);
    // 只为实际卡内点击做高度过渡；三态外框布局和原生命中不参与此动画。
    useLayoutEffect(() => {
        if (mode !== "expanded") return;
        for (const [card, origin] of foldOrigins.current) {
            foldAnimations.current.get(card)?.cancel();
            foldAnimations.current.delete(card);
            const settle = () => {
                card.style.removeProperty("block-size");
                setFoldStates((current) => current[origin.id] ? { ...current, [origin.id]: settleIslandFold(current[origin.id], origin.seq) } : current);
                remeasure.current(true);
            };
            if (!card.isConnected) {
                settle();
                continue;
            }
            const natural = card.getBoundingClientRect().height;
            const target = islandFoldTargetHeight(natural, cardHeights.current[origin.id]);
            if (!animationsEnabled || target === undefined || Math.abs(target - origin.height) < 0.5) {
                settle(); // 关动效、测不到目标或无需动画时直接落到自然流，避免 0 高或无限忙。
                continue;
            }
            cardHeights.current[origin.id] = target;
            setFoldStates((current) => current[origin.id] ? { ...current, [origin.id]: animateIslandFold(current[origin.id], origin.seq) } : current);
            const animation = card.animate([{ blockSize: `${origin.height}px` }, { blockSize: `${target}px` }], { duration: ISLAND_ANCHOR_SHRINK_MS, easing: ISLAND_ANCHOR_EASING });
            foldAnimations.current.set(card, animation);
            animation.onfinish = () => {
                if (foldAnimations.current.get(card) !== animation) return;
                foldAnimations.current.delete(card);
                settle();
            };
        }
        foldOrigins.current.clear();
    }, [collapsedCards, mode, animationsEnabled]);
    // 展开态消息卡固定列宽：切换 1/全部只改高度，列偏移与窗口盒不随文字宽变化（否则窗口先变宽再居中，卡片左右抖）。
    useLayoutEffect(() => {
        const card = root.current?.querySelector<HTMLElement>(`[data-island-card-id="${ISLAND_MESSAGE_CARD_ID}"]`);
        if (!card) return;
        if (mode === "expanded") {
            card.style.inlineSize = `${ISLAND_EXPANDED_COLUMN_WIDTH}px`;
            remeasure.current(true);
        } else {
            card.style.removeProperty("inline-size");
        }
        return () => { card.style.removeProperty("inline-size"); };
    }, [mode]);

    useLayoutEffect(() => () => {
        foldOrigins.current.clear();
        for (const animation of foldAnimations.current.values()) animation.cancel();
        foldAnimations.current.clear();
        const sortGesture = cardSortGesture.current; // 切态/卸载时释放拖卡捕获与计时，不留下半截手势。
        if (sortGesture) {
            clearTimeout(sortGesture.timer);
            if (sortGesture.button.isConnected && sortGesture.button.hasPointerCapture(sortGesture.pointerId)) sortGesture.button.releasePointerCapture(sortGesture.pointerId);
        }
        cardSortGesture.current = undefined;
        cardSorting.current = false;
        setFoldStates({}); // 切态/关动效时清掉短期 pending，不回写偏好。
    }, [mode, animationsEnabled]);
    /** 点击先切本地状态，再串行持久化；同一时刻只让最后一份偏好回执更新页面。 */
    const handleToggleCard = (componentId: string, card: HTMLElement | null) => {
        const currentCollapsed = collapsedCardsRef.current[componentId] ?? collapseDefault(componentId);
        const collapsed = !currentCollapsed;
        const next = { ...collapsedCardsRef.current, [componentId]: collapsed };
        const fold = beginIslandFold({ phase: "idle", target: currentCollapsed, seq: foldSeqs.current[componentId] ?? 0 }, currentCollapsed);
        foldSeqs.current[componentId] = fold.seq;
        setFoldStates((current) => ({ ...current, [componentId]: fold }));
        if (animationsEnabled && card && layoutSettled.current) foldOrigins.current.set(card, { id: componentId, height: card.getBoundingClientRect().height, seq: fold.seq });
        else setFoldStates((current) => current[componentId] ? { ...current, [componentId]: settleIslandFold(current[componentId], fold.seq) } : current); // 关动效立即落自然高。
        collapsedCardsRef.current = next;
        setCollapsedCards(next);
        if (collapseMemory[componentId] === false) { transientFolds.current[componentId] = collapsed; return; }
        const save = foldSave.current;
        const revision = beginIslandFoldSave(save);
        const stored = Object.fromEntries(Object.entries(next).filter(([id]) => collapseMemory[id] !== false));
        save.queue = save.queue.then(async () => {
            try {
                if (!isIslandFoldSaveCurrent(save, revision)) return; // 旧 revision 早退也由 finally 减 inflight。
                const preferences = await saveIslandCardCollapsed(stored);
                if (revision === save.revision && mounted.current) onPreferences(preferences);
            } catch (error) {
                console.warn("浮岛卡收起状态保存失败", error);
            } finally {
                endIslandFoldSave(save);
            }
        });
    };
    /** 拖出结束后按原生光标判态：OLE 拖动期间页面收不到 pointerout，不能只等悬停事件。 */
    const handleCollectDragEnd = () => {
        void islandCursorInside()
            .then((inside) => {
                if (!mounted.current) return;
                pointerInside.current = inside;
                if (!inside && modeRef.current === "expanded") changeMode("collapsed", false);
            })
            .catch(() => undefined);
    };
    const edgeKey = useRef(""); // 上次测量的贴边方向，变化时丢弃旧几何缓存。
    const formKey = useRef(""); // 上次测量的岛形态，变化时丢弃旧胶囊高度与并集缓存。
    useLayoutEffect(() => {
        if (mode !== "hover") hoverSessionInline.current = islandHoverSessionWidth(false, hoverSessionInline.current, 0);
        if (!preferences || !work || !root.current) return;
        const samples = hoverWidthSamples.current;
        const hoverCard = samples?.parentElement;
        // 先更新宽度下限，再进入原有卡片实测与几何去重；镜像不属于卡片区域列表。
        const setHoverInline = (width: number) => {
            const value = `${width}px`;
            if (hoverCard?.style.getPropertyValue("--island-hover-inline") !== value) hoverCard?.style.setProperty("--island-hover-inline", value);
        };
        let active = true;
        geometryResizeAt.current = performance.now(); // 切态/重测刚开始，忽略窗口尺寸变化产生的假 leave。
        let failClosedResubmit = islandNativeFailClosed; // 仅在本次外部状态变化后的首个测量允许绕过一次去重。
        let frame = 0;
        let frameForce = false; // 已排队重测是否为强制；绘制写入的根尺寸会触发普通重测，不能把强制重测降级成按旧尺寸跳过。
        let motionFrame = 0;
        let maskFrame = 0; // 首次原生提交前的隐藏帧等待句柄；cleanup 时取消（等待另有超时兜底）。
        let wakeMaskFrame: (() => void) | undefined; // 隐藏帧等待的唤醒句柄：cleanup/接管时用，避免空等超时。
        let 隐藏帧门: { done: Promise<"frame" | "timeout" | "wake">; wake: () => void } | undefined; // 共享隐藏帧门：等帧期间的重测共用同一个门，避免互相推迟提交。
        let maskFrameWaiting = false; // 等隐藏帧期间：新的重测先挂起，避免在等帧窗口里被接管。
        let pendingMaskMeasure = false; // 等帧期间被挂起的重测，收口后再补跑一次。
        let wakeMotion: (() => void) | undefined;
        let wakeReveal: (() => void) | undefined; // 揭幕等待的唤醒器：cleanup/finishMotion 取消帧时用它收口。
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
        /**
         * 按属性名清掉 paint 留在根、卡片与卡片内容上的内联几何。
         * 保存集合是 effect 实例私有的：上一实例卸载时写入的重同步帧只有按名字清理才收得回来，否则会永久压住自然布局。
         */
        const clearPaintedStyles = () => {
            savedStyles.clear(); // 属性已按名字统一清掉，旧集合不再需要。
            const element = root.current;
            if (!element) return;
            for (const target of [element, ...element.querySelectorAll<HTMLElement>(".island-card, .island-card__content")]) {
                for (const property of PAINTED_STYLE_PROPERTIES) target.style.removeProperty(property);
            }
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
            seatBaseline.current = undefined; // 换边/换形态后旧贴边身份不再对应已提交几何。
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
            // 展开根固定到工作区，根尺寸相同也要重测内部卡片与滚动范围。
            if (!force && mode !== "expanded" && root.current && painted) {
                const box = root.current.getBoundingClientRect();
                // paint 写入的根尺寸会触发 ResizeObserver；它不改变内容，不能借机 restore 成自然布局。
                if (mode === painted.state && Math.abs(box.width - painted.width) < 0.6 && Math.abs(box.height - painted.height) < 0.6) return;
            }
            cancelAnimationFrame(frame);
            clearPaintedStyles(); // 动画被打断、切态竞争或上一实例留下的内联尺寸都要按属性名清掉，测量必须基于真实文档流。
            {
                if (!active || !root.current || dragging.current) return;
                // 展开只解除旧宽度测自然内容；固定高度保持不变，避免测量时滚动位置回退。
                const element = root.current;
                const expandedInline = element.style.getPropertyValue("--island-expanded-inline");
                if (mode === "expanded") element.style.removeProperty("--island-expanded-inline");
                if (samples) {
                    const naturalInline = islandHoverInlineSize(Array.from(samples.children, (sample) => sample.getBoundingClientRect().width), capsuleInline.current);
                    hoverSessionInline.current = islandHoverSessionWidth(true, hoverSessionInline.current, naturalInline);
                    setHoverInline(hoverSessionInline.current);
                }
                const origin = element.getBoundingClientRect();
                // 展开根铺满工作区，内容列由 padding 定位；测量卡片时去掉列偏移，几何函数再统一加回。
                const expandedPad = mode === "expanded" ? Number.parseFloat(element.style.getPropertyValue("--island-expanded-column") || "0") : 0;
                // 刘海非展开态由根 padding 让出圆弧区；测量时换回内容坐标，几何函数再统一加回 12 DIP。
                const notchPad = preferences.islandForm === "notch" && mode !== "expanded" ? 12 : 0;
                const horizontalEdge = edge === "left" || edge === "right";
                const measured = Array.from(root.current.querySelectorAll<HTMLElement>(".island-card")).flatMap((card) => {
                    const rect = card.getBoundingClientRect();
                    const local = {
                        left: rect.left - origin.left - expandedPad - (horizontalEdge ? 0 : notchPad),
                        top: rect.top - origin.top - (horizontalEdge ? notchPad : 0),
                        width: rect.width,
                        height: rect.height,
                    };
                    // 展开先量全列宽度，再由岛几何裁剪原生卡片区域，滚出视口不会改变列宽。
                    return mode === "expanded" ? [local] : visibleIslandRect(local, origin.height);
                });
                if (mode === "expanded" && expandedInline) element.style.setProperty("--island-expanded-inline", expandedInline);
                if (!measured.length) return;
                // 顶栏也可能撑宽卡片；插件自绘只在选中时挂载，只有真实卡片能测到它的宽度。
                if (hoverCard) {
                    hoverSessionInline.current = islandHoverSessionWidth(true, hoverSessionInline.current, measured[0].width);
                    setHoverInline(hoverSessionInline.current);
                }
                try {
                    const radius = mode === "collapsed" ? 22 : mode === "hover" ? 16 : preferences.islandRadius;
                    const pending = pendingOffset.current;
                    // 默认空边按顶部居中；拖动返回的显式偏移必须按有效边 top 计算，不能在中点丢掉。
                    const geometryEdge = pending !== undefined && edge === "" ? "top" : edge;
                    // 自然宽切换（胶囊/悬停换消息、relay 逐帧收放）在过渡期间把窗口左缘锚定在上一帧，
                    // 宽度向右扩展，避免“先按新宽居中、再随原生 margin 回执跳回”的横向抖动；稳定态仍按居中。
                    const 过渡锚定 = mode !== "expanded" && (transitionMasked.current || nativeSwap.pending > 0)
                        ? currentGeometry.current?.windowBox.left
                        : undefined;
                    let request = islandGeometry(mode, measured, radius, work, geometryEdge, pending ?? preferences.islandOffset, preferences.islandForm, { anchorStart: 过渡锚定 });
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
                    const expanded = request.state === "expanded";
                    const key = islandGeometryKey(request);
                    const element = root.current;
                    const cards = Array.from(element.querySelectorAll<HTMLElement>(".island-card"));
                    const from = currentGeometry.current;
                    // 卡内动画期间不更改原生窗口；若内容列确实变宽，待动画完成后再按正常规则测一次。
                    if (expanded && foldAnimations.current.size && from && islandWindowKey(from) !== key) return;
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
                    /**
                     * 等视口条件：inner 尺寸与目标视口一致后再等一帧；等待受剩余超时约束，
                     * 隐藏页 rAF 不触发时也会到点收口。返回是否在超时前匹配。
                     */
                    const awaitViewport = (viewport: IslandStateRequest["viewport"]) => awaitIslandViewport({
                        viewport,
                        inner: () => ({ width: window.innerWidth, height: window.innerHeight }),
                        wait: (remaining) => new Promise<void>((resolve) => {
                            const timer = setTimeout(resolve, Math.max(16, remaining));
                            requestAnimationFrame(() => {
                                clearTimeout(timer);
                                resolve();
                            });
                        }),
                        timeoutMs: ISLAND_VIEWPORT_WAIT_MS,
                    });

                    const applyViewportMargin = (value: IslandStateRequest) => {
                        const offset = value.state === "expanded" ? { x: 0, y: 0 } : islandContentOffset(value); // 展开根铺满工作区，列偏移交给卡片。
                        element.style.marginInlineStart = `${offset.x}px`;
                        element.style.marginBlockStart = `${offset.y}px`;
                        islandTrace("margin", { state: value.state, offset, windowBox: value.windowBox, viewport: value.viewport, el: { cls: element.className, marginInlineStart: element.style.marginInlineStart, marginBlockStart: element.style.marginBlockStart } });
                    };
                    const paint = (value: IslandStateRequest, elapsed: number, start?: number, staggerMs = 0) => {
                        const expandedPaint = value.state === "expanded";
                        const columnX = expandedPaint ? islandExpandedColumn(value) : 0; // 展开卡片列在满工作区根内的左边距。
                        paintedGeometry.current = expandedPaint
                            ? { state: value.state, width: work.width, height: work.height }
                            : { state: value.state, width: value.windowBox.width, height: value.windowBox.height };
                        saveStyle(element);
                        if (expandedPaint) element.style.setProperty("--island-expanded-column", `${columnX}px`);
                        Object.assign(element.style, {
                            position: "relative",
                            // 展开根铺满工作区接收指针，卡片列按贴边偏移绝对定位；紧凑态仍由窗口盒决定。
                            marginInlineStart: expandedPaint ? "0px" : `${value.windowBox.left - value.viewport.left}px`,
                            marginBlockStart: expandedPaint ? "0px" : `${value.windowBox.top - value.viewport.top}px`,
                            inlineSize: `${expandedPaint ? work.width : value.windowBox.width}px`,
                            blockSize: `${expandedPaint ? work.height : value.windowBox.height}px`,
                            maxInlineSize: "none",
                            overflow: "clip",
                        });
                        islandTraceLazy("paint", () => ({ state: value.state, expanded: expandedPaint, windowBox: value.windowBox, viewport: value.viewport, el: (() => { const rect = element.getBoundingClientRect(); return { cls: element.className, marginInlineStart: element.style.marginInlineStart, inlineSize: element.style.inlineSize, blockSize: element.style.blockSize, column: element.style.getPropertyValue("--island-expanded-column"), rect: { x: Math.round(rect.x), y: Math.round(rect.y), w: Math.round(rect.width), h: Math.round(rect.height) } }; })() }));
                        // 贴边侧角从最近实际绘制的带符号圆角过渡到目标态，连续切态不会先跳回普通圆角再长出补面。
                        const end = islandAttachedCorner(request.state, form, request.cards[0]?.radius ?? 0);
                        const attached = elapsed >= ISLAND_MOTION_MS ? end : islandAttachedCornerAt(start ?? end, request, elapsed, form);
                        // 内容只在展开相位 2 的其他卡上淡入；当前卡与收起等其它切态内容始终可见，避免 1→0→1 的整卡闪淡。
                        cards.forEach((card, index) => {
                            const shape = value.cards[index];
                            if (!shape) return;
                            // 展开相位 2：其他卡延迟 staggerMs 起步，内容与卡片本身同步淡入并去模糊。
                            const 相位淡入 = staggerMs > 0 && index > 0; // 只有展开相位 2 的其他卡参与淡入。
                            const localElapsed = 相位淡入 ? Math.max(0, elapsed - staggerMs) : elapsed;
                            const reveal = 相位淡入 && localElapsed < ISLAND_MOTION_MS ? Math.min(1, Math.max(0, localElapsed / ISLAND_MOTION_MS)) : 1;
                            saveStyle(card);
                            Object.assign(card.style, {
                                position: "absolute",
                                left: `${shape.rect.left + columnX}px`,
                                top: `${shape.rect.top}px`,
                                inlineSize: `${shape.rect.width}px`,
                                blockSize: `${shape.rect.height}px`,
                                maxInlineSize: "none",
                                minBlockSize: "0",
                                borderRadius: islandCornerRadius(shape.radius, form, edge, attached),
                                zIndex: String(cards.length - index), // 当前卡压在其他卡之上，其他卡从它背后弹出。
                            });
                            // hover 的自然宽度下限不能限制动画中插值的外框；restoreStyles 后自动恢复。
                            if (mode === "hover") card.style.minInlineSize = "0";
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
                        hadPaintedFrame.current = true; // 已经有画面可闪，后续换视口的提交必须等隐藏帧。
                    };
                    // 展开态不写回内联几何，必须单独收回上一帧的补面与描边抑制，否则会残留贴边弧面。
                    const clearFillets = () => {
                        element.dataset.filleted = "false";
                        attachedCorner.current = undefined; // 展开态没有贴边负圆角，下一次从目标态重新推导。
                        if (surface.current) surface.current.style.display = "none";
                    };
                    // 重试耗尽后同一目标几何不再重发：等外部状态变化的 fail-closed 重提或几何本身改变。
                    if (islandFailedGeometryGate({ failedKey: failedGeometry.current, key, resubmit: failClosedResubmit })) return;
                    const deduped = key === geometryKey.current && (!expanded || (from && islandWindowKey(from) === key));
                    if (deduped && !failClosedResubmit) {
                        // 展开同盒只更新页面本地可见卡片、透明列与滚动条；不把内部布局变更送回原生。
                        if (expanded) {
                            clearFillets();
                            applyViewportMargin(request);
                            currentGeometry.current = request;
                            inputGeometryKey.current = key;
                            layoutSettled.current = true;
                        } else {
                            // 稳态无需固定尺寸：hover 的原生命中可保留并集，绘制与实测仍以自然内容盒为原点。
                            applyViewportMargin(natural);
                            paintAttached(islandAttachedCorner(natural.state, form, natural.cards[0]?.radius ?? 0), natural);
                        }
                        targetGeometry.current = request; // 同一窗口盒下滚动、透明列和卡片尺寸仍可变化。
                        refreshScrollbar();
                        return;
                    }
                    if (failClosedResubmit) failClosedResubmit = false; // 一次外部变化只强制重提一次，防自激。
                    // 几何键留到隐藏帧等待之后再登记（见 commit）：等帧期间被替换时不留可被继任去重的键。
                    geometryResizeAt.current = performance.now(); // 原生窗口即将换尺寸，300ms 内 leave 视为假离开。
                    committing = true;
                    lastCompactFrame = now;
                    const reseat = skipMotionOnce.current; // 拖动落点后的首次提交：父窗口已移位，必须整层遮蔽一次再回锚。
                    skipMotionOnce.current = false;
                    // 同视口只改窗口盒（如胶囊加宽）时不遮蔽：整层隐藏会把新消息交接动画一起藏掉。
                    const commitDecision = islandGeometryCommitDecision(from, request, seatBaseline.current, { edge: nextEdge, form: nextForm }, reseat);
                    islandTrace("geometry-commit", { state: request.state, mask: transitionMasked.current || commitDecision.mask, reseat, fromWindow: from?.windowBox, fromViewport: from?.viewport, nextWindow: request.windowBox, nextViewport: request.viewport, seat: commitDecision.seat });
                    seatBaseline.current = commitDecision.seat; // 基准随本次提交前移，同一 effect 内后续窗口盒变化不再遮蔽。
                    const maskToken = beginMask(transitionMasked.current || commitDecision.mask); // 换视口前先领取租约；不需要遮蔽时只占租约。
                    cancelExpandedReturn(true);
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
                        paint(from, 0, attachedCorner.current ?? islandAttachedCorner(from.state, form, from.cards[0]?.radius ?? 0)); // 过渡期保持旧坐标系：根与卡片都用旧原点。
                    }
                    resyncPaint = (value) => { if (!active || !root.current) return; paint(value, 0); }; // 当前绘制闭包供唯一解除入口重同步；卸载/替换后不再画。
                    /** native 全部回包后，统一交给 settleMaskRelease 结算挂起的遮蔽。 */
                    const drainMaskReleases = (freshApplied: boolean, failed = false) => {
                        if (nativeSwap.pending > 0) return;
                        for (const release of takeIslandMaskReleases(deferredMaskReleases)) {
                            // 保留挂起时的 keepOwner/alreadySynced：被替换或已卸载的实例只结算租约，不再重画。
                            settleMaskRelease(release.token, { keepOwner: release.keepOwner, alreadySynced: release.alreadySynced, resynced: freshApplied, failed });
                        }
                    };
                    let commitFailed = false; // 本任务最后一次原生提交是否补偿失败。
                    let 隐藏帧已等 = false; // 本任务首次原生提交前是否已经等过「隐藏帧提交」。

                    const commit = async (value: IslandStateRequest): Promise<boolean> => {
                        if (!ownsMask(maskToken)) { settleMaskRelease(maskToken); return false; } // 新任务接管后旧任务不再提交坐标。
                        // 带遮蔽的视口切换：先把 visibility:hidden 真正提交一帧，再换窗口原点；
                        // 否则合成器仍持有旧的可见胶囊帧，窗口移到 (0,0) 后可能把它画在左上角。
                        if (!隐藏帧已等) {
                            隐藏帧已等 = true;
                            // 已有画面（换边/形态/切态）且窗口原点会变时必须等隐藏帧；首次挂载没有旧内容可闪，不推迟首次提交。
                            // 换边/形态变化时 currentGeometry 已被清空，from 为空；若只按 hadPaintedFrame 判定，
                            // 这里会在提交前 await，把租约暴露给等帧期间的重测（先 beginMask 抢租约、再因同键去重跳过），
                            // 提交会挂死。因此保持 from 条件：只覆盖主路径的展开/收起，换边保护留待重做提交骨架后再启用。
                            if (from && hadPaintedFrame.current && transitionMasked.current && islandViewportChanged(from, request) && !maskPainted.current) {
                                隐藏帧门 ??= islandRenderGate({
                                    count: 1, // 一帧 + 帧后宏任务 = 一次完整绘制机会（rAF 在绘制前执行，单独 rAF 不能证明已上屏）。
                                    timeoutMs: ISLAND_NO_FRAME_TIMEOUT_MS, // 两次机会共享同一个总超时。
                                    requestFrame: (callback) => {
                                        maskFrame = requestAnimationFrame(callback);
                                        return maskFrame;
                                    },
                                    cancelFrame: (handle) => cancelAnimationFrame(Number(handle)),
                                    setTimer: (callback) => setTimeout(callback, ISLAND_NO_FRAME_TIMEOUT_MS),
                                    clearTimer: (handle) => clearTimeout(handle as ReturnType<typeof setTimeout>),
                                });
                                const 门 = 隐藏帧门;
                                wakeMaskFrame = 门.wake;
                                void 门.done.finally(() => { if (隐藏帧门 === 门) 隐藏帧门 = undefined; }); // 门收口即释放，下一轮换视口重新建门。
                                maskFrameWaiting = true;
                                let 收口: "frame" | "timeout" | "wake";
                                try {
                                    收口 = await 门.done;
                                } finally {
                                    maskFrameWaiting = false;
                                }
                                pendingMaskMeasure = false; // 挂起的重测交给提交后的 ResizeObserver 重测，避免在登记键之前抢租约。
                                wakeMaskFrame = undefined;
                                if (收口 === "frame") await new Promise<void>((resolve) => setTimeout(resolve, 0)); // 等这一帧画完（rAF 之后的宏任务）。
                                if (!active || !ownsMask(maskToken)) {
                                    // 等待期间被接管/卸载：本次提交作废。清掉去重键并重测一次，
                                    // 否则接管者会因同一几何键被去重跳过，窗口停在旧几何上。
                                    if (active && geometryKey.current === key) { geometryKey.current = ""; measure(true); }
                                    return false;
                                }
                                islandTrace("mask-frame", { settled: 收口 }); // 区分 timeout 收口与真实 frame 收口。
                                maskPainted.current = true; // owner/active 校验通过后才记：wake/接管路径不得替继任者记成已绘制。
                            }
                        }
                        // 隐藏帧已到/已上屏：现在才登记几何键与预留。等帧期间被替换时不留可被继任去重的键。
                        if (!pendingSubmit.current || pendingSubmit.current.token !== maskToken) {
                            geometryKey.current = key;
                            pendingSubmit.current = { key, token: maskToken, submitted: false, submittedFinal: false };
                        }
                        let response = false;
                        commitFailed = false; // 每次提交先清上一次失败标记，成功路径不能沿用旧失败。
                        if (pendingSubmit.current && pendingSubmit.current.token === maskToken) {
                            pendingSubmit.current.submitted = true; // 已发过渡帧（并集等中间帧）。
                            if (value === request) pendingSubmit.current.submittedFinal = true; // 只有真正发出最终目标才算收口：cleanup 据此决定是否撤销。
                        }
                        beginIslandNativeSwap(nativeSwap);
                        try {
                            try {
                                islandTrace("native-request", { state: value.state, windowBox: value.windowBox, viewport: value.viewport });
                                await setIslandState(value);
                                response = true;
                                islandTrace("native-ack", { state: value.state });
                            } catch (error) {
                                // 原生失败会清几何并隐藏窗口；当前租约持有者有界补偿重提交。
                                if (ownsMask(maskToken)) {
                                    response = await islandRetrySubmit({
                                        submit: () => setIslandState(value),
                                        wait: (milliseconds) => new Promise<void>((resolve) => setTimeout(resolve, milliseconds)),
                                        current: () => ownsMask(maskToken),
                                    });
                                }
                                if (!response && ownsMask(maskToken)) {
                                    islandNativeFailClosed = true;
                                    commitFailed = true;
                                    console.warn("岛原生几何提交失败，已标记 fail-closed，等待下一次状态变化重试", error);
                                }
                            }
                        } finally {
                            finishIslandNativeSwap(nativeSwap, String(maskToken), response); // 任何返回路径都成对归零 pending。
                            if (response) {
                                nativeSwap.appliedGeometry = value; // 成功回包保留实际几何供重同步。
                                islandNativeFailClosed = false;
                                failedGeometry.current = ""; // 失败门随成功提交清除。
                                commitFailed = false;
                            }
                            drainMaskReleases(response, !response); // reject 也一定会 drain。
                        }
                        if (!active || !ownsMask(maskToken)) {
                            settleMaskRelease(maskToken, { resynced: response, failed: !response }); // 回包作废也走唯一结算入口。
                            return false;
                        }
                        if (!response) {
                            // 重试耗尽不进外层异常分支：这里显式清空几何与贴边基准，恢复提交按「没有可比坐标」重新遮蔽。
                            if (ownsMask(maskToken)) {
                                const baseline = resetIslandGeometryBaseline();
                                currentGeometry.current = baseline.geometry;
                                seatBaseline.current = baseline.seat;
                                geometryKey.current = baseline.key;
                            }
                                failedGeometry.current = islandGeometryKey(request); // 始终记外层最终目标：动画中间帧的并集 key 不能当失败门。
                            commitFailed = true;
                            return false;
                        }
                        currentGeometry.current = value; // 只在持有当前显示租约时写共享几何。
                        if (!dragging.current) inputReady.current = true;
                        return true;
                    };
                    finish = submission.current
                        .catch(() => undefined)
                        .then(async () => {
                            if (!active || !mounted.current) { settleMaskRelease(maskToken); return; }
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
                                    if (!(await commit(unionFirst))) return; // 回包作废时直接丢弃，不绘制旧/新坐标。
                                    if (!active || !ownsMask(maskToken)) return; // 旧回包不得绘制或解除新遮蔽。
                                    paint(unionFirst, 0, startAttached, staggerMs);
                                    // 只有换了固定视口带（页面原点变化）才需要等新帧提交；同视口的窗口盒变化沿用旧节奏，不为一次形变多压两帧。
                                    if (islandViewportChanged(startGeometry, request)) {
                                        // 等页面真正落到目标视口（inner 与目标 viewport 一致）再解除遮蔽：双 rAF 覆盖不了 WebView 换视口的偶发滞后。
                                        const 已到位 = await awaitViewport(request.viewport);
                                        if (!active || !ownsMask(maskToken)) return; // 等待期间卸载或被接管：交给新任务结算。
                                        islandTrace("frame-barrier", { where: "union", matched: 已到位 });
                                    }
                                    islandTrace("mask-release-union", {});
                                    settleMaskRelease(maskToken, { keepOwner: true, alreadySynced: true }); // 回包并同步目标布局后显示，保留所有权给动画。
                                    // 揭幕后的首个帧等待可被 cleanup/finishMotion 唤醒：取消帧不能让 submission 挂死。
                                    const reveal = islandWakingFrame(
                                        (callback) => {
                                            motionFrame = requestAnimationFrame(callback);
                                            return motionFrame;
                                        },
                                        // 无帧时由计时器推进：隐藏页不派发帧也不会把 submission 卡在揭幕等待上。
                                        { timeoutMs: ISLAND_NO_FRAME_TIMEOUT_MS, setTimer: (callback) => setTimeout(callback, ISLAND_NO_FRAME_TIMEOUT_MS), clearTimer: (handle) => clearTimeout(handle as ReturnType<typeof setTimeout>) },
                                    );
                                    wakeReveal = reveal.wake; // cleanup/finishMotion 取消帧时用它收口。
                                    await reveal.done;
                                    wakeReveal = undefined;
                                    if (!active) return;
                                    const started = performance.now();
                                    let last = -Infinity;
                                    let previousKey = "";
                                    while (active && !forceFinish) {
                                        const elapsed = await new Promise<number>((resolve) => {
                                            wakeMotion = () => resolve(motionTotal);
                                            // 无帧兜底：到点按动画结束收口，避免隐藏页把 submission 挂在下一帧上。
                                            const 无帧计时 = setTimeout(() => {
                                                wakeMotion = undefined;
                                                resolve(motionTotal);
                                            }, ISLAND_NO_FRAME_TIMEOUT_MS);
                                            motionFrame = requestAnimationFrame((now) => {
                                                clearTimeout(无帧计时);
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
                                            if (!(await commit(value))) return; // 中间帧重试耗尽：直接进 finally，不再提交最终目标绕开失败门。
                                            if (!active || !ownsMask(maskToken)) break;
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
                                        // 入态并集已占满工作区高度；终帧只恢复自然列宽与滚动布局，不再收缩窗口高度。
                                        applyViewportMargin(request); // 原生窗口尺寸变化前先算目标内容偏移。
                                        element.style.setProperty("--island-expanded-inline", `${work.width}px`);
                                        element.style.setProperty("--island-expanded-block", `${work.height}px`);
                                        if (!(await commit(request))) return;
                                        if (active && ownsMask(maskToken)) {
                                            clearFillets();
                                            applyViewportMargin(request);
                                            targetGeometry.current = request;
                                            element.style.setProperty("--island-expanded-inline", `${work.width}px`);
                                            element.style.setProperty("--island-expanded-block", `${work.height}px`);
                                            paintedGeometry.current = {
                                                state: "expanded",
                                                width: work.width,
                                                height: work.height,
                                            };
                                            // 终帧恢复相对流之前先按最终列写一次左边距：上一帧可能是并集列，不能让它带进终帧。
                                            element.style.setProperty("--island-expanded-column", `${islandExpandedColumn(request)}px`);
                                            restoreStyles(); // 先回到自然文档流，再显示目标布局。
                                            applyViewportMargin(request);
                                            element.style.setProperty("--island-expanded-inline", `${work.width}px`);
                                            element.style.setProperty("--island-expanded-block", `${work.height}px`);
                                            remeasure.current(true); // 动画结束按实际几何复核一次。
                                        }
                                    } else if (!animate && grows) {
                                        // 目标比当前窗口大时先提交窗口再绘制，否则旧窗口会裁掉新布局一帧；动画路径的窗口已固定为并集。
                                        if (!(await commit(request))) return;
                                        if (active && ownsMask(maskToken)) paint(request, ISLAND_MOTION_MS);
                                    } else {
                                        paint(request, ISLAND_MOTION_MS);
                                        if (!(await commit(request))) return;
                                    }
                                    recovering.current = false;
                                    // 终态要揭幕且换了视口：等页面真正落到目标视口再交还 finally 结算（无动效展开、紧凑态跨视口都走这里）。
                                    if (transitionMasked.current && islandViewportChanged(from, request)) {
                                        const 已到位 = await awaitViewport(request.viewport);
                                        if (!active) return;
                                        islandTrace("frame-barrier", { where: "final", matched: 已到位 });
                                    }
                                    settled = true;
                                } // 有界动画结束必须补最终态，不受去重键影响。
                            } catch (error) {
                                if (ownsMask(maskToken)) {
                                    currentGeometry.current = undefined; // 只有当前租约持有者能清共享几何。
                                    seatBaseline.current = undefined; // 基准与几何成对失效，失败后按首次提交保守遮蔽。
                                    geometryKey.current = "";
                                }
                                console.warn("岛状态提交失败", error);
                                if (active && mounted.current) {
                                    changeMode("collapsed", false); // 原生仍执行 fail-closed，恢复从常态开始。
                                    if (!recovering.current) {
                                        recovering.current = true;
                                        setDragRevision((value) => value + 1);
                                    }
                                }
                            } finally {
                                animating = false;
                                committing = false;
                                // 成功路径先把终态、几何键、滚动与重测做完，最后才释放租约。
                                if (settled && active && ownsMask(maskToken) && request.state !== "expanded") {
                                    applyViewportMargin(natural);
                                    paintAttached(islandAttachedCorner(natural.state, form, natural.cards[0]?.radius ?? 0), natural);
                                }
                                if (settled && active && ownsMask(maskToken) && modeRef.current === mode) {
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
                                if (ownsMask(maskToken)) settleMaskRelease(maskToken, { alreadySynced: settled, failed: commitFailed }); // 成功终态后或失败兜底，统一结算。
                            }
                        });
                    submission.current = finish;
                    finishMotion.current = async () => {
                        forceFinish = true;
                        cancelAnimationFrame(motionFrame);
                        wakeMotion?.();
                        wakeReveal?.();
                        wakeReveal = undefined;
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
            if (maskFrameWaiting) { pendingMaskMeasure = true; frameForce ||= force; return; } // 等隐藏帧期间的重测先挂起，收口后补跑。
            cancelAnimationFrame(frame);
            frameForce ||= force; // 后到的普通重测（RO 自触发）不能把强制重测降级，否则仍按绘制尺寸跳过。
            frame = requestAnimationFrame(() => {
                const nextForce = frameForce;
                frameForce = false;
                measureNow(nextForce);
            });
        };
        remeasure.current = measure; // 供消息集合变化等无 DOM 尺寸变化的内容更新主动调用。
        let resyncPaint: ((value: IslandStateRequest) => void) | undefined; // 由当前 measureNow 的 paint 提供，用于解除前重同步。
        /** 唯一的解除入口：native 已清空、token 匹配/重同步后，先重绘生效一帧再解除。 */
        const settleMaskRelease = (token: number, options: { keepOwner?: boolean; resynced?: boolean; alreadySynced?: boolean; failed?: boolean } = {}) => {
            const { keepOwner = false, resynced = false, alreadySynced = false, failed = false } = options;
            if (!ownsMask(token)) return;
            if (nativeSwap.pending > 0) { deferIslandMaskRelease(deferredMaskReleases, token, { keepOwner, alreadySynced }); return; } // 等回包后再结算，语义随 token 保留。
            if (!transitionMasked.current) {
                if (!keepOwner) releaseMask(token, true);
                return;
            }
            const actual = nativeSwap.appliedGeometry ?? currentGeometry.current;
            if (failed || (!resynced && nativeSwap.appliedToken !== String(token))) {
                if (!maskFailTimers.has(token)) {
                    maskFailTimers.set(token, setTimeout(() => {
                        maskFailTimers.delete(token);
                        settleMaskRelease(token, { keepOwner, resynced: true, alreadySynced });
                    }, 800)); // native 失败兜底，避免永久隐藏。
                }
                return;
            }
            const timer = maskFailTimers.get(token);
            if (timer !== undefined) { clearTimeout(timer); maskFailTimers.delete(token); }
            if (!alreadySynced && active) {
                restoreStyles(); // 先还样式，再由重绘写入实际几何，restore 不会覆盖新帧。
                // 被替换或已卸载的实例只结算租约：旧闭包不得再用固定尺寸/透明度写回新实例的 DOM。
                if (canResyncIslandGeometry({ active, applied: actual !== undefined, alreadySynced }) && actual && resyncPaint) resyncPaint(actual);
            }
            // 解除遮蔽：帧到或超时任一先到都只执行一次；隐藏页不派发帧时也能释放租约、结束提交。
            islandFrameOrTimeout({
                act: () => {
                    if (!ownsMask(token)) return;
                    if (keepOwner) unmaskTransition(token, true);
                    else releaseMask(token, true);
                    if (pendingEscape.current) { pendingEscape.current = false; changeMode("collapsed", false); } // 遮蔽延长后补收期间按下的 Esc。
                },
                requestFrame: (callback) => requestAnimationFrame(callback),
                cancelFrame: (handle) => cancelAnimationFrame(Number(handle)),
                setTimer: (callback) => setTimeout(callback, ISLAND_NO_FRAME_TIMEOUT_MS),
                clearTimer: (handle) => clearTimeout(handle as ReturnType<typeof setTimeout>),
                timeoutMs: ISLAND_NO_FRAME_TIMEOUT_MS,
            });
        };
        const observer = new ResizeObserver(() => measure(false));
        observer.observe(root.current);
        const handleScroll = () => { refreshScrollbar(); measure(true); }; // 展开态滚动必须重新计算可见卡片，不受自触发 resize 跳过影响。
        root.current.addEventListener("scroll", handleScroll);
        for (const card of root.current.querySelectorAll(".island-card")) observer.observe(card);
        const contentObserver = createIslandContentResizeObserver(() => measure(true), ResizeObserver);
        const compactContent = root.current.querySelector(".island-card--anchors .island-card__content");
        if (compactContent) contentObserver.observe(compactContent);
        if (samples) contentObserver.observe(samples); // 未选中的宽内容变化也可触发重测。
        observeIslandPluginLayout(root.current, (target) => contentObserver.observe(target)); // 插件分组展开/收缩也触发重测。
        measureNow(true); // 首帧绘制前完成目标实测与内容冻结，不先闪出最终布局。
        const element = root.current;
        return () => {
            active = false;
            remeasure.current = () => {}; // 卸载后不再调度测量帧。
            restoreStyles(); // 卸载前先撤掉本实例写入的内联几何：下一实例的保存集合不认识它们，重同步帧会永久压住自然布局。
            element.removeEventListener("scroll", handleScroll);
            observer.disconnect();
            contentObserver.disconnect();
            cancelAnimationFrame(frame);
            cancelAnimationFrame(motionFrame);
            wakeMotion?.();
            wakeReveal?.();
            wakeReveal = undefined;
            // 撤销本实例「已预留但尚未首发」的几何：否则继任 effect 会因同 key 去重跳过，而本任务已 inactive，
            // 原生收起/展开请求两边都不会发出。首发之后不再撤销。
            const 撤销键 = revokeIslandReservation(pendingSubmit.current, maskLease.current.owner ?? -1, geometryKey.current);
            if (撤销键 !== undefined) {
                geometryKey.current = "";
                pendingSubmit.current = undefined;
            }
            wakeMaskFrame?.(); // 先唤醒等待方，避免取消后空等 300ms 超时。
            wakeMaskFrame = undefined;
            cancelAnimationFrame(maskFrame); // 隐藏帧等待句柄（等待已唤醒收口，帧只作兜底取消）。
            if (maskLease.current.owner !== undefined) settleMaskRelease(maskLease.current.owner, { resynced: nativeSwap.appliedToken !== null, alreadySynced: true }); // 唯一入口：本实例已还原样式，不再重同步重绘；pending 挂续延，失败 800ms 兜底。
        };
    }, [mode, snapshot, preferences, work, edge, animationsEnabled, dragRevision]);
    useEffect(() => {
        // 展开根高度固定，消息卡增减可能不会触发根 RO，必须主动重测。
        remeasure.current(true);
    }, [messages, messageFailure, mode, anchors.length, drop]);
    useEffect(() => {
        const pending = pendingOffset.current;
        if (pending === undefined || !preferences) return;
        if ((preferences.islandEdge || "top") === (edge || "top") && Math.abs(preferences.islandOffset - pending) < 0.01) pendingOffset.current = undefined; // 权威落点已到，不再压住后续偏移。
    }, [preferences, edge]);
    const interactionBusy = () => dragging.current || Boolean(gesture.current) || Boolean(scrollbarDrag.current);
    // 常态关闭透明像素检测：门闩只在 hover 与展开态参与判定，常态进入仍是原 180ms 消抖。
    const hoverLatched = () => modeRef.current !== "collapsed" && marginState.current.latch !== "open";
    const scheduleHover = (inside: boolean) => {
        if (inside && modeRef.current === "collapsed" && expandedReentryGate.current) return;
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
            if (pointerInside.current === inside && islandHoverTarget(modeRef.current, inside, interactionBusy(), hoverLatched()) === target) changeMode(target, false);
        }, ISLAND_HOVER_DELAY_MS);
    };
    const leaveIsFake = (event?: { screenX: number; screenY: number }): boolean => {
        const eventPoint = event && Number.isFinite(event.screenX) && Number.isFinite(event.screenY) ? { x: event.screenX, y: event.screenY } : undefined;
        const point = eventPoint ?? lastPointerScreen.current; // 事件屏幕坐标优先，last 只兜底。
        const frame = currentGeometry.current;
        const expanded = modeRef.current === "expanded";
        const layout = expanded ? scrollbarFrame.current : undefined;
        const expandedWork = work ?? frame?.viewport;
        const windowRect = expanded ? expandedWork : frame?.windowBox; // state windowBox 是屏幕绝对 DIP。
        const contentRect = expanded && expandedWork && layout?.column ? { left: expandedWork.left + layout.column.left, top: expandedWork.top + layout.column.top, width: layout.column.width, height: layout.column.height } : undefined;
        return islandLeaveSuppressed({
            masked: transitionMasked.current,
            graceActive: islandExpandedGraceActive(expandedGrace.current, performance.now(), lastPointerPoint.current),
            resizedAt: geometryResizeAt.current,
            now: performance.now(),
            point,
            windowRect,
            contentRect,
        });
    };
    const leaveWindow = (event?: { screenX: number; screenY: number }) => {
        if (leaveIsFake(event)) return; // 窗口放大/刚展开时的 pointerleave 不是真正离岛；真实屏幕坐标在盒外仍放行。
        if (cardSorting.current) return; // 拖动排序期间保持当前态，释放后再按落点判定。
        pointerInside.current = false;
        resetExpandedDwell();
        expandedReentryGate.current = false; // 真正离开后不再阻止下一次 hover。
        clearTimeout(expandedReentryGateTimer.current);
        expandedReentryGateTimer.current = undefined;
        if (modeRef.current === "collapsed") marginState.current = { latch: "open" };
        if (hoverKind.current !== "leave") stopHoverTimer();
        cancelDragHold(); marginState.current.since = undefined;
        if (interactionBusy()) return;
        marginState.current = islandMarginStep(marginState.current, { mode: modeRef.current, region: "outside", now: performance.now(), busy: false, outlineDistance: Infinity, coreDistance: Infinity, leave: true, detect: modeRef.current !== "collapsed" }).state;
        if (dropActiveRef.current) return; // 投放拖动期间页面指针本就不可信，保持当前态等原生投放事件收口。
        if (modeRef.current === "expanded") changeMode("collapsed", false); else scheduleHover(false);
    };
    const handleLeave = (event: PointerEvent<HTMLElement>) => {
        if (leaveIsFake(event)) return; // 假离开不撤触发者、不收起；屏幕坐标在盒外才继续。
        // 根之外仍可能是union内的body透明区，交给pointermove判定；只把真正离开文档当传统离岛。
        if (event.relatedTarget instanceof Node && document.documentElement.contains(event.relatedTarget)) return;
        navigationRequest.current += 1;
        setNavigationFeedback(undefined); // 触发者离开就撤下失败提示。
        leaveWindow(event);
    };
    const handleEnter = (event?: PointerEvent<HTMLElement>) => {
        pointerInside.current = true;
        expandedReentryGate.current = false; // 重新进入说明已离开过，解除极短 gate。
        clearTimeout(expandedReentryGateTimer.current);
        expandedReentryGateTimer.current = undefined;
        scheduleHover(true);
    };
    /** 按下或重新入卡都抢先取消空白回退，包括正在等待滚动条渐出的自动收起。 */
    const cancelExpandedReturn = (content: boolean) => {
        resetExpandedDwell(content);
        if (!expandedDwellClosing.current) return;
        expandedDwellClosing.current = false;
        cancelCollapseWait();
        refreshScrollbar();
    };
    /** 展开使用缓存的列/槽几何，只有内容→空白的边沿能启动计时。 */
    const moveExpandedPointer = (event: globalThis.PointerEvent) => {
        const gracePoint = { x: event.clientX, y: event.clientY };
        if (islandExpandedGraceActive(expandedGrace.current, performance.now(), gracePoint)) return; // 刚展开的同一交互不启动空白计时。
        expandedGrace.current = undefined;
        if (transitionMasked.current) { cancelExpandedReturn(true); return; } // 遮蔽期间不响应透明区回退。
        if (cardSorting.current) {
            cancelExpandedReturn(true); // 拖卡排序期间第二指针移到空白也不启动/续期 120ms 回退。
            return;
        }
        const layout = scrollbarFrame.current;
        if (!layoutSettled.current || !layout || inputGeometryKey.current !== geometryKey.current) return;
        const point = { x: event.clientX, y: event.clientY };
        if (!islandPointInRect(point, layout.window)) { leaveWindow(); return; }
        pointerInside.current = true;
        const inGutter = layout.gutter && islandPointInRect(point, layout.gutter);
        const target = event.target instanceof Element ? event.target : undefined;
        const onCard = Boolean(target?.closest(".island-card"));
        const blank = islandExpandedBlank(layout, point, onCard);
        // 卡间隙和滚动槽只保护，不算入卡；没有元素目标时以缓存的列边界兜底。
        const sample = blank ? "blank" : !inGutter && (onCard || !target) ? "content" : "protected";
        const step = islandExpandedDwellStep(expandedDwell.current, sample);
        expandedDwell.current = step.state;
        if (blank) expandedDwellPoint.current = point; // 只更新位置，保留原截止时刻。
        if (step.timer === "cancel" || (!blank && expandedDwellClosing.current)) cancelExpandedReturn(sample === "content");
        if (step.timer !== "start") return;
        expandedDwellTimer.current = setTimeout(() => {
            expandedDwellTimer.current = undefined;
            const latest = scrollbarFrame.current, cursor = expandedDwellPoint.current;
            if (!mounted.current || modeRef.current !== "expanded" || expandedDwell.current !== "blank" || islandExpandedDwellBlocked(cardSorting.current, interactionBusy()) || !inputReady.current || dropActiveRef.current || !latest || !cursor) return;
            // 空白列可能在计时期间被变高的卡片覆盖；到点只确认一次当前列。
            const stillBlank = latest.bands.some((band) => islandPointInRect(cursor, band)) && (!latest.gutter || !islandPointInRect(cursor, latest.gutter));
            if (!stillBlank) { resetExpandedDwell(true); return; }
            expandedDwellClosing.current = true;
            changeMode("collapsed", false);
        }, ISLAND_MARGIN_DELAY_MS);
    };
    // 展开只跟踪边界，紧凑两态沿用现有轮廓和门闩。
    inputHandlers.current = {
        move: (event) => {
            lastPointerPoint.current = { x: event.clientX, y: event.clientY };
            lastPointerScreen.current = { x: event.screenX, y: event.screenY };
            const drag = scrollbarDrag.current;
            if (drag) {
                if (event.pointerId === drag.id && root.current && drag.travel > 0) root.current.scrollTop = drag.top + (event.clientY - drag.y) * drag.range / drag.travel;
                return;
            }
            if (transitionMasked.current) { cancelExpandedReturn(true); return; } // 换视口期间不接受指针判定。
            if (event.pointerType === "touch") return;
            if (modeRef.current === "expanded") {
                if (!interactionBusy() && inputReady.current && !dropActiveRef.current) moveExpandedPointer(event);
                return;
            }
            if (interactionBusy() || !inputReady.current) {
                marginState.current.since = undefined;
                if (hoverKind.current === "margin") stopHoverTimer();
                return;
            }
            const element = root.current, frame = currentGeometry.current;
            if (!element || !frame) return;
            const point = { x: event.clientX, y: event.clientY };
            const layout = islandScrollGeometry(frame, targetGeometry.current, preferences?.islandEdge ?? "", { mode: modeRef.current, settled: layoutSettled.current, scrollHeight: element.scrollHeight, clientHeight: element.clientHeight, scrollTop: element.scrollTop }, work);
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
            if (next.collapse) { if (!dropActiveRef.current) changeMode("collapsed", true); return; }
            if (region === "transparent" && marginState.current.since !== undefined) {
                if (hoverKind.current === "margin" && hoverTimer.current !== undefined) return;
                stopHoverTimer(); hoverKind.current = "margin";
                const version = hoverVersion.current;
                const deadline = marginState.current.since + ISLAND_MARGIN_DELAY_MS;
                hoverTimer.current = setTimeout(() => {
                    hoverTimer.current = undefined; hoverKind.current = undefined;
                    const last = lastMarginInput.current;
                    if (version !== hoverVersion.current || !mounted.current || !last || marginState.current.since === undefined) return;
                    // setTimeout已达到deadline；受控断言时performance.now可暂停，仍不依赖rAF推进。
                    const result = islandMarginStep(marginState.current, { ...last, mode: modeRef.current, now: Math.max(performance.now(), deadline), busy: interactionBusy() || !inputReady.current });
                    marginState.current = result.state;
                    if (result.collapse && !dropActiveRef.current) changeMode("collapsed", modeRef.current === "hover");
                }, Math.max(0, Math.ceil(deadline - performance.now())));
                return;
            }
            if (hoverKind.current === "margin") stopHoverTimer(); // 离开延时区：撤销未到点的退回计时。
            if (modeRef.current === "hover") scheduleHover(region === "content");
        },
        leave: (event) => leaveWindow(event),
        up: (event) => {
            if (!event) return;
            endScrollbarDrag(event);
            // 展开态透明区单击直接回胶囊；卡片、内容列与滚动条槽不接管点击。
            if (event.type !== "pointerup" || transitionMasked.current || modeRef.current !== "expanded" || cardSorting.current || interactionBusy() || dropActiveRef.current) return;
            const layout = scrollbarFrame.current;
            if (!layout || !inputReady.current) return;
            const target = event.target instanceof Element ? event.target : undefined;
            const onCard = Boolean(target?.closest(".island-card"));
            if (islandExpandedGraceActive(expandedGrace.current, performance.now(), { x: event.clientX, y: event.clientY })) return; // 触发展开的那次 pointerup 不算空白单击。
            if (islandExpandedBlank(layout, { x: event.clientX, y: event.clientY }, onCard)) {
                cancelExpandedReturn(true);
                changeMode("collapsed", false);
            }
        },
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
            stopHoverTimer(); resetExpandedDwell(); endScrollbarDrag(); cancelCollapseWait();
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
                changeMode(islandAfterDrag(releasedInside), false);
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
        lastPointerPoint.current = { x: event.clientX, y: event.clientY };
        lastPointerScreen.current = { x: event.screenX, y: event.screenY };
        if (transitionMasked.current) return; // 遮蔽期间不响应卡片/空白点击。
        const target = event.target as Element;
        if (modeRef.current === "expanded" && target.closest(".island-card")) cancelExpandedReturn(true);
        // 点击候选独立于拖拽：卡片虽不可拖窗口，仍要能由单击触发展开；控件不参与。
        const control = target.closest("button, a, input, textarea, select, [role='button'], [data-island-card-handle]");
        tapCandidate.current = event.button === 0 && !control ? { id: event.pointerId, x: event.screenX, y: event.screenY, mode: modeRef.current } : undefined;
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
        lastPointerPoint.current = { x: event.clientX, y: event.clientY };
        lastPointerScreen.current = { x: event.screenX, y: event.screenY };
        if (cardSorting.current) return; // 拖卡期间不评估透明边距回退。
        const candidate = gesture.current;
        if (!candidate || candidate.id !== event.pointerId || !islandDragMoved(candidate, { x: event.screenX, y: event.screenY })) return;
        cancelDragHold(); // 先移动取消长按拖动，不再按位移升级。
    };
    const handlePointerUp = (event: PointerEvent<HTMLElement>) => {
        lastPointerPoint.current = { x: event.clientX, y: event.clientY };
        lastPointerScreen.current = { x: event.screenX, y: event.screenY };
        if (cardSorting.current || (event.target instanceof Element && event.target.closest("[data-island-card-handle]"))) return;
        const tap = tapCandidate.current;
        const candidate = gesture.current;
        tapCandidate.current = undefined;
        cancelDragHold();
        gesture.current = undefined;
        if (!candidate && (!tap || tap.id !== event.pointerId)) return;
        if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
        const hit = document.elementFromPoint(event.clientX, event.clientY);
        pointerInside.current = Boolean(hit && root.current?.contains(hit));
        const moved = tap ? islandDragMoved(tap, { x: event.screenX, y: event.screenY }) : candidate ? islandDragMoved(candidate, { x: event.screenX, y: event.screenY }) : true;
        const onControl = hit instanceof Element && Boolean(hit.closest("button, a, input, textarea, select, [role='button'], [data-island-card-handle]"));
        const canExpand = event.type === "pointerup" && tap && tap.id === event.pointerId && !suppressClick.current && !moved && hit && root.current?.contains(hit) && !onControl;
        if (canExpand) {
            stopHoverTimer();
            expandedGrace.current = beginIslandExpandedGrace(performance.now(), { x: event.clientX, y: event.clientY });
            changeMode("expanded", false);
        } else scheduleHover(pointerInside.current);
    };
    const handleReorder = async (from: string, to: string) => {
        if (sorting.current) return;
        const order = preferences?.islandCardOrder ?? instances.map((instance) => instance.componentId); // 完整保存顺序，含消息槽。
        const next = moveIslandVisibleCard(order, from, to);
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
    /** 结束卡头排序手势；取消与释放共用，确保捕获、定时器与回退门闩都复位。 */
    const endCardSort = () => {
        const gesture = cardSortGesture.current;
        cardSortGesture.current = undefined;
        if (gesture) {
            clearTimeout(gesture.timer);
            if (gesture.button.isConnected && gesture.button.hasPointerCapture(gesture.pointerId)) gesture.button.releasePointerCapture(gesture.pointerId);
        }
        cardSorting.current = false;
        setSortingCard(undefined);
        setDropCard(undefined);
        suppressClick.current = false;
        cancelExpandedReturn(true); // 排序结束/取消后清掉残留计时，回到正常空白判定。
    };
    /** 按实时坐标算落点，避免释放时用到上一帧状态；列表外返回 undefined 回滚。 */
    const cardDropAt = (id: string, x: number, y: number) => {
        const cards = Array.from(root.current?.querySelectorAll<HTMLElement>("[data-island-card-id]") ?? []).map((card) => {
            const rect = card.getBoundingClientRect();
            return { id: card.dataset.islandCardId ?? "", left: rect.left, top: rect.top, width: rect.width, height: rect.height };
        }).filter((card) => card.id);
        const bounds = cards.length ? {
            left: Math.min(...cards.map((card) => card.left)),
            top: Math.min(...cards.map((card) => card.top)),
            width: Math.max(...cards.map((card) => card.left + card.width)) - Math.min(...cards.map((card) => card.left)),
            height: Math.max(...cards.map((card) => card.top + card.height)) - Math.min(...cards.map((card) => card.top)),
        } : undefined;
        return islandCardDropTarget(cards, { x, y }, id, bounds);
    };
    /** 卡头按下：先等 400ms 长按，提前移动超过 4 DIP 取消；短点只聚焦。 */
    const handleCardHandlePointerDown = (id: string, event: React.PointerEvent<HTMLButtonElement>) => {
        if (transitionMasked.current) return; // 遮蔽期间不开始拖排。
        if (event.button !== 0 || mode !== "expanded" || (foldStates[id]?.phase ?? "idle") !== "idle") return; // 折叠在途不叠加拖卡。
        event.preventDefault();
        event.stopPropagation();
        cancelExpandedReturn(true); // 拖卡前取消仍在途的透明边距自动收起。
        endCardSort();
        const button = event.currentTarget;
        button.setPointerCapture(event.pointerId);
        cardSortGesture.current = {
            id,
            x: event.clientX,
            y: event.clientY,
            pointerId: event.pointerId,
            button,
            timer: setTimeout(() => {
                if (!cardSortGesture.current || cardSortGesture.current.id !== id) return;
                cardSorting.current = true;
                suppressClick.current = true;
                pointerInside.current = true;
                setSortingCard(id);
            }, 400),
        };
    };
    /** 拖卡移动：未激活时按位移取消长按；激活后更新落点预览。 */
    const handleCardHandlePointerMove = (id: string, event: React.PointerEvent<HTMLButtonElement>) => {
        const gesture = cardSortGesture.current;
        if (!gesture || gesture.id !== id) return;
        event.stopPropagation();
        if (!cardSorting.current) {
            if (Math.hypot(event.clientX - gesture.x, event.clientY - gesture.y) > 4) endCardSort();
            return;
        }
        setDropCard(cardDropAt(id, event.clientX, event.clientY));
    };
    /** 释放卡头：按最终落点提交一次完整顺序；取消则不写偏好。 */
    const handleCardHandlePointerUp = (id: string, event: React.PointerEvent<HTMLButtonElement>) => {
        const gesture = cardSortGesture.current;
        if (!gesture || gesture.id !== id) return;
        event.preventDefault();
        event.stopPropagation();
        if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
        const active = cardSorting.current;
        const target = active ? cardDropAt(id, event.clientX, event.clientY) : undefined;
        endCardSort();
        if (!active) event.currentTarget.focus(); // 短点只聚焦句柄，不进入排序。
        if (active && target && target !== id) void handleReorder(id, target);
    };
    /** 失去捕获或指针取消：回滚到已保存顺序，不提交。 */
    const handleCardHandleCancel = (_id: string, event: React.PointerEvent<HTMLButtonElement>) => {
        if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
        endCardSort();
    };
    /** 卡头键盘替代：上下键移动，复用与设置页相同的完整顺序。 */
    const handleCardHandleKeyDown = (id: string, event: React.KeyboardEvent<HTMLButtonElement>) => {
        if (transitionMasked.current) return;
        if ((foldStates[id]?.phase ?? "idle") !== "idle") return; // 折叠在途不叠加键盘排序。
        if (event.key !== "ArrowUp" && event.key !== "ArrowDown") return;
        const index = sortableIds.indexOf(id);
        const target = sortableIds[index + (event.key === "ArrowUp" ? -1 : 1)];
        const to = target && islandSortTarget(mode, sortableIds, id, target, false);
        if (!to) return;
        event.preventDefault();
        event.stopPropagation();
        void handleReorder(id, to);
    };
    /** 关闭一条消息：先本地移除避免闪烁，后端走与原生关闭按钮相同的离场路径。 */
    const handleDismissMessage = (id: number) => {
        messagesRevision.current++;
        notifications.dismiss(`native:${id}`);
        heldMessages.current = heldMessages.current.filter((message) => message.id !== id);
        setMessages((current) => current.filter((message) => message.id !== id));
        void dismissIslandMessage(id).catch((error: unknown) => {
            console.warn("岛消息关闭失败", error);
            const revision = messagesRevision.current; // 补偿读取前记录代次。
            void getIslandMessages()
                .then((next) => {
                    if (messagesRevision.current === revision) {
                        notifications.allowRetry(id);
                        setMessages(next); // 期间有新广播时以广播为准。
                    }
                })
                .catch(() => undefined); // 消息仍在队列时恢复展示，避免本地与后端分叉。
        });
    };
    /** 展开卡的键盘关闭在 DOM 更新后才续焦，鼠标与 hover 关闭仍走普通路径。 */
    const handleDismissPanelMessage: IslandMessageDismiss = (id, keyboardOrigin) => {
        if (keyboardOrigin) messageFocus.current = { origin: keyboardOrigin, candidates: islandMessageFocusOrder(visibleMessages.map((message) => message.id), id) };
        handleDismissMessage(id);
    };
    /** 消息通知卡槽：跟随 islandCardOrder 的位置；无消息且无通道故障时不画空卡，但顺序槽保留。 */
    const renderMessageCard = () => {
        if (!islandMessageCardEnabled(preferences)) return null;
        if (!visibleMessages.length && !messageFailure) return null;
        return (
            <section
                className="island-card"
                aria-label="消息通知"
                data-island-message="true"
                data-island-card-id={ISLAND_MESSAGE_CARD_ID}
                data-sorting={sortingCard === ISLAND_MESSAGE_CARD_ID ? "true" : undefined}
                data-drop-target={dropCard === ISLAND_MESSAGE_CARD_ID ? "true" : undefined}
                data-fold-pending={(foldStates[ISLAND_MESSAGE_CARD_ID]?.phase ?? "idle") !== "idle" ? "true" : undefined}
                tabIndex={0}
                aria-description="聚焦卡片后使用上下方向键调整顺序"
                onKeyDown={(event) => {
                    if (event.target !== event.currentTarget) return;
                    const index = sortableIds.indexOf(ISLAND_MESSAGE_CARD_ID);
                    const target = sortableIds[index + (event.key === "ArrowUp" ? -1 : event.key === "ArrowDown" ? 1 : 0)];
                    const to = target && islandSortTarget(mode, sortableIds, ISLAND_MESSAGE_CARD_ID, target, false);
                    if (!to) return;
                    event.preventDefault();
                    void handleReorder(ISLAND_MESSAGE_CARD_ID, to);
                }}
            >
                <IslandCardToolbar
                    cardId={ISLAND_MESSAGE_CARD_ID}
                    name="消息通知"
                    collapsed={cardCollapsed(ISLAND_MESSAGE_CARD_ID)}
                    pending={(foldStates[ISLAND_MESSAGE_CARD_ID]?.phase ?? "idle") !== "idle"}
                    sorting={sortingCard === ISLAND_MESSAGE_CARD_ID}
                    onToggle={(card) => {
                        cancelExpandedReturn(true);
                        handleToggleCard(ISLAND_MESSAGE_CARD_ID, card);
                    }}
                    onHandlePointerDown={(event) => handleCardHandlePointerDown(ISLAND_MESSAGE_CARD_ID, event)}
                    onHandlePointerMove={(event) => handleCardHandlePointerMove(ISLAND_MESSAGE_CARD_ID, event)}
                    onHandlePointerUp={(event) => handleCardHandlePointerUp(ISLAND_MESSAGE_CARD_ID, event)}
                    onHandleCancel={(event) => handleCardHandleCancel(ISLAND_MESSAGE_CARD_ID, event)}
                    onHandleKeyDown={(event) => handleCardHandleKeyDown(ISLAND_MESSAGE_CARD_ID, event)}
                />
                <div className="island-card__content">
                    {visibleMessages.length ? (
                        <IslandMessagePanel
                            messages={visibleMessages}
                            collapsed={cardCollapsed(ISLAND_MESSAGE_CARD_ID)}
                            onDismiss={handleDismissPanelMessage}
                        />
                    ) : <p role="status">消息通道不可用</p>}
                </div>
            </section>
        );
    };
    // 消息卡在已启用卡片里的插入下标：只数排它前面的实例卡。
    const messageSlotIndex = messageSlotIndexFor(preferences?.islandCardOrder ?? [], instances.map((instance) => instance.componentId));
    const hoverNow = Date.now(); // 同一帧的可见内容和所有测宽样本使用同一时刻。
    /** 通知逐条关闭：本地通知不调用原生离场命令。 */
    const handleDismissNotification = (notice: IslandNotification) => {
        if (notice.nativeId === undefined) notifications.dismiss(notice.key);
        else handleDismissMessage(notice.nativeId);
    };
    /** 声明式导航：插件只声明去向，宿主校验并打开；失败留在页内状态行。 */
    const openNavigationTarget = (componentId: string | undefined, uri: string | undefined) => {
        const destination = islandNavigationUri(uri); // 前端先按宿主白名单归一，非法地址不发命令。
        if (!componentId || !destination) {
            setNavigationFeedback("无法打开链接：地址不在允许范围内。");
            return;
        }
        const request = ++navigationRequest.current;
        setNavigationFeedback(undefined);
        void openIslandNavigation(openPluginUri, componentId, destination).then((result) => {
            if (request !== navigationRequest.current) return; // 迟到的打开结果不覆盖新状态。
            setNavigationFeedback(result.ok ? undefined : result.message);
        });
    };
    /** 可见详情与镜像共用布局；镜像不加载媒体图、挂载插件或执行动作。 */
    const renderHoverDetail = (anchor: IslandAnchor | undefined, measuring = false) => {
        const body = islandHoverBody(anchor);
        if (!body.expandable) return null;
        const plugin = !measuring && body.provider === "plugin" ? instances.find((instance) => snapshot?.runtime.manifests.find((manifest) => manifest.id === instance.componentId)?.name === anchor?.source) : undefined;
        const manifest = plugin && snapshot?.runtime.manifests.find((item) => item.id === plugin.componentId);
        const secondary = islandAnchorValue(body.lines[1] ?? "") ? body.lines[1] : undefined;
        const bell = anchor?.id === ISLAND_NOTIFICATION_ANCHOR_ID;
        return <div key={measuring ? anchor?.id : undefined} className={measuring ? "island-hover-width__sample" : "island-anchor-detail"}
            ref={measuring ? undefined : hoverContentRef} tabIndex={measuring ? undefined : -1} role={measuring ? undefined : "group"} aria-label={measuring ? undefined : bell ? "消息通知内容" : `${anchor?.source}内容`}>
            {!measuring && navigationFeedback ? <p className="island-navigation-feedback" role="status">{navigationFeedback}</p> : null}
            {body.provider === "music" ? <div className="island-music__header island-music-hover">
                <IslandMediaInk artwork={measuring ? undefined : media?.artworkUrl} hover />
                <div className="island-music__metadata">
                    <span className="island-music__title island-anchor__text">{media?.title || body.lines[0]}</span>
                    <span className="island-music__artist island-anchor__text">{media?.artist}</span>
                    <span className="island-music__source island-anchor__text">{media?.source}</span>
                </div>
                <IslandMusicControls media={media} />
            </div> : bell ? <IslandNotificationList rows={notifications.rows.slice(0, ISLAND_MESSAGE_LIMIT)} measuring={measuring} hoverNow={hoverNow} animationsEnabled={animationsEnabled} onDismiss={measuring ? undefined : handleDismissNotification} onOpen={openNavigationTarget} /> : <>
                <span className="island-anchor island-anchor--detail" aria-hidden="true">{measuring ? null : <IslandAnchorVisual anchor={anchor} specs={statusSpecs} special={anchor === dropAnchor ? "release" : undefined} motion={animationsEnabled} artwork={media?.artworkUrl} />}</span>
                {manifest && plugin && hoverPluginInstance?.instanceId === plugin.instanceId ? <div className="island-plugin-hover" data-island-no-drag="true">
                    <PluginSlot manifest={manifest} instanceId={plugin.instanceId} tileType={pluginTileType(plugin.state)} collapsed={false} capabilities={snapshot?.runtime.instances.find((item) => item.instanceId === plugin.instanceId)?.authorizedCapabilities ?? manifest.requiredCapabilities} onIslandSummary={summaryCallback(plugin.componentId)} hover fallback={<span className="island-anchor__text">{islandShortText(body.lines[0] ?? "")}</span>} />
                </div> : <div className="island-anchor-detail__lines">
                    <span className="island-anchor__text"><ScrollingText text={body.lines[0] ?? ""} title={body.lines[0] ?? ""} enabled={!measuring && animationsEnabled} /></span>
                    {secondary ? <span className="island-anchor__text"><ScrollingText text={secondary} title={secondary} enabled={!measuring && animationsEnabled} /></span> : null}
                </div>}
            </>}
        </div>;
    };
    /** 通知计数独立于通用附加项白名单，不挤占 bell 的绘制区。 */
    const notificationCount = (anchor: IslandAnchor | undefined) => anchor?.id === ISLAND_NOTIFICATION_ANCHOR_ID ? islandNotificationCountLabel(anchor.count ?? 0) : undefined;
    return (
        <main
            ref={root}
            id="island-scroll-content"
            className="island"
            style={{ maxBlockSize: work?.height, "--island-work-block": work ? `${work.height}px` : undefined, "--island-anchor-size": `${ISLAND_ANCHOR_SIZE}px`, "--island-capsule-line-max": `${islandCapsuleLineLimit(vertical, media?.playback === "playing")}px`, "--island-anchor-gap": `${ISLAND_ANCHOR_GAP}px`, "--island-anchor-grow": `${ISLAND_ANCHOR_GROW_MS}ms`, "--island-anchor-shrink": `${ISLAND_ANCHOR_SHRINK_MS}ms`, "--island-anchor-easing": ISLAND_ANCHOR_EASING } as CSSProperties}
            data-island-page="true"
            data-dragging={dragActive ? "true" : undefined}
            data-form={preferences?.islandForm ?? "capsule"}
            data-edge={edge || "top"}
            data-state={mode}
            data-animations={animationsEnabled ? "true" : undefined}
            data-vertical={vertical}
            data-scrollbar={edge === "right" ? "start" : "end"}
            data-fillets={preferences?.islandForm === "notch" && mode !== "expanded" ? "true" : undefined}
            // 浮岛没有自有右键菜单；默认浏览器菜单属于 WebView2 泄漏，统一吞掉。
            onContextMenu={(event) => event.preventDefault()}
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
                islandTrace("keydown", { key: event.key, root: event.target === event.currentTarget, masked: transitionMasked.current, busy: interactionBusy(), drop: dropActiveRef.current, prevented: event.defaultPrevented }); // gated 诊断：复验切态期间按键被哪道闸门拦下。
                if (transitionMasked.current) { if (event.key === "Escape") pendingEscape.current = true; return; } // 遮蔽期间整层不可交互；Esc 记下来，解除后补收。
                if (event.defaultPrevented || interactionBusy() || dropActiveRef.current) return;
                stopHoverTimer();
                if (event.key === "Escape") {
                    event.preventDefault();
                    if (cardSorting.current) { endCardSort(); return; } // Esc 先取消排序并回滚，不再顺带收胶囊。
                    changeMode("collapsed", false);
                }
                else if (event.key === "Enter" && event.target === event.currentTarget) {
                    expandedGrace.current = beginIslandExpandedGrace(performance.now());
                    changeMode("expanded", false);
                }
            }}
        >
            <div ref={palette} className="fence-workspace island__palette" hidden aria-hidden="true" />
            {/* 常驻播报节点：消息卡随内容重建，直接挂 aria-live 时常见读屏器不播报。 */}
            <p className="island__announcer" role="status" aria-live="polite">
                {liveText}
                {mode === "collapsed" && navigationFeedback ? ` ${navigationFeedback}` : ""}
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
            {/* 常态与 hover 也挂载插件卡：隐藏容器不用 display:none，插件可继续轮询并投递提醒；展开态复用同一实例。 */}
            {mode !== "expanded" && islandRunning ? (
                <div className="island-plugin-runtime" aria-hidden="true" inert>
                    {pluginCards.map((instance) => {
                        if (pluginSites.get(instance.instanceId) !== "hidden") return null; // hover/展开的实例在这里不重复挂。
                        const manifest = snapshot?.runtime.manifests.find((item) => item.id === instance.componentId);
                        const registration = snapshot?.runtime.instances.find((item) => item.instanceId === instance.instanceId);
                        if (!manifest || !registration) return null;
                        return (
                            <div className="island-plugin-runtime__host" key={instance.instanceId} data-island-plugin="true">
                                <PluginSlot
                                    manifest={manifest}
                                    instanceId={instance.instanceId}
                                    tileType={pluginTileType(instance.state)}
                                    collapsed
                                    capabilities={registration.authorizedCapabilities ?? manifest.requiredCapabilities}
                                    onIslandSummary={summaryCallback(instance.componentId)}
                                />
                            </div>
                        );
                    })}
                </div>
            ) : null}
            {mode !== "expanded" ? (
                <section className="island-card island-card--anchors" aria-label={mode === "collapsed" ? "浮岛概览" : "浮岛摘要"}>
                    <div className="island-card__content">
                        {mode === "collapsed" ? (
                            <div className="island-anchor-row" data-layout="capsule">
                                {face.kind === "clock" ? <div className="island-anchor-item"><span className="island-anchor" data-anchor="clock"><IslandAnchorVisual special="clock" motion={false} /></span><span className="island-anchor__text island-anchor__clock">{face.text}</span></div> : <>
                                    {face.visible.map((anchor) => {
                                        const active = capsuleActive?.id === anchor.id ? anchor : undefined;
                                        const musicAnchor = isMusic(anchor);
                                        const lyric = musicAnchor && musicView.hasLine && anchor.unit?.primary === musicView.line;
                                        const inserting = contentMotion && previousIds.length > 0 && !previousIds.includes(anchor.id) && !drop;
                                        const navigable = islandNavigationUri(anchor.uri) !== undefined && anchor.componentId !== undefined; // 声明了去向才可点。
                                        return <div className="island-anchor-item" key={anchor.id}>
                                            <span
                                                className="island-anchor"
                                                data-anchor={anchor.id}
                                                data-inserting={inserting || undefined}
                                                data-island-no-drag={navigable ? "true" : undefined}
                                                role={navigable ? "button" : undefined}
                                                tabIndex={navigable ? 0 : undefined}
                                                aria-label={navigable ? islandNavigationLabel(anchor.source, anchor.uri!) : notificationCount(anchor) ? `消息通知 ${notificationCount(anchor)} 条` : undefined}
                                                onClick={navigable ? (event) => { event.stopPropagation(); openNavigationTarget(anchor.componentId, anchor.uri); } : undefined}
                                                onKeyDown={navigable ? (event) => {
                                                    if (event.key !== "Enter" && event.key !== " ") return;
                                                    event.preventDefault();
                                                    event.stopPropagation();
                                                    if (event.repeat) return; // 已阻断默认与冒泡，长按只触发一次打开。
                                                    openNavigationTarget(anchor.componentId, anchor.uri);
                                                } : undefined}
                                            ><IslandAnchorVisual anchor={anchor} specs={statusSpecs} special={anchor === dropAnchor ? "release" : undefined} motion={contentMotion} entering={inserting} artwork={media?.artworkUrl} messageTone={anchor.id === ISLAND_NOTIFICATION_ANCHOR_ID ? islandNotificationTone(notifications.rows[0]) : undefined} messageGlyph={anchor.id === ISLAND_NOTIFICATION_ANCHOR_ID ? capsuleMessageGlyph : undefined} /></span>
                                            <IslandRelayContent className="island-anchor-label" anchor={active} motion={contentMotion && !drop} vertical={vertical} music={musicAnchor} handoff={active?.id === ISLAND_NOTIFICATION_ANCHOR_ID ? messageHandoff : undefined} growDelay={Math.max(relayFrom && relayFrom !== active?.id ? ISLAND_ANCHOR_SHRINK_MS : 0, inserting ? ISLAND_ANCHOR_GROW_MS : 0)}>
                                                {/* 音乐进度仍供播报使用，胶囊只显示歌词或回落标题。 */}
                                                {active?.unit ? lyric ? <IslandCapsuleLyric music={music} position={position} motion={contentMotion} vertical={vertical} /> : <span className="island-anchor__text">{islandShortText(active.unit.primary)}{notificationCount(active) ? <span className="island-anchor__value island-notification-count">{notificationCount(active)}</span> : !musicAnchor && islandAnchorValue(active.unit.secondary ?? "") ? <span className="island-anchor__value">{active.unit.secondary}</span> : null}</span> : null}
                                            </IslandRelayContent>
                                        </div>;
                                    })}
                                    {face.overflow ? <div className="island-anchor-item" key="overflow">
                                        <span className="island-anchor" data-anchor="overflow"><IslandAnchorVisual special={`＋${face.overflow > 99 ? "99+" : face.overflow}`} motion={false} /></span>
                                        {capsuleActive?.unit && !face.visible.some((anchor) => anchor.id === capsuleActive.id) ? <span className="island-anchor__text">{islandShortText(capsuleActive.unit.primary)}{notificationCount(capsuleActive) ? <span className="island-anchor__value island-notification-count">{notificationCount(capsuleActive)}</span> : null}</span> : null}
                                    </div> : null}
                                </>}
                            </div>
                        ) : <>
                            <div ref={hoverList} className="island-anchor-row" data-layout="hover" role="group" aria-label="浮岛锚点" onKeyDown={handleHoverKey}
                                onFocus={(event) => { if (event.target instanceof HTMLButtonElement) focusedAnchor.current = event.target; }}
                                onBlur={(event) => {
                                    const target = focusedAnchor.current;
                                    if (!target) return;
                                    if (event.relatedTarget) focusedAnchor.current = null;
                                    else queueMicrotask(() => {
                                        // 浏览器会在真正摘除焦点节点前发 blur，暂留引用供本轮布局接续键盘焦点。
                                        if (focusedAnchor.current === target && target.isConnected && document.activeElement !== target) focusedAnchor.current = null;
                                    });
                                }}>
                                {anchors.length ? anchors.map((anchor) => {
                                    const navigable = islandNavigationUri(anchor.uri) !== undefined && anchor.componentId !== undefined; // 声明了去向才可点。
                                    return <button
                                    type="button" className="island-anchor" data-anchor={anchor.id} key={anchor.id} tabIndex={anchor.id === selected?.id ? 0 : -1}
                                    aria-label={navigable ? islandNavigationLabel(anchor.source, anchor.uri!) : notificationCount(anchor) ? `消息通知 ${notificationCount(anchor)} 条` : `${anchor.source} ${islandStatusSpec(anchor.unit?.status ?? "", statusSpecs)?.word ?? ""}`.trim()} aria-pressed={anchor.id === selected?.id}
                                    onPointerEnter={(event) => { if (event.pointerType !== "touch" && !drop) content.setSelected(anchor.id); }}
                                    onClick={(event) => {
                                        if (drop) return;
                                        if (navigable) {
                                            event.stopPropagation();
                                            openNavigationTarget(anchor.componentId, anchor.uri);
                                            return;
                                        }
                                        content.setSelected(anchor.id);
                                    }}
                                    onKeyDown={navigable ? (event) => {
                                        if (event.key !== "Enter" && event.key !== " ") return;
                                        event.preventDefault(); // 抢在父级 Enter 焦点导航之前。
                                        event.stopPropagation();
                                        if (event.repeat) return; // 已阻断默认与冒泡，长按只触发一次打开。
                                        openNavigationTarget(anchor.componentId, anchor.uri);
                                    } : undefined}
                                >{anchor === dropAnchor ? <IslandAnchorVisual anchor={anchor} special="release" motion={contentMotion} /> : islandAnchorKind(anchor) === "plugin" ? <PluginIdentityIcon iconPath={anchor.iconPath} /> : <IslandFunctionIcon kind={islandAnchorKind(anchor)} />}</button>;
                                }) : <div className="island-anchor-item"><span className="island-anchor" data-anchor="clock"><IslandAnchorVisual special="clock" motion={false} /></span><span className="island-anchor__text island-anchor__clock">{islandClockLabel(clockNow)}</span></div>}
                            </div>
                            <IslandHoverRelay anchors={anchors} selected={selected} motion={animationsEnabled && !drop} renderDetail={renderHoverDetail} />
                        </>}
                    </div>
                    {mode === "hover" ? <div ref={hoverWidthSamples} className="island-hover-width" inert aria-hidden="true">{anchors.map((anchor) => renderHoverDetail(anchor, true))}</div> : null}
                </section>
            ) : visibleMessages.length || messageFailure || instances.length || expandedContent.length ? (
                <>
                    {navigationFeedback ? <p className="island-navigation-feedback" role="status">{navigationFeedback}</p> : null}
                    {instances.map((instance, index) => {
                    const manifest = snapshot!.runtime.manifests.find((item) => item.id === instance.componentId)!;
                    const registration = snapshot!.runtime.instances.find((item) => item.instanceId === instance.instanceId);
                    const summary = pluginSummaries[instance.componentId];
                    const cardStatus = islandPluginAnchorStatus(
                        summary?.status,
                        summary?.text,
                        registration?.status === "failed" ? "failed" : registration?.status === "ready" ? "pending" : "disconnected",
                    );
                    const actionUri = summary?.navigation?.uri;
                    const action = instance.componentId === "plugin.dsh-status" ? {
                        label: "打开 DSH",
                        disabled: islandPaused || cardStatus === "failed" || !islandNavigationUri(actionUri),
                        onClick: () => openNavigationTarget(instance.componentId, actionUri),
                    } : undefined; // 只有 DSH 卡先提供安全动作，能力/宿主不可用时禁用。
                    return (
                        <Fragment key={instance.instanceId}>
                        {index === messageSlotIndex ? renderMessageCard() : null}
                        <section
                            className="island-card"
                            key={instance.instanceId}
                            aria-label={manifest.name}
                            data-island-card-id={instance.componentId}
                            data-sorting={sortingCard === instance.componentId ? "true" : undefined}
                            data-drop-target={dropCard === instance.componentId ? "true" : undefined}
                            data-fold-pending={(foldStates[instance.componentId]?.phase ?? "idle") !== "idle" ? "true" : undefined}
                            tabIndex={0}
                            aria-description="聚焦卡片后使用上下方向键调整顺序"
                            onKeyDown={(event) => {
                                if (event.target !== event.currentTarget) return;
                                const index = sortableIds.indexOf(instance.componentId);
                                const target = sortableIds[index + (event.key === "ArrowUp" ? -1 : event.key === "ArrowDown" ? 1 : 0)];
                                const to = target && islandSortTarget(mode, sortableIds, instance.componentId, target, false);
                                if (to) {
                                    event.preventDefault();
                                    void handleReorder(instance.componentId, to);
                                }
                            }}
                        >
                            <IslandCardToolbar
                                cardId={instance.componentId}
                                name={manifest.name}
                                collapsed={cardCollapsed(instance.componentId)}
                                pending={(foldStates[instance.componentId]?.phase ?? "idle") !== "idle"}
                                action={action}
                                sorting={sortingCard === instance.componentId}
                                onToggle={(card) => {
                                    cancelExpandedReturn(true); // 键盘激活也取消仍在途的自动收起。
                                    handleToggleCard(instance.componentId, card);
                                }}
                                onHandlePointerDown={(event) => handleCardHandlePointerDown(instance.componentId, event)}
                                onHandlePointerMove={(event) => handleCardHandlePointerMove(instance.componentId, event)}
                                onHandlePointerUp={(event) => handleCardHandlePointerUp(instance.componentId, event)}
                                onHandleCancel={(event) => handleCardHandleCancel(instance.componentId, event)}
                                onHandleKeyDown={(event) => handleCardHandleKeyDown(instance.componentId, event)}
                            />
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
                                    pluginSites.get(instance.instanceId) === "expanded" ? (
                                        <div className="island-plugin-card" data-island-plugin="true">
                                            <span className="island-card__plugin-identity" aria-hidden="true"><PluginIdentityMark status={cardStatus} iconPath={manifest.iconPath} size={16} /></span>
                                            <div className="island-plugin-card__body">
                                                <PluginSlot
                                                    manifest={manifest}
                                                    instanceId={instance.instanceId}
                                                    tileType={pluginTileType(instance.state)}
                                                    collapsed={cardCollapsed(instance.componentId)}
                                                    capabilities={registration?.authorizedCapabilities ?? manifest.requiredCapabilities}
                                                    onIslandSummary={summaryCallback(instance.componentId)}
                                                />
                                            </div>
                                        </div>
                                    ) : null
                                ) : (
                                    <p role="status">{manifest.name} · 组件入口不可用</p>
                                )}
                            </div>
                        </section>
                        </Fragment>
                    );
                    })}
                    {messageSlotIndex === instances.length ? renderMessageCard() : null}
                    {expandedContent.map((anchor) => <section className="island-card" key={anchor.id} aria-label={anchor.source}>
                        <div className="island-card__content island-anchor-expanded">
                            <span className="island-anchor" data-anchor={anchor.id}><IslandAnchorVisual anchor={anchor} specs={statusSpecs} motion={contentMotion} /></span>
                            {anchor.unit?.primary}
                        </div>
                    </section>)}
                </>
            ) : (
                <section className="island-card">
                    <div className="island-card__content">
                        <p role="status">{failure ? "岛加载失败" : snapshot ? "暂无岛卡" : "正在加载"}</p>
                    </div>
                </section>
            )}
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
