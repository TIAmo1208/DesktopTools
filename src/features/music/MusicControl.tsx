import { useLayoutEffect, useRef, useState } from "react";
import { usePreferences } from "../settings/PreferencesProvider";
import { isCompactMusicLayout } from "./musicLayout";
import previousIcon from "../../../resources/music-skip-back.svg?raw";
import nextIcon from "../../../resources/music-skip-forward.svg?raw";
import playIcon from "../../../resources/music-play.svg?raw";
import pauseIcon from "../../../resources/music-pause.svg?raw";
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
    actions,
    onFailure,
    onMinimumSizeChange,
}: {
    snapshot?: MusicMediaSnapshot;
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
    const mode = useRef<boolean | undefined>(undefined); // 滞回需要保存上一次布局。
    const previous = useRef<DOMRect[] | undefined>(undefined); // 模式变化前的当前呈现位置。
    const animations = useRef<Animation[]>([]); // 连续拖动时取消旧目标并从当前位置衔接。
    const { animationsEnabled, preferences } = usePreferences(); // 跨窗口共享的动画与字号偏好。
    const [loadedArtwork, setLoadedArtwork] = useState<string>(); // 仅成功加载的封面开启遮罩与白色文字。
    const hasArtwork = Boolean(snapshot?.artworkUrl && loadedArtwork === snapshot.artworkUrl); // 缺失与加载失败保持主题背景。
    useLayoutEffect(() => {
        const element = root.current; // 宿主已分配空间的内容根节点。
        const information = metadata.current; // 内部信息固有尺寸。
        const buttons = controls.current; // 内部操作固有尺寸。
        if (!onMinimumSizeChange || !element || !information || !buttons) return;
        let last: MusicControlMinimumSize | undefined; // 当前订阅最近上报的尺寸，避免父子重复更新。
        const handleMinimumSize = () => {
            if (!element.getClientRects().length) return;
            const style = getComputedStyle(element); // 组件独立拥有的内边距和最小尺寸规则。
            const minimum = {
                width: Math.ceil(Math.max(MUSIC_CONTROL_MINIMUM_SIZE.width, parseFloat(style.minWidth) || 0)),
                height: Math.ceil(
                    Math.max(
                        MUSIC_CONTROL_MINIMUM_SIZE.height,
                        information.offsetHeight + buttons.offsetHeight + parseFloat(style.paddingTop) + parseFloat(style.paddingBottom),
                    ),
                ),
            }; // offsetHeight 不包含动画变换；不读取封面或被外框拉伸后的右栏高度。
            if (last?.width === minimum.width && last.height === minimum.height) return;
            last = minimum;
            onMinimumSizeChange(minimum);
        };
        const observer = new ResizeObserver(handleMinimumSize); // 字号、模式及可见性变化后重新报告内部需求。
        for (const target of [element, information, buttons]) observer.observe(target);
        handleMinimumSize();
        return () => observer.disconnect();
    }, [onMinimumSizeChange]);
    useLayoutEffect(() => {
        const element = root.current; // 当前音乐内容根节点。
        if (!element) return;
        const handleResize = () => {
            if (!element.getClientRects().length) return;
            const style = getComputedStyle(element); // 扣除内边距后的宽度不依赖当前布局。
            const width = element.clientWidth - parseFloat(style.paddingLeft) - parseFloat(style.paddingRight); // 实际内容宽度。
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
                    <small title={snapshot?.source}>
                        {empty
                            ? ""
                            : `${snapshot?.source ?? ""}${snapshot?.playback === "playing" ? " · 正在播放" : snapshot?.playback === "paused" ? " · 已暂停" : ""}`}
                    </small>
                </div>
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
        </section>
    );
};
