import { createContext, useCallback, useEffect, useLayoutEffect, useRef, useState, type ComponentPropsWithRef, type PointerEvent, type ReactNode } from "react";
import { createPortal } from "react-dom";

import type { TileAppearance, TileAppearanceChange } from "../../contracts/tile";
import { useDesktopTileGeometry, type DesktopTileGeometryOptions } from "./useDesktopTileGeometry";
import { useDesktopTileExpansion } from "./useDesktopTileExpansion";
import { usePreferences } from "../settings/PreferencesProvider";
import { DesktopTileMenu, DesktopTileMenuHoverContext, type DesktopTileMenuItem } from "./DesktopTileMenu";
import "./DesktopTileBase.css";

/** 标题只读取外壳的瞬时可见状态与保存的按钮偏好。 */
export const DesktopTileContext = createContext<{
    collapsed: boolean; // 实际折叠状态，包括临时展开。
    titleVisible: boolean; // 当前标题可见性。
    titleMode: string; // 保存的标题显示方式。
    buttonMode: string; // 保存的按钮显示方式。
    sortMenuOpen: boolean; // 公共排序菜单状态。
    onHeaderPointerDown?: (event: PointerEvent<HTMLElement>) => void; // 唯一标题拖动入口。
}>({ collapsed: false, titleVisible: true, titleMode: "fixed", buttonMode: "hover", sortMenuOpen: false });

/** 公共表面保留宿主身份，按内容能力组合菜单。 */
interface DesktopTileBaseProps extends Omit<ComponentPropsWithRef<"section">, "onResize" | "children"> {
    children?: ReactNode | ((collapsed: boolean) => ReactNode); // 内容可以读取实际展开状态，不自行注册展开监听。
    geometry: Omit<DesktopTileGeometryOptions, "surfaceRef" | "collapsed" | "collapsedSize">; // 类型化几何参数和领域回调。
    "data-hit-id"?: string; // 宿主提供的稳定表面身份。
    collapsed?: boolean; // 当前有效折叠状态。
    collapsedSize?: { width?: number; height?: number }; // 调用方指定折叠尺寸，缺省高度为 28px。
    appearance?: TileAppearance; // 共享外观。
    headerPresent?: boolean; // 开发者组合能力。
    renaming?: boolean; // 重命名允许临时显示标题。
    onRenameStart?: () => void; // 编辑入口。
    onAppearanceChange?: (change: TileAppearanceChange) => void; // 单字段领域命令。
    menuItems?: DesktopTileMenuItem[]; // 内容专属菜单。
    sortItems?: DesktopTileMenuItem[]; // 排序按钮共用的菜单。
}

/** 共用表面、标题显隐、菜单与缩放结构，保留内容生命周期。 */
export const DesktopTileBase = ({
    children,
    className = "",
    appearance,
    collapsed: savedCollapsed = appearance?.collapsed ?? false,
    collapsedSize,
    geometry,
    headerPresent = true,
    renaming = false,
    onRenameStart,
    onAppearanceChange,
    menuItems = [],
    sortItems,
    ref,
    onContextMenu,
    onKeyDown,
    onFocus,
    onPointerEnter,
    onPointerLeave,
    style,
    ...props
}: DesktopTileBaseProps) => {
    const surface = useRef<HTMLElement | null>(null); // 外壳节点。
    const expansion = useDesktopTileExpansion(surface, savedCollapsed, appearance?.autoExpand ?? false, geometry.currentMonitor?.id); // 所有格子共用临时展开。
    const collapsed = expansion.collapsed; // 标题、内容、几何与命中共享同一实际状态。
    const { handlePointerDown, handleKeyDown } = useDesktopTileGeometry({
        ...geometry,
        surfaceRef: surface,
        collapsed,
        collapsedSize,
        onFocus: () => {
            surface.current?.focus({ preventScroll: true });
            geometry.onFocus();
        },
    }); // 外框独占输入装配，宿主只提交业务命令。
    const collapseAnimation = useRef<Animation | undefined>(undefined); // 当前折叠高度动画，连续切换从当前帧衔接。
    const previousCollapsed = useRef(collapsed); // 上次呈现的实际折叠状态。
    const previousHeight = useRef<number | undefined>(undefined); // 最近一次稳定外壳高度。
    const [titleVisible, setTitleVisible] = useState(false); // 悬浮标题瞬时显示状态。
    const [menu, setMenu] = useState<{ left: number; top: number; sorting: boolean }>(); // 菜单锚点。
    const trigger = useRef<HTMLElement | null>(null); // 菜单焦点恢复位置。
    const frozen = useRef(false); // 连续操作冻结标题。
    const awaitingMove = useRef(false); // 释放后等待普通鼠标移动。
    const hideTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined); // 离开标题后的隐藏延迟。
    const titleMode = appearance?.titleMode ?? "fixed"; // 默认固定标题。
    const buttonMode = appearance?.buttonMode ?? "hover"; // 默认悬停按钮。
    const { animationsEnabled } = usePreferences(); // 用户偏好与系统减少动态效果合并后的有效状态。
    const visible = titleMode === "fixed" || collapsed || titleVisible || renaming; // 编辑标题期间始终可见，避免冻结态把输入框隐藏成不可聚焦。
    const state = useRef({ menu, renaming, visible }); // 原生监听读取最新交互状态。
    state.current = { menu, renaming, visible };
    const handleClose = useCallback(() => {
        setMenu(undefined);
        trigger.current?.focus({ preventScroll: true });
    }, []); // 关闭整组菜单。

    useLayoutEffect(() => {
        const element = surface.current;
        if (!element) return;
        const start = collapseAnimation.current ? element.getBoundingClientRect().height : previousHeight.current; // 反向切换读取当前动画帧。
        collapseAnimation.current?.cancel();
        collapseAnimation.current = undefined;
        const target = element.getBoundingClientRect().height; // 取消旧动画后读取当前目标高度。
        const changed = previousCollapsed.current !== collapsed;
        previousCollapsed.current = collapsed;
        previousHeight.current = target;
        if (!changed || !animationsEnabled || start === undefined || Math.abs(start - target) < 0.5) return;
        const duration = Number.parseFloat(getComputedStyle(element).getPropertyValue("--interface-transition-duration")); // 复用全局动效时长。
        if (!duration) return;
        const animation = element.animate([{ height: `${start}px` }, { height: `${target}px` }], { duration, easing: "ease-out" });
        collapseAnimation.current = animation;
        animation.onfinish = () => {
            if (collapseAnimation.current !== animation) return;
            collapseAnimation.current = undefined;
            animation.cancel(); // 底层目标高度已生效，移除完成实例避免连续切换读取旧动画。
        };
    }, [animationsEnabled, collapsed, geometry.bounds.height, style?.height, collapsedSize?.height]);

    useEffect(() => () => collapseAnimation.current?.cancel(), []);

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
                      separatorBefore: true,
                      checked: appearance?.autoExpand ?? false,
                      onSelect: () => onAppearanceChange({ type: "auto-expand", value: !appearance?.autoExpand }),
                  },
                  { label: appearance?.collapsed ? "展开" : "折叠", onSelect: () => onAppearanceChange({ type: "collapsed", value: !appearance?.collapsed }) },
              ]
            : []; // 无标题时不提供会失去恢复入口的命令。

    return (
        <DesktopTileMenuHoverContext.Provider value={expansion.onMenuHoverChange}>
            <DesktopTileContext.Provider
                value={{
                    collapsed,
                    titleVisible: visible,
                    titleMode,
                    buttonMode,
                    sortMenuOpen: menu?.sorting === true,
                    onHeaderPointerDown: (event) => void handlePointerDown(event),
                }}
            >
                <section
                    {...props}
                    ref={(element) => {
                        surface.current = element;
                        if (typeof ref === "function") ref(element);
                        else if (ref) ref.current = element;
                    }}
                    className={`desktop-tile${collapsed ? " desktop-tile--collapsed" : ""} ${className}`}
                    data-title-mode={headerPresent ? titleMode : "none"}
                    data-auto-expanded={appearance?.collapsed && !collapsed ? "true" : undefined}
                    style={
                        {
                            ...geometry.bounds,
                            ...style,
                            ...(collapsed ? { ...collapsedSize, height: collapsedSize?.height ?? 28, minHeight: collapsedSize?.height ?? 28 } : {}),
                            "--tile-header-height": collapsed && (collapsedSize?.height ?? 28) > 28 ? `${(collapsedSize?.height ?? 28) - 4}px` : "28px",
                        } as import("react").CSSProperties
                    }
                    onPointerEnter={(event) => {
                        expansion.onPointerEnter();
                        onPointerEnter?.(event);
                    }}
                    onPointerLeave={(event) => {
                        expansion.onPointerLeave();
                        onPointerLeave?.(event);
                    }}
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
                        handleKeyDown(event);
                        if (event.defaultPrevented) return;
                        if (event.target instanceof Element && event.target.closest("input,textarea,[data-item-id]")) return;
                        if (event.key === "ContextMenu" || (event.shiftKey && event.key === "F10")) {
                            event.preventDefault();
                            event.stopPropagation();
                            const bounds = event.currentTarget.getBoundingClientRect(); // 键盘菜单使用格子位置。
                            event.currentTarget.dispatchEvent(
                                new CustomEvent("desktop-tile-menu", { detail: { left: bounds.left + 12, top: bounds.top + 28 } }),
                            );
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
                    {typeof children === "function" ? children(collapsed) : children}
                    {!collapsed &&
                        !geometry.locked &&
                        (["n", "ne", "e", "se", "s", "sw", "w", "nw"] as const).map((direction) => (
                            <span
                                key={direction}
                                className={`desktop-tile__resize desktop-tile__resize--${direction}`}
                                aria-hidden="true"
                                data-hit-id={`${props["data-hit-id"] ?? "tile"}:resize:${direction}`}
                                data-hit-kind="component"
                                data-hit-priority="70"
                                onPointerDown={(event) => void handlePointerDown(event, direction)}
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
        </DesktopTileMenuHoverContext.Provider>
    );
};
