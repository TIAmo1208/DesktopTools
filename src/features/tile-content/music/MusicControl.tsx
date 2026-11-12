import { Fragment, useEffect, useLayoutEffect, useRef, useState, type CSSProperties } from "react";
import type { LyricLine, MusicTimeline } from "../../../contracts/music";
import { lyricIndex, lyricTickDelay, positionAt } from "../../music/musicState";
import { usePreferences } from "../../settings/PreferencesProvider";
import { isCompactMusicLayout, musicMediaMinimum } from "./musicLayout";
import { musicLyricsLineRows, musicLyricsPlacement, musicLyricsRowCount, musicLyricsWindow, type MusicLyricsPlacement } from "./musicLyricsLayout";
import { MusicSpectrum } from "./MusicSpectrum";
import previousIcon from "../../../../resources/music-skip-back.svg?raw";
import nextIcon from "../../../../resources/music-skip-forward.svg?raw";
import playIcon from "../../../../resources/music-play.svg?raw";
import pauseIcon from "../../../../resources/music-pause.svg?raw";
import "./MusicControl.css";

/** 音乐控制组件可呈现的媒体快照。 */
export interface MusicMediaSnapshot {
    title?: string;
    artist?: string;
    source?: string;
    artworkUrl?: string;
    playback: "playing" | "paused" | "stopped" | "unknown";
    canPrevious: boolean;
    canPlayPause: boolean;
    canNext: boolean;
}

/** 音乐控制组件依赖的外部媒体动作。 */
export interface MusicControlActions {
    previous: () => Promise<void>;
    playPause: () => Promise<void>;
    next: () => Promise<void>;
}

/** 音乐组件所需的内容尺寸，不包含宿主标题栏和外壳。 */
export interface MusicControlMinimumSize {
    width: number; // 内容最小宽度，单位为 CSS px。
    height: number; // 内容最小高度，单位为 CSS px。
}

export const MUSIC_CONTROL_MINIMUM_SIZE: MusicControlMinimumSize = { width: 168, height: 94 }; // 首次测量前的组件内容基线。

/** 呈现媒体并上报内部最小尺寸，宿主无需读取内部 DOM。 */
export const MusicControl = ({
    snapshot,
    lyrics = [],
    timeline,
    anchorNow,
    actions,
    onFailure,
    onMinimumSizeChange,
}: {
    snapshot?: MusicMediaSnapshot;
    lyrics?: readonly LyricLine[]; // 共享音乐源的逐句歌词。
    timeline?: MusicTimeline; // 与歌词同源的时间轴。
    anchorNow: number; // 时间轴投影的本地单调锚点。
    actions: MusicControlActions;
    onFailure: (message: string) => void;
    onMinimumSizeChange?: (minimum: MusicControlMinimumSize) => void; // 仅在有效最小内容尺寸变化时通知宿主。
}) => {
    const root = useRef<HTMLElement>(null); // 布局测量只读取外部已经分配的内容空间。
    const artwork = useRef<HTMLDivElement>(null); // 同一个封面节点在两种布局中复用。
    const body = useRef<HTMLDivElement>(null); // 信息和按钮保持节点身份及键盘焦点。
    const metadata = useRef<HTMLDivElement>(null); // 组件私有的信息尺寸测量节点。
    const controls = useRef<HTMLDivElement>(null); // 组件私有的按钮尺寸测量节点。
    const [compact, setCompact] = useState(true); // 首次测量后按内容宽度确定布局。
    const [contentSize, setContentSize] = useState({ width: 0, height: 0 }); // 歌词区只读取宿主分配的内容盒。
    const [mediaMinimum, setMediaMinimum] = useState(MUSIC_CONTROL_MINIMUM_SIZE.height); // 正文媒体区下限，频谱与大字号会抬高。
    const mode = useRef<boolean | undefined>(undefined); // 滞回需要保存上一次布局。
    const previous = useRef<DOMRect[] | undefined>(undefined); // 模式变化前的当前呈现位置。
    const animations = useRef<Animation[]>([]); // 连续拖动时取消旧目标并从当前位置衔接。
    const { animationsEnabled, preferences } = usePreferences(); // 跨窗口共享的动画与字号偏好。
    const [loadedArtwork, setLoadedArtwork] = useState<string>(); // 仅成功加载的封面开启遮罩与白色文字。
    const showSource = preferences?.musicShowSource !== false; // 用户关闭时只隐藏来源名称。
    const spectrumEnabled = preferences?.musicSpectrumEnabled === true; // 默认关闭，避免无播放时占用采集。
    const spectrumBars = preferences?.musicSpectrumBars ?? 32; // 用户选择的频段数。
    const spectrumFps = preferences?.musicSpectrumFps ?? 30; // 用户选择的刷新帧率。
    const spectrumDevice = preferences?.musicSpectrumDevice ?? ""; // 空字符串表示跟随系统默认设备。
    const hasArtwork = Boolean(snapshot?.artworkUrl && loadedArtwork === snapshot.artworkUrl); // 缺失与加载失败保持主题背景。
    const playbackLabel = snapshot?.playback === "playing" ? "正在播放" : snapshot?.playback === "paused" ? "已暂停" : ""; // 播放状态独立于来源显示。
    useLayoutEffect(() => {
        const element = root.current; // 宿主已分配空间的内容根节点。
        const information = metadata.current; // 内部信息固有尺寸。
        const buttons = controls.current; // 内部操作固有尺寸。
        if (!element || !information || !buttons) return;
        let last: MusicControlMinimumSize | undefined; // 当前订阅最近上报的尺寸，避免父子重复更新。
        const handleMinimumSize = () => {
            if (!element.getClientRects().length) return;
            const style = getComputedStyle(element); // 组件独立拥有的内边距和最小尺寸规则。
            const minimum = {
                width: Math.ceil(Math.max(MUSIC_CONTROL_MINIMUM_SIZE.width, parseFloat(style.minWidth) || 0)),
                height: musicMediaMinimum(
                    information.offsetHeight,
                    buttons.offsetHeight,
                    parseFloat(style.paddingTop),
                    parseFloat(style.paddingBottom),
                    spectrumEnabled,
                    compact,
                ),
            }; // offsetHeight 不包含动画变换；频谱外部尺寸按当前布局计入。
            setMediaMinimum(minimum.height); // 下方歌词按真实正文下限让位。
            if (last?.width === minimum.width && last.height === minimum.height) return;
            last = minimum;
            onMinimumSizeChange?.(minimum);
        };
        const observer = new ResizeObserver(handleMinimumSize); // 字号、模式及可见性变化后重新报告内部需求。
        for (const target of [element, information, buttons]) observer.observe(target);
        handleMinimumSize();
        return () => observer.disconnect();
    }, [onMinimumSizeChange, spectrumEnabled, compact]);
    useLayoutEffect(() => {
        const element = root.current; // 当前音乐内容根节点。
        if (!element) return;
        const handleResize = () => {
            if (!element.getClientRects().length) return;
            const style = getComputedStyle(element); // 扣除内边距后的宽度不依赖当前布局。
            const width = element.clientWidth; // 宿主分配的内容盒宽度，含组件自身内边距。
            const height = element.clientHeight; // 频谱开关与内边距不改变该判定盒高度。
            setContentSize((current) => (current.width === width && current.height === height ? current : { width, height })); // 仅在内容盒变化时重算歌词区。
            const next = isCompactMusicLayout(width, parseFloat(style.fontSize), mode.current); // 高度不参与布局选择。
            if (next === mode.current) return;
            if (mode.current !== undefined && artwork.current && body.current)
                previous.current = [artwork.current.getBoundingClientRect(), body.current.getBoundingClientRect()];
            animations.current.forEach((animation) => animation.cancel());
            mode.current = next;
            setCompact(next);
        };
        const observer = new ResizeObserver(handleResize); // 外框实时缩放，内部只在跨阈值时切换模式。
        observer.observe(element);
        if (body.current) observer.observe(body.current);
        handleResize();
        return () => observer.disconnect();
    }, [preferences?.fontSize]);
    useLayoutEffect(() => {
        const before = previous.current; // 使用切换前采样的屏幕位置，避免连续切换跳回初始状态。
        previous.current = undefined;
        if (!animationsEnabled) {
            animations.current.forEach((animation) => animation.cancel());
            return;
        }
        if (!before) return;
        animations.current = [artwork.current!, body.current!].map((element, index) => {
            const after = element.getBoundingClientRect(); // 目标布局的实际尺寸。
            const from = before[index]; // 原位置可能是上一次过渡的中间帧。
            const scale = index === 0 && after.width && after.height ? ` scale(${from.width / after.width}, ${from.height / after.height})` : ""; // 只缩放图片，文字与图标仅移动。
            return element.animate(
                [
                    { transform: `translate(${from.x - after.x}px, ${from.y - after.y}px)${scale}`, transformOrigin: "top left" },
                    { transform: "none", transformOrigin: "top left" },
                ],
                { duration: 180, easing: "ease-out" },
            );
        });
        return () => animations.current.forEach((animation) => animation.cancel());
    }, [compact, animationsEnabled]);
    const placementRef = useRef<MusicLyricsPlacement>("none"); // 滞回判定读取上一次稳定布局。
    const lyricsEnabled = preferences?.musicLyricsEnabled !== false; // 关闭后不显示歌词区，后端同时停止取词。
    const hasLyrics = lyricsEnabled && lyrics.length > 0 && Boolean(timeline); // 取词中、无匹配、失败与无会话都按无词处理。
    const placement = musicLyricsPlacement(contentSize.width, contentSize.height, hasLyrics, placementRef.current); // 先高度后宽度。
    placementRef.current = placement;
    const [renderedPlacement, setRenderedPlacement] = useState<MusicLyricsPlacement>("none"); // 收起动画期间保留旧布局。
    useLayoutEffect(() => {
        if (placement !== "none") {
            setRenderedPlacement(placement); // 布局就绪前切到目标形态，避免一帧旧布局。
            return;
        }
        if (renderedPlacement === "none") return;
        if (!animationsEnabled) {
            setRenderedPlacement("none"); // 关闭动画时直接收起，不保留 180ms 旧布局。
            return;
        }
        const timer = setTimeout(() => setRenderedPlacement("none"), 180); // 与展开共用 180ms，收起后不占位。
        return () => clearTimeout(timer);
    }, [placement, renderedPlacement, animationsEnabled]);
    const leaving = placement === "none" && renderedPlacement !== "none";
    const [now, setNow] = useState(() => performance.now()); // 歌词只按可见播放位置推进。
    const position = timeline ? positionAt({ timeline, anchorNow }, now) : 0; // 与格子、浮岛共用同一投影。
    const songKey = timeline ? `${timeline.sessionId}\u0000${timeline.title}\u0000${timeline.artist}` : ""; // 切歌身份。
    const [shownLyrics, setShownLyrics] = useState<readonly LyricLine[]>(lyrics); // 切歌时旧词先淡出 120ms。
    const [fading, setFading] = useState(false);
    const songKeyRef = useRef(songKey); // 当前歌词所属歌曲。
    useEffect(() => {
        if (songKey === songKeyRef.current) {
            setShownLyrics(lyrics); // 同曲更新逐句替换，行内已有 120ms 过渡。
            setFading(false); // 快速切回原曲时恢复可见，不能停在透明态。
            return;
        }
        if (!animationsEnabled || shownLyrics.length === 0) {
            songKeyRef.current = songKey;
            setShownLyrics(lyrics);
            setFading(false);
            return;
        }
        setFading(true);
        const timer = setTimeout(() => {
            songKeyRef.current = songKey;
            setShownLyrics(lyrics);
            setFading(false);
        }, 120); // 旧词淡出后再换新词；新歌无词时由收起动画收口。
        return () => clearTimeout(timer);
    }, [songKey, lyrics, shownLyrics.length, animationsEnabled]);
    const mediaHeight = Math.max(94, mediaMinimum); // 正文媒体区下限，频谱与大字号会抬高。
    const lyricsHeight = Math.max(0, contentSize.height - mediaHeight - 6); // 下方区高 = 内容高 − 正文下限 − 6。
    const rows = musicLyricsRowCount(renderedPlacement === "bottom" ? lyricsHeight : contentSize.height); // 行数由可用高度决定。
    const index = lyricIndex(shownLyrics, position); // 最后一个已开始的句子。
    const translation = index >= 0 ? shownLyrics[index]?.translation : undefined; // 当前句可选翻译。
    const showTranslation = Boolean(translation) && rows > 1; // 只有能多容纳一行时才占位。
    const view = musicLyricsWindow(shownLyrics, index, musicLyricsLineRows(rows, showTranslation)); // 当前句居中的可见窗口。
    // 换句滚动：整块一次位移，行与行不拆开；当前句的字号与颜色延后 200ms 过渡，其余行立即回落。
    const lyricsRef = useRef<HTMLDivElement>(null);
    const previousLyricIndex = useRef<number | undefined>(undefined);
    useLayoutEffect(() => {
        const element = lyricsRef.current;
        const previous = previousLyricIndex.current;
        previousLyricIndex.current = index; // 布局效应里同步，避免 React 双次渲染把变化判定吃掉。
        if (!element || previous === undefined || previous === index || !animationsEnabled) return;
        const direction = index > previous ? 1 : -1; // 前进向上滚，回退向下滚。
        const rows = Array.from(element.querySelectorAll<HTMLElement>("[data-line-index]"));
        const step = rows.length > 1 ? Math.abs(rows[1].offsetTop - rows[0].offsetTop) : 18; // 一行歌词的实际步进。
        // 已播放行先位移、当前与未播放行延迟 80ms；fill=backwards 让延迟段停在起始位移，不回到原位。
        const animations = rows.flatMap((row) => {
            const played = Number(row.dataset.lineIndex) < index;
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
    }, [index, animationsEnabled]);
    useEffect(() => {
        if (renderedPlacement === "none" || !timeline?.advancing) return;
        let timer: ReturnType<typeof setTimeout> | undefined;
        const refresh = () => {
            const current = performance.now();
            setNow(current);
            const projected = positionAt({ timeline, anchorNow }, current);
            const delay = lyricTickDelay(lyrics, projected, timeline.durationMs); // 轨道末端与超出时长的句子不再调度。
            if (delay !== undefined) timer = setTimeout(refresh, delay);
        };
        refresh();
        return () => clearTimeout(timer);
    }, [renderedPlacement, timeline, anchorNow, lyrics]);
    const [busy, setBusy] = useState(false); // 防止一次媒体命令完成前重复提交。
    const pending = useRef(false); // 同一渲染周期内同步锁住重复点击。
    const empty = !snapshot; // 无会话、加载和失败后的统一空态。
    const run = async (action: () => Promise<void>) => {
        if (pending.current) return;
        pending.current = true;
        setBusy(true);
        try {
            await action();
        } catch (error) {
            onFailure(
                error instanceof Error
                    ? error.message
                    : typeof error === "string"
                      ? error
                      : typeof error === "object" && error && "message" in error
                        ? String(error.message)
                        : "媒体控制失败",
            );
        } finally {
            pending.current = false;
            setBusy(false);
        }
    };
    const button = (label: string, icon: string, enabled: boolean, action: () => Promise<void>) => (
        <button type="button" aria-label={label} title={label} disabled={!enabled || busy} onClick={() => void run(action)}>
            <span aria-hidden="true" dangerouslySetInnerHTML={{ __html: icon }} />
        </button>
    );
    return (
        <section
            ref={root}
            className="music-control"
            data-layout={compact ? "compact" : "wide"}
            data-artwork={hasArtwork ? "ready" : "empty"}
            data-spectrum={spectrumEnabled ? "on" : "off"}
            data-lyrics={renderedPlacement}
            data-lyrics-open={placement === "none" ? "off" : "on"}
            data-animations={animationsEnabled ? "on" : "off"}
            style={{ "--music-media-height": `${mediaHeight}px` } as CSSProperties}
            aria-label="音乐控制"
        >
            <div ref={artwork} className="music-control__artwork" aria-label={snapshot?.title ? `${snapshot.title} 封面` : "空白封面"}>
                {snapshot?.artworkUrl && (
                    <img
                        key={snapshot.artworkUrl}
                        src={snapshot.artworkUrl}
                        alt=""
                        onLoad={() => setLoadedArtwork(snapshot.artworkUrl)}
                        onError={() => setLoadedArtwork(undefined)}
                    />
                )}
            </div>
            <div ref={body} className="music-control__body">
                <div ref={metadata} className="music-control__metadata">
                    <strong title={snapshot?.title}>{empty ? "" : snapshot?.title}</strong>
                    <span title={snapshot?.artist}>{empty ? "" : snapshot?.artist}</span>
                    <small title={showSource ? snapshot?.source : undefined}>
                        {empty ? "" : [showSource ? snapshot?.source : "", playbackLabel].filter(Boolean).join(" · ")}
                    </small>
                </div>
                {spectrumEnabled && (
                    <div className="music-control__spectrum">
                        <MusicSpectrum bars={spectrumBars} fps={spectrumFps} device={spectrumDevice} playing={snapshot?.playback === "playing"} />
                    </div>
                )}
                <div ref={controls} className="music-control__actions">
                    {button("上一曲", previousIcon, Boolean(snapshot?.canPrevious), actions.previous)}
                    {button(
                        snapshot?.playback === "playing" ? "暂停" : "播放",
                        snapshot?.playback === "playing" ? pauseIcon : playIcon,
                        Boolean(snapshot?.canPlayPause),
                        actions.playPause,
                    )}
                    {button("下一曲", nextIcon, Boolean(snapshot?.canNext), actions.next)}
                </div>
            </div>
            {renderedPlacement !== "none" && (
                <div
                    ref={lyricsRef}
                    className="music-control__lyrics"
                    data-placement={renderedPlacement}
                    data-leaving={leaving}
                    data-fading={fading}
                    aria-hidden="true"
                    style={
                        leaving
                            ? { height: 0 }
                            : renderedPlacement === "right"
                              ? { height: contentSize.height }
                              : renderedPlacement === "bottom" && !compact
                                ? { height: lyricsHeight }
                                : undefined
                    }
                >
                    {shownLyrics.slice(view.start, view.end).map((line, offset) => {
                        const current = view.start + offset === view.current; // 当前句加重且更深。
                        return (
                            <Fragment key={line.startMs}>
                                <p data-line-index={view.start + offset} data-current={current} title={line.text}>
                                    {line.text}
                                </p>
                                {current && showTranslation && line.translation ? (
                                    <p data-translation="true" title={line.translation}>
                                        {line.translation}
                                    </p>
                                ) : null}
                            </Fragment>
                        );
                    })}
                </div>
            )}
        </section>
    );
};
