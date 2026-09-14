import { Fragment, useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent } from "react";

import "./DesktopTileMenu.css";

/** 公共菜单项目，内容只提供真实支持的命令。 */
export interface DesktopTileMenuItem {
    label: string; // 用户可见命令。
    checked?: boolean; // 单选或复选状态。
    radio?: boolean; // 同组互斥选项。
    disabled?: boolean; // 禁止执行的选项。
    description?: string; // 禁用原因和辅助技术说明。
    children?: DesktopTileMenuItem[]; // 同时显示的二级菜单。
    onSelect?: () => void; // 选择最终命令。
    separatorBefore?: boolean; // 与前一功能组分离。
}

/** 呈现同组两级菜单，统一焦点、边缘定位与透明宿主命中。 */
export const DesktopTileMenu = ({
    items,
    left,
    top,
    onClose,
}: {
    items: DesktopTileMenuItem[]; // 按产品顺序排列的命令。
    left: number; // 客户区菜单锚点。
    top: number; // 客户区菜单锚点。
    onClose: () => void; // 关闭整组并恢复触发位置。
}) => {
    const root = useRef<HTMLMenuElement>(null); // 一级菜单容器。
    const child = useRef<HTMLMenuElement>(null); // 二级菜单容器。
    const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined); // 悬停展开延迟。
    const [active, setActive] = useState<number>(); // 当前展开的唯一父项。
    const [position, setPosition] = useState({ left, top }); // 按实际菜单尺寸约束的位置。
    const [childPosition, setChildPosition] = useState({ left, top }); // 子菜单边缘翻转后的位置。
    const focusChild = useRef(false); // 键盘或点击立即展开时进入子菜单。

    useLayoutEffect(() => {
        const bounds = root.current?.getBoundingClientRect(); // 包含实际字号与长文本的一级菜单尺寸。
        if (bounds)
            setPosition({
                left: Math.max(0, Math.min(left, window.innerWidth - bounds.width)),
                top: Math.max(0, Math.min(top, window.innerHeight - bounds.height)),
            });
        root.current?.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus();
    }, [left, top]);

    useLayoutEffect(() => {
        if (active === undefined || !root.current || !child.current) return;
        const parent = root.current.querySelectorAll<HTMLElement>('li[role="none"]')[active]?.getBoundingClientRect(); // 当前父项真实位置。
        const bounds = child.current.getBoundingClientRect(); // 二级菜单真实尺寸。
        const menuBounds = root.current.getBoundingClientRect(); // 一级菜单边界。
        if (parent)
            setChildPosition({
                left: Math.max(0, menuBounds.right + bounds.width <= window.innerWidth ? menuBounds.right - 1 : menuBounds.left - bounds.width + 1),
                top: Math.max(0, Math.min(parent.top, window.innerHeight - bounds.height)),
            });
        if (focusChild.current) child.current.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus();
        focusChild.current = false;
    }, [active, position]);

    useEffect(() => {
        const handleOutside = (event: globalThis.PointerEvent) => {
            if (event.target instanceof Node && (root.current?.contains(event.target) || child.current?.contains(event.target))) return;
            onClose();
        }; // 外部点击覆盖整个父子菜单组。
        const handleClose = () => onClose(); // 窗口失焦和另一菜单打开时关闭整组。
        document.addEventListener("pointerdown", handleOutside, true);
        window.addEventListener("blur", handleClose);
        window.addEventListener("desktop-tile-menu-opening", handleClose);
        return () => {
            clearTimeout(timer.current);
            document.removeEventListener("pointerdown", handleOutside, true);
            window.removeEventListener("blur", handleClose);
            window.removeEventListener("desktop-tile-menu-opening", handleClose);
        };
    }, [onClose]);

    /** 返回父项时保留菜单组及其键盘焦点。 */
    const handleReturn = () => {
        const index = active; // 关闭前保存父项位置。
        setActive(undefined);
        if (index !== undefined) root.current?.querySelectorAll<HTMLElement>('li[role="none"]')[index]?.querySelector<HTMLButtonElement>("button")?.focus();
    };

    /** 两级菜单共用方向导航与逐级关闭规则。 */
    const handleKeyDown = (event: KeyboardEvent<HTMLElement>, nested: boolean) => {
        event.stopPropagation();
        if (event.key === "Escape" || (nested && event.key === "ArrowLeft")) {
            event.preventDefault();
            if (active !== undefined) handleReturn();
            else onClose();
            return;
        }
        if (event.key === "Tab") {
            onClose();
            return;
        }
        if (!["ArrowUp", "ArrowDown", "Home", "End"].includes(event.key)) return;
        event.preventDefault();
        const buttons = [...event.currentTarget.querySelectorAll<HTMLButtonElement>("button:not(:disabled)")]; // 当前层可访问项目。
        const index = buttons.indexOf(document.activeElement as HTMLButtonElement); // 当前层焦点位置。
        buttons[
            event.key === "Home"
                ? 0
                : event.key === "End"
                  ? buttons.length - 1
                  : (index + (event.key === "ArrowDown" ? 1 : -1) + buttons.length) % buttons.length
        ]?.focus();
    };

    /** 一级与二级使用相同命令表面，不复制状态规则。 */
    const renderItems = (entries: DesktopTileMenuItem[], nested: boolean) =>
        entries.map((item, index) => (
            <Fragment key={item.label}>
                {item.separatorBefore && <li className="desktop-tile-menu__separator" role="separator" aria-hidden="true" />}
                <li
                    role="none"
                    onPointerEnter={() => {
                        clearTimeout(timer.current);
                        if (!nested) timer.current = setTimeout(() => setActive(item.children && !item.disabled ? index : undefined), 200);
                    }}
                >
                    <button
                        type="button"
                        role={item.checked === undefined ? "menuitem" : item.radio ? "menuitemradio" : "menuitemcheckbox"}
                        aria-checked={item.checked}
                        disabled={item.disabled}
                        aria-disabled={item.disabled}
                        title={item.description}
                        aria-description={item.description}
                        aria-haspopup={item.children ? "menu" : undefined}
                        aria-expanded={item.children ? active === index : undefined}
                        data-expanded={!nested && active === index ? "true" : undefined}
                        onKeyDown={(event) => {
                            if (event.key !== "ArrowRight" || !item.children || item.disabled) return;
                            event.preventDefault();
                            event.stopPropagation();
                            clearTimeout(timer.current);
                            focusChild.current = true;
                            setActive(index);
                            if (active === index) child.current?.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus();
                        }}
                        onClick={() => {
                            if (item.disabled) return;
                            clearTimeout(timer.current);
                            if (item.children) {
                                focusChild.current = true;
                                setActive(index);
                                return;
                            }
                            onClose();
                            item.onSelect?.();
                        }}
                    >
                        <span className="desktop-tile-menu__check" aria-hidden="true">
                            {item.checked ? "✓" : ""}
                        </span>
                        {item.label}
                        {item.children && (
                            <svg className="desktop-tile-menu__arrow" width="16" height="16" viewBox="0 0 16 16" aria-hidden="true">
                                <path d="m6 4 4 4-4 4" fill="none" stroke="currentColor" />
                            </svg>
                        )}
                    </button>
                </li>
            </Fragment>
        ));

    return (
        <>
            <menu
                ref={root}
                className="desktop-tile-menu"
                role="menu"
                aria-label="格子菜单"
                style={position}
                data-hit-id="tile-menu"
                data-hit-kind="component"
                data-hit-priority="100"
                onPointerDown={(event) => event.stopPropagation()}
                onKeyDown={(event) => handleKeyDown(event, false)}
            >
                {renderItems(items, false)}
            </menu>
            {active !== undefined && items[active]?.children && (
                <menu
                    ref={child}
                    className="desktop-tile-menu"
                    role="menu"
                    aria-label={items[active].label}
                    style={childPosition}
                    data-hit-id="tile-submenu"
                    data-hit-kind="component"
                    data-hit-priority="101"
                    onPointerEnter={() => clearTimeout(timer.current)}
                    onPointerDown={(event) => event.stopPropagation()}
                    onKeyDown={(event) => handleKeyDown(event, true)}
                >
                    {renderItems(items[active].children, true)}
                </menu>
            )}
        </>
    );
};
