import { useEffect, useState, type RefObject } from "react";

/** 鼠标与跨屏拖入共用临时展开状态，不修改保存的折叠值。 */
export const useDesktopTileExpansion = (surface: RefObject<HTMLElement | null>, collapsed: boolean, autoExpand: boolean, monitorId?: string) => {
    const [hovered, setHovered] = useState(false); // 当前本地或远端指针位于表面。
    const [menuHovered, setMenuHovered] = useState(false); // 所属 Portal 菜单与格子表面组成同一悬停范围。
    const [dropHovered, setDropHovered] = useState(false); // 业务允许的拖入独立于普通悬停偏好临时展开。
    useEffect(() => {
        if (!collapsed || !monitorId) return;
        let timeout: ReturnType<typeof setTimeout> | undefined; // 远端异常停止时收起临时表面。
        const handleRemote = (event: Event) => {
            const drag = (event as CustomEvent).detail; // 宿主跨屏消息。
            if (!drag || drag.phase === "result" || drag.source === monitorId) return;
            const bounds = surface.current?.getBoundingClientRect(); // 当前有效表面。
            const contentTarget = surface.current?.dataset.dropComponentType; // 具体格子声明业务类型，公共外壳不处理内容。
            const acceptsContent = surface.current?.dataset.expandOnDrop === "true" && contentTarget && drag.contentTargets?.includes(contentTarget); // 业务声明与宿主白名单共同决定展开。
            const top =
                drag.pointer &&
                document
                    .elementsFromPoint(drag.pointer.left, drag.pointer.top)
                    .map((node) => node.closest(".desktop-tile"))
                    .find(Boolean); // 遮挡格子不能被穿透唤出。
            const inside =
                drag.phase !== "end" &&
                drag.phase !== "pending" &&
                drag.target === monitorId &&
                top === surface.current &&
                bounds &&
                drag.pointer?.left >= bounds.left &&
                drag.pointer?.left <= bounds.right &&
                drag.pointer?.top >= bounds.top &&
                drag.pointer?.top <= bounds.bottom; // 沿用围栏临时展开边界。
            setHovered(Boolean(inside));
            setDropHovered(Boolean(inside && acceptsContent));
            clearTimeout(timeout);
            if (inside)
                timeout = setTimeout(() => {
                    setHovered(false);
                    setDropHovered(false);
                }, 2500);
        };
        window.addEventListener("desktop-tools-drag", handleRemote);
        return () => {
            clearTimeout(timeout);
            window.removeEventListener("desktop-tools-drag", handleRemote);
            setDropHovered(false);
        };
    }, [collapsed, autoExpand, monitorId, surface]);
    return {
        collapsed: collapsed && !(autoExpand && (hovered || menuHovered)) && !dropHovered,
        onMenuHoverChange: setMenuHovered,
        onPointerEnter: () => setHovered(true),
        onPointerLeave: () => setHovered(false),
    };
};
