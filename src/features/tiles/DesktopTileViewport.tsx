import { useEffect, useId, useRef, useState, type HTMLAttributes, type RefObject } from "react";

import collapseIcon from "../../../resources/fence-toggle-collapse.svg?raw";
import expandIcon from "../../../resources/fence-toggle-expand.svg?raw";
import "./DesktopTileViewport.css";

/** 公共滚动视口保留列表或普通内容的语义标签。 */
interface DesktopTileViewportProps extends HTMLAttributes<HTMLElement> {
    as?: "ul" | "div"; // 文件列表与普通内容分别保留原有语义。
    contentRef?: RefObject<HTMLElement | null>; // 内容持久滚动与拖放使用同一节点。
    collapsed?: boolean; // 只隐藏内容，不卸载子组件。
    scrollable?: boolean; // 固定内容关闭滚动行为及覆盖式滚动条。
}

/** 复用围栏覆盖式滚动条，文件与官方组件共享滚动视口。 */
export const DesktopTileViewport = ({
    as: Content = "div",
    contentRef,
    collapsed,
    scrollable = true,
    children,
    className = "",
    onScroll,
    ...props
}: DesktopTileViewportProps) => {
    const viewportId = useId(); // 辅助技术通过稳定标识关联滚动条与内容。
    const localRef = useRef<HTMLElement | null>(null); // 未提供外部引用时使用的滚动节点。
    const scrollRef = contentRef ?? localRef; // 拖放、持久化与滚动条共同拥有的滚动位置。
    const trackRef = useRef<HTMLDivElement>(null); // 覆盖式滚动轨道。
    const [bar, setBar] = useState({ max: 0, top: 0, height: 0, track: 0 }); // 当前滑块比例、位置与轨道尺寸。
    const cancel = useRef<(() => void) | undefined>(undefined); // 卸载时释放滑块连续输入。
    const update = () => {
        const content = scrollRef.current; // 当前内容视口。
        const track = trackRef.current; // 当前覆盖轨道。
        if (!content || !track) return;
        const max = Math.max(0, content.scrollHeight - content.clientHeight); // 最大内容滚动距离。
        const height = max > 0 ? Math.max(18, (content.clientHeight / content.scrollHeight) * track.clientHeight) : 0; // 最小 18 DIP 滑块。
        setBar({ max, height, track: track.clientHeight, top: max ? (content.scrollTop / max) * Math.max(0, track.clientHeight - height) : 0 });
    }; // 同步覆盖式滚动条几何。
    useEffect(() => {
        if (!scrollable) return;
        update();
        const observer = new ResizeObserver(update); // 字号、网格和内容尺寸变化均刷新。
        if (scrollRef.current) {
            observer.observe(scrollRef.current);
            for (const child of scrollRef.current.children) observer.observe(child);
        }
        if (trackRef.current) observer.observe(trackRef.current);
        return () => observer.disconnect();
    }, [children, collapsed, scrollable]);
    useEffect(() => () => cancel.current?.(), []);
    const scrollBy = (top: number) => scrollRef.current?.scrollBy({ top }); // 使用当前 46 DIP 步进和分页距离。
    return (
        <div className="desktop-tile-viewport" hidden={collapsed}>
            <Content
                {...props}
                id={props.id ?? viewportId}
                ref={(element: HTMLElement | null) => {
                    scrollRef.current = element;
                }}
                className={`desktop-tile-viewport__content${scrollable ? "" : " desktop-tile-viewport__content--fixed"} ${className}`}
                onScroll={(event) => {
                    update();
                    onScroll?.(event);
                }}
            >
                {children}
            </Content>
            {scrollable && (
                <div className="desktop-tile-viewport__scrollbar" data-scrollable={bar.max > 1}>
                    <button type="button" className="desktop-tile-viewport__button" aria-label="向上滚动" onClick={() => scrollBy(-46)}>
                        <span aria-hidden="true" dangerouslySetInnerHTML={{ __html: collapseIcon }} />
                    </button>
                    <div
                        ref={trackRef}
                        className="desktop-tile-viewport__track"
                        onPointerDown={(event) => {
                            if (event.button !== 0 || event.target !== event.currentTarget || !scrollRef.current) return;
                            scrollBy(
                                event.clientY < event.currentTarget.getBoundingClientRect().top + bar.top
                                    ? -scrollRef.current.clientHeight
                                    : scrollRef.current.clientHeight,
                            );
                        }}
                    >
                        <div
                            className="desktop-tile-viewport__thumb"
                            role="scrollbar"
                            aria-controls={props.id ?? viewportId}
                            tabIndex={0}
                            aria-label="格子滚动位置"
                            aria-orientation="vertical"
                            aria-valuemin={0}
                            aria-valuemax={Math.round(bar.max)}
                            aria-valuenow={Math.round(scrollRef.current?.scrollTop ?? 0)}
                            style={{ height: bar.height, transform: `translateY(${bar.top}px)` }}
                            onKeyDown={(event) => {
                                if (!["ArrowUp", "ArrowDown", "PageUp", "PageDown", "Home", "End"].includes(event.key)) return;
                                event.preventDefault();
                                event.stopPropagation();
                                scrollBy(
                                    event.key === "Home"
                                        ? -bar.max
                                        : event.key === "End"
                                          ? bar.max
                                          : (event.key.endsWith("Up") ? -1 : 1) * (event.key.startsWith("Page") ? (scrollRef.current?.clientHeight ?? 0) : 46),
                                );
                            }}
                            onPointerDown={(event) => {
                                if (event.button !== 0 || bar.max <= 0 || bar.track <= bar.height) return;
                                event.preventDefault();
                                event.stopPropagation();
                                const thumb = event.currentTarget; // 当前捕获滑块。
                                const pointer = event.pointerId; // 当前唯一指针。
                                const startY = event.clientY; // 起始指针纵坐标。
                                const startScroll = scrollRef.current?.scrollTop ?? 0; // 本次滑块拖动的滚动起点。
                                const handleMove = (move: globalThis.PointerEvent) => {
                                    if (move.pointerId === pointer && scrollRef.current)
                                        scrollRef.current.scrollTop = startScroll + ((move.clientY - startY) / (bar.track - bar.height)) * bar.max;
                                }; // 按轨道比例转换内容滚动。
                                const finish = () => {
                                    thumb.removeEventListener("pointermove", handleMove);
                                    thumb.removeEventListener("pointerup", finish);
                                    thumb.removeEventListener("pointercancel", finish);
                                    thumb.removeEventListener("lostpointercapture", finish);
                                    window.removeEventListener("blur", finish);
                                    if (thumb.hasPointerCapture(pointer)) thumb.releasePointerCapture(pointer);
                                    cancel.current = undefined;
                                }; // 所有终止路径释放输入。
                                try {
                                    thumb.setPointerCapture(pointer);
                                } catch {
                                    return;
                                }
                                cancel.current = finish;
                                thumb.addEventListener("pointermove", handleMove);
                                thumb.addEventListener("pointerup", finish);
                                thumb.addEventListener("pointercancel", finish);
                                thumb.addEventListener("lostpointercapture", finish);
                                window.addEventListener("blur", finish);
                            }}
                        />
                    </div>
                    <button type="button" className="desktop-tile-viewport__button" aria-label="向下滚动" onClick={() => scrollBy(46)}>
                        <span aria-hidden="true" dangerouslySetInnerHTML={{ __html: expandIcon }} />
                    </button>
                </div>
            )}
        </div>
    );
};
