import { useLayoutEffect, useRef, useState, type CSSProperties } from "react";

import { overflowDistance, overflowLoopMs } from "./textScroll";
import "./ScrollingText.css";

/** 一次溢出滚动计划：需要位移的像素与一个往返周期的时长。 */
interface OverflowPlan {
    distance: number; // 需要位移的像素。
    durationMs: number; // 一个往返周期的时长。
}

/**
 * 单行文本：内容超出可视宽度时慢速往返滚动，否则保持单行截断。
 *
 * 只读尺寸不写布局：滚动由子层 transform 推进，不撑高也不改变父级测量，
 * 浮岛窗口提交与桌面格子几何都不会被滚动重新触发。
 */
export const ScrollingText = ({
    text,
    title,
    enabled = true,
    className,
}: {
    text: string; // 呈现文本；变化时重新测量并重置动画。
    title?: string; // 悬停查看完整文本；与文本相同时由调用方决定是否给出。
    enabled?: boolean; // 关闭动效时不滚动，交给 CSS 单行截断。
    className?: string; // 调用方的排版类，组件只负责滚动外壳。
}) => {
    const box = useRef<HTMLSpanElement>(null); // 可视宽度来源。
    const track = useRef<HTMLSpanElement>(null); // 实际内容宽度来源与动画层。
    const [plan, setPlan] = useState<OverflowPlan>();

    useLayoutEffect(() => {
        const boxElement = box.current;
        const trackElement = track.current;
        if (!boxElement || !trackElement) {
            setPlan(undefined);
            return;
        }
        let last = 0; // 与渲染值同步的测量缓存，同一尺寸不重复写状态。
        const update = () => {
            const distance = enabled ? overflowDistance(trackElement.scrollWidth, boxElement.clientWidth) : 0;
            if (last === distance) return;
            last = distance;
            setPlan(distance > 0 ? { distance, durationMs: overflowLoopMs(distance) } : undefined);
        };
        update();
        const observer = new ResizeObserver(update); // 字号、内容与容器宽度变化都重新判定。
        observer.observe(boxElement);
        observer.observe(trackElement);
        return () => observer.disconnect();
    }, [enabled, text]);

    const style = plan ? ({ "--overflow-distance": `${plan.distance}px`, "--overflow-duration": `${plan.durationMs}ms` } as CSSProperties) : undefined;
    return (
        <span ref={box} className={className ? `scrolling-text ${className}` : "scrolling-text"} data-overflow={plan ? "true" : "false"} title={title}>
            <span ref={track} className="scrolling-text__track" style={style}>
                {text}
            </span>
        </span>
    );
};
