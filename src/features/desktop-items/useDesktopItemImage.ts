import { useEffect, useRef, useState } from "react";
import { getDesktopItemImage } from "../../bridge/desktopItems";
import type { DesktopItemViewModel } from "./desktopItemViewModel";

/** 只为可见项目读取缩略图，身份变化、暂停和卸载均隔离迟到结果。 */
export const useDesktopItemImage = (item: DesktopItemViewModel) => {
    const container = useRef<HTMLDivElement>(null); // 图像槽沿用父容器裁剪。
    const [loaded, setLoaded] = useState<{ key: string; revision?: number; source: string }>(); // 当前实例的结果。
    useEffect(() => {
        const element = container.current; // 本次挂载的 DOM。
        if (!element || !item.isAvailable) return;
        let active = true; // 卸载后拒绝写回。
        let visible = false; // 观察完成前不读取。
        let generation = 0; // 本地请求代次。
        let requestedPixels = 0; // 合并重复尺寸请求。
        const request = async () => {
            if (!active || !visible || window.__DESKTOP_TOOLS_NATIVE__?.isRefreshPaused?.()) return;
            const pixels = Math.max(16, Math.min(384, Math.ceil(element.getBoundingClientRect().width * window.devicePixelRatio))); // DPI 只影响位图分辨率。
            if (requestedPixels === pixels) return;
            requestedPixels = pixels;
            const current = ++generation; // 隔离旧尺寸和旧可见状态。
            try {
                const source = await getDesktopItemImage(item.key, pixels); // 后端合并相同读取。
                if (!active || current !== generation || !visible || window.__DESKTOP_TOOLS_NATIVE__?.isRefreshPaused?.()) return;
                setLoaded(source ? { key: item.key, revision: item.imageRevision, source } : undefined);
            } catch {
                if (active && current === generation) setLoaded(undefined);
            }
        };
        const intersection = new IntersectionObserver(([entry]) => {
            visible = entry?.isIntersecting ?? false;
            if (!visible) {
                generation++;
                requestedPixels = 0;
            } else void request();
        }); // 折叠和滚出视口时停止新请求。
        const resize = new ResizeObserver(() => void request()); // 列表切换按槽尺寸读取。
        const handleRefresh = () => {
            generation++;
            requestedPixels = 0;
            void request();
        }; // 暂停不补绘，恢复重读。
        const handleResize = () => void request(); // 跨 DPI 显示器变化。
        intersection.observe(element);
        resize.observe(element);
        window.addEventListener("desktop-refresh-changed", handleRefresh);
        window.addEventListener("resize", handleResize);
        return () => {
            active = false;
            generation++;
            intersection.disconnect();
            resize.disconnect();
            window.removeEventListener("desktop-refresh-changed", handleRefresh);
            window.removeEventListener("resize", handleResize);
        };
    }, [item.key, item.imageRevision, item.isAvailable]);
    const source = loaded?.key === item.key && loaded.revision === item.imageRevision ? loaded.source : item.imageSource; // 不借用上一文件的图像。
    return { container, source };
};
