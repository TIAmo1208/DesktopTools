import { createContext, useCallback, useEffect, useRef, useState, type ComponentPropsWithRef, type PointerEvent } from "react";
import { createPortal } from "react-dom";

import type { TileAppearance, TileAppearanceChange } from "../../contracts/tile";
import type { FenceResizeDirection } from "../fences/fenceGeometry";
import { DesktopTileMenu, type DesktopTileMenuItem } from "./DesktopTileMenu";
import "./DesktopTileBase.css";

/** 标题只读取外壳的瞬时可见状态与保存的按钮偏好。 */
export const DesktopTileContext = createContext({ titleVisible: true, titleMode: "fixed", buttonMode: "hover", sortMenuOpen: false });

/** 公共表面保留宿主身份，按内容能力组合菜单。 */
interface DesktopTileBaseProps extends Omit<ComponentPropsWithRef<"section">, "onResize"> {
    "data-hit-id"?: string; // 宿主提供的稳定表面身份。
    collapsed?: boolean; // 当前有效折叠状态。
    appearance?: TileAppearance; // 共享外观。
    headerPresent?: boolean; // 开发者组合能力。
    renaming?: boolean; // 重命名允许临时显示标题。
    onRenameStart?: () => void; // 编辑入口。
    onAppearanceChange?: (change: TileAppearanceChange) => void; // 单字段领域命令。
    menuItems?: DesktopTileMenuItem[]; // 内容专属菜单。
    sortItems?: DesktopTileMenuItem[]; // 排序按钮共用的菜单。
    onResize: (event: PointerEvent<HTMLElement>, direction: FenceResizeDirection) => void; // 公共几何入口。
}

/** 共用表面、标题显隐、菜单与缩放结构，保留内容生命周期。 */
export const DesktopTileBase = ({
    children,
    className = "",
    collapsed = false,
    appearance,
    headerPresent = true,
    renaming = false,
    onRenameStart,
    onAppearanceChange,
    menuItems = [],
    sortItems,
    onResize,
    ref,
    onContextMenu,
    onKeyDown,
    onFocus,
    ...props
}: DesktopTileBaseProps) => {
    const surface = useRef<HTMLElement | null>(null); // 外壳节点。
    const [titleVisible, setTitleVisible] = useState(false); // 悬浮标题瞬时显示状态。
    const [menu, setMenu] = useState<{ left: number; top: number; sorting: boolean }>(); // 菜单锚点。
    const trigger = useRef<HTMLElement | null>(null); // 菜单焦点恢复位置。
    const frozen = useRef(false); // 连续操作冻结标题。
    const awaitingMove = useRef(false); // 释放后等待普通鼠标移动。
    const hideTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined); // 离开标题后的隐藏延迟。
    const titleMode = appearance?.titleMode ?? "fixed"; // 默认固定标题。
    const buttonMode = appearance?.buttonMode ?? "hover"; // 默认悬停按钮。
    const visible = titleMode === "fixed" || collapsed || titleVisible || (!frozen.current && renaming); // 包括重命名和折叠入口的实际显示状态。
    const state = useRef({ menu, renaming, visible }); // 原生监听读取最新交互状态。
    state.current = { menu, renaming, visible };
    const handleClose = useCallback(() => {
        setMenu(undefined);
        trigger.current?.focus({ preventScroll: true });
    }, []); // 关闭整组菜单。

    useEffect(() => {
        const handleFreeze = () => {
            if (!frozen.current) setTitleVisible(state.current.visible); // 冻结实际状态，临时展开或重命名变化不隐藏已有标题。
            frozen.current = true;
            clearTimeout(hideTimer.current);
            hideTimer.current = undefined;
        }; // 指针按下或原生拖动不唤出标题。
        const handleRelease = () => {
            frozen.current = false;
            awaitingMove.current = true;
        }; // 释放时不判断悬停。
        const handleRemote = (event: Event) => {
            const drag = (event as CustomEvent).detail; // 原生 OLE 与跨屏消息。
            if (!drag || drag.phase === "result") return;
            if (drag.phase === "end") handleRelease();
            else handleFreeze();
        };
        const handleMove = (event: globalThis.PointerEvent) => {
            if (event.buttons) {
                handleFreeze();
                return;
            }
            if (awaitingMove.current) {
                awaitingMove.current = false;
                frozen.current = false;
            }
            if (frozen.current || state.current.menu || state.current.renaming) return;
            const bounds = surface.current?.getBoundingClientRect(); // 当前 DIP 表面。
            const inside =
                bounds && event.clientX >= bounds.left && event.clientX <= bounds.right && event.clientY >= bounds.top && event.clientY <= bounds.top + 28; // 顶部 28 DIP 触发区。
            if (inside) {
                clearTimeout(hideTimer.current);
                hideTimer.current = undefined;
                setTitleVisible(true);
            } else if (!hideTimer.current)
                hideTimer.current = setTimeout(() => {
                    hideTimer.current = undefined;
                    if (!frozen.current && !state.current.menu && !state.current.renaming) setTitleVisible(false);
                }, 200);
        };
        document.addEventListener("pointerdown", handleFreeze, true);
        document.addEventListener("pointerup", handleRelease, true);
        document.addEventListener("pointercancel", handleRelease, true);
        document.addEventListener("pointermove", handleMove, true);
        window.addEventListener("blur", handleRelease);
        window.addEventListener("desktop-tools-drag", handleRemote);
        return () => {
            clearTimeout(hideTimer.current);
            document.removeEventListener("pointerdown", handleFreeze, true);
            document.removeEventListener("pointerup", handleRelease, true);
            document.removeEventListener("pointercancel", handleRelease, true);
            document.removeEventListener("pointermove", handleMove, true);
            window.removeEventListener("blur", handleRelease);
            window.removeEventListener("desktop-tools-drag", handleRemote);
        };
    }, []);

    useEffect(() => {
        const element = surface.current; // 菜单键与排序按钮的共同请求目标。
        const handleOpen = (event: Event) => {
            const detail = (event as CustomEvent).detail as { left: number; top: number; sorting?: boolean; trigger?: HTMLElement }; // 本地界面请求。
            event.stopPropagation();
            window.dispatchEvent(new Event("desktop-tile-menu-opening"));
            trigger.current = detail.trigger ?? element;
            setMenu({ left: detail.left, top: detail.top, sorting: detail.sorting === true });
        };
        element?.addEventListener("desktop-tile-menu", handleOpen);
        return () => element?.removeEventListener("desktop-tile-menu", handleOpen);
    }, []);

    const commonItems: DesktopTileMenuItem[] =
        headerPresent && onAppearanceChange
            ? [
                  ...(onRenameStart ? [{ label: "重命名", onSelect: onRenameStart }] : []),
                  {
                      label: "标题显示方式",
                      children: [
                          {
                              label: "固定显示",
                              radio: true,
                              checked: titleMode === "fixed",
                              onSelect: () => onAppearanceChange({ type: "title-mode", value: "fixed" }),
                          },
                          {
                              label: "悬浮隐藏",
                              radio: true,
                              checked: titleMode === "hover",
                              onSelect: () => onAppearanceChange({ type: "title-mode", value: "hover" }),
                          },
                      ],
                  },
                  {
                      label: "功能选项显示",
                      description: titleMode === "hover" ? "悬浮标题完整显示按钮，已保存偏好在固定标题下可修改" : undefined,
                      children: [
                          {
                              label: "固定显示",
                              disabled: titleMode === "hover",
                              radio: true,
                              checked: buttonMode === "fixed",
                              onSelect: () => onAppearanceChange({ type: "button-mode", value: "fixed" }),
                          },
                          {
                              label: "悬停显示",
                              disabled: titleMode === "hover",
                              radio: true,
                              checked: buttonMode === "hover",
                              onSelect: () => onAppearanceChange({ type: "button-mode", value: "hover" }),
                          },
                      ],
                  },
                  {
                      label: "折叠时自动展开",
                      checked: appearance?.autoExpand ?? false,
                      onSelect: () => onAppearanceChange({ type: "auto-expand", value: !appearance?.autoExpand }),
                  },
                  { label: appearance?.collapsed ? "展开" : "折叠", onSelect: () => onAppearanceChange({ type: "collapsed", value: !appearance?.collapsed }) },
              ]
            : []; // 无标题时不提供会失去恢复入口的命令。

    return (
        <DesktopTileContext.Provider value={{ titleVisible: visible, titleMode, buttonMode, sortMenuOpen: menu?.sorting === true }}>
            <section
                {...props}
                ref={(element) => {
                    surface.current = element;
                    if (typeof ref === "function") ref(element);
                    else if (ref) ref.current = element;
                }}
                className={`desktop-tile${collapsed ? " desktop-tile--collapsed" : ""} ${className}`}
                data-title-mode={headerPresent ? titleMode : "none"}
                onFocus={(event) => {
                    if (!frozen.current && !state.current.menu && event.currentTarget.matches(":focus-visible")) setTitleVisible(true);
                    onFocus?.(event);
                }}
                onContextMenu={(event) => {
                    event.preventDefault();
                    event.stopPropagation();
                    onContextMenu?.(event);
                    event.currentTarget.dispatchEvent(new CustomEvent("desktop-tile-menu", { detail: { left: event.clientX, top: event.clientY } }));
                }}
                onKeyDown={(event) => {
                    if (event.target instanceof Element && event.target.closest("input,textarea,[data-item-id]")) return;
                    if (event.key === "ContextMenu" || (event.shiftKey && event.key === "F10")) {
                        event.preventDefault();
                        event.stopPropagation();
                        const bounds = event.currentTarget.getBoundingClientRect(); // 键盘菜单使用格子位置。
                        event.currentTarget.dispatchEvent(new CustomEvent("desktop-tile-menu", { detail: { left: bounds.left + 12, top: bounds.top + 28 } }));
                        return;
                    }
                    if (event.key === "F2" && headerPresent) {
                        event.preventDefault();
                        event.stopPropagation();
                        onRenameStart?.();
                        return;
                    }
                    if (!frozen.current) setTitleVisible(true);
                    onKeyDown?.(event);
                }}
            >
                {children}
                {!collapsed &&
                    (["n", "ne", "e", "se", "s", "sw", "w", "nw"] as const).map((direction) => (
                        <span
                            key={direction}
                            className={`desktop-tile__resize desktop-tile__resize--${direction}`}
                            aria-hidden="true"
                            data-hit-id={`${props["data-hit-id"] ?? "tile"}:resize:${direction}`}
                            data-hit-kind="component"
                            data-hit-priority="70"
                            onPointerDown={(event) => onResize(event, direction)}
                        />
                    ))}
            </section>
            {menu &&
                surface.current &&
                createPortal(
                    <DesktopTileMenu
                        items={menu.sorting && sortItems ? sortItems : [...commonItems, ...menuItems]}
                        left={menu.left}
                        top={menu.top}
                        onClose={handleClose}
                    />,
                    surface.current.closest(".fence-workspace") ?? document.body,
                )}
        </DesktopTileContext.Provider>
    );
};
