import { useEffect, useState, type RefObject } from "react";

/** 鼠标与跨屏拖入共用临时展开状态，不修改保存的折叠值。 */
export const useDesktopTileExpansion = (surface: RefObject<HTMLElement | null>, collapsed: boolean, autoExpand: boolean, monitorId?: string) => {
    const [hovered, setHovered] = useState(false); // 当前本地或远端指针位于表面。
    useEffect(() => {
        if (!collapsed || !autoExpand || !monitorId) return;
        let timeout: ReturnType<typeof setTimeout> | undefined; // 远端异常停止时收起临时表面。
        const handleRemote = (event: Event) => {
            const drag = (event as CustomEvent).detail; // 宿主跨屏消息。
            if (!drag || drag.phase === "result" || drag.source === monitorId) return;
            const bounds = surface.current?.getBoundingClientRect(); // 当前有效表面。
            const inside =
                drag.phase !== "end" &&
                drag.target === monitorId &&
                bounds &&
                drag.pointer?.left >= bounds.left &&
                drag.pointer?.left <= bounds.right &&
                drag.pointer?.top >= bounds.top &&
                drag.pointer?.top <= bounds.bottom; // 沿用围栏临时展开边界。
            setHovered(Boolean(inside));
            clearTimeout(timeout);
            if (inside) timeout = setTimeout(() => setHovered(false), 2500);
        };
        window.addEventListener("desktop-tools-drag", handleRemote);
        return () => {
            clearTimeout(timeout);
            window.removeEventListener("desktop-tools-drag", handleRemote);
        };
    }, [collapsed, autoExpand, monitorId, surface]);
    return { collapsed: collapsed && !(autoExpand && hovered), onPointerEnter: () => setHovered(true), onPointerLeave: () => setHovered(false) };
};
