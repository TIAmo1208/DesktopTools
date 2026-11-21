import { useEffect, useLayoutEffect, useRef, useState } from "react";

import { overflowDistance, overflowScrollKeyframes, overflowScrollPlan, type OverflowScrollPlan } from "./textScroll";
import "./ScrollingText.css";

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
    const animation = useRef<Animation | undefined>(undefined); // 当前往返动画；计划失效时同步取消。
    const [plan, setPlan] = useState<OverflowScrollPlan>();
    const [reducedMotion, setReducedMotion] = useState(() => typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches);

    // 系统减少动效在运行中变化时立即停下并复位。
    useEffect(() => {
        if (typeof matchMedia !== "function") return;
        const query = matchMedia("(prefers-reduced-motion: reduce)");
        const handleChange = () => setReducedMotion(query.matches);
        query.addEventListener("change", handleChange);
        return () => query.removeEventListener("change", handleChange);
    }, []);

    useLayoutEffect(() => {
        const boxElement = box.current;
        const trackElement = track.current;
        if (!boxElement || !trackElement || typeof trackElement.animate !== "function") {
            setPlan(undefined); // 不支持动画时保持单行省略，不留下无动画的裁切态。
            return;
        }
        const update = () => {
            const distance = enabled ? overflowDistance(trackElement.scrollWidth, boxElement.clientWidth) : 0;
            const next = distance > 0 ? overflowScrollPlan(distance) : undefined;
            setPlan((current) => {
                if (!current && !next) return current; // 已经是静态省略，不触发重渲染。
                if (current && next && current.distance === next.distance) return current;
                return next; // 禁用或文本变短时清空旧计划，旧动画由下方 effect 取消并复位。
            });
        };
        update();
        const observer = new ResizeObserver(update); // 字号、内容与容器宽度变化都重新判定。
        observer.observe(boxElement);
        observer.observe(trackElement);
        return () => observer.disconnect();
    }, [enabled, text]);

    // 只有存在计划且未减少动效时才启动；停止/切换时取消旧动画并把位移归零。
    useLayoutEffect(() => {
        const trackElement = track.current;
        animation.current?.cancel();
        animation.current = undefined;
        if (trackElement) trackElement.style.transform = "";
        if (!trackElement || !plan || reducedMotion) return;
        const frames = overflowScrollKeyframes(plan).map(({ offset, x }) => ({ offset, transform: `translate3d(${x}px, 0, 0)` }));
        const next = trackElement.animate(frames, { duration: plan.cycleMs, iterations: Infinity, easing: "linear", fill: "both" });
        animation.current = next;
        return () => {
            next.cancel();
            trackElement.style.transform = "";
        };
    }, [plan, reducedMotion]);

    return (
        <span ref={box} className={className ? `scrolling-text ${className}` : "scrolling-text"} data-overflow={plan ? "true" : "false"} title={title}>
            <span ref={track} className="scrolling-text__track">
                {text}
            </span>
        </span>
    );
};
