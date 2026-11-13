import { Fragment, type CSSProperties, type KeyboardEvent, type PointerEvent, type Ref } from "react";

import type { DesktopTileMenuItem } from "./DesktopTileMenu";

/** 单层菜单的呈现属性；行为（悬停延时、键盘导航、关闭）全部由调用方持有。 */
export interface TileMenuListProps {
    items: DesktopTileMenuItem[]; // 当前层的命令项。
    label: string; // 供读屏识别的菜单名。
    nested?: boolean; // 二级菜单：与一级共用标记，但由调用方决定键盘语义。
    activeIndex?: number; // 一级菜单当前展开的父项下标。
    menuRef?: Ref<HTMLMenuElement>; // 调用方定位与测量使用的容器引用。
    style?: CSSProperties; // 调用方计算出的位置与尺寸约束。
    attributes?: Record<string, string>; // `data-hit-*` 等命中图属性。
    onEnterItem?: (index: number) => void; // 指针进入某一项，由调用方决定延时展开。
    onEnterGroup?: () => void; // 指针进入整组菜单。
    onLeaveGroup?: (event: PointerEvent<HTMLElement>) => void; // 指针离开整组菜单。
    onPointerDown?: (event: PointerEvent<HTMLElement>) => void; // 阻止菜单内按下冒泡到外部关闭。
    onSelect: (index: number) => void; // 选中某一项。
    onItemKeyDown?: (event: KeyboardEvent<HTMLButtonElement>, index: number) => void; // 单项方向键（如右开子菜单）。
    onKeyDown?: (event: KeyboardEvent<HTMLMenuElement>) => void; // 方向键与关闭键。
}

/** 渲染一层菜单项；宿主 Portal 与置顶弹层复用同一份标记与无障碍语义。 */
export const TileMenuList = ({
    items,
    label,
    nested = false,
    activeIndex,
    menuRef,
    style,
    attributes,
    onEnterItem,
    onEnterGroup,
    onLeaveGroup,
    onPointerDown,
    onSelect,
    onItemKeyDown,
    onKeyDown,
}: TileMenuListProps) => (
    <menu
        ref={menuRef}
        className="desktop-tile-menu"
        role="menu"
        aria-label={label}
        tabIndex={-1}
        style={style}
        {...attributes}
        onPointerEnter={onEnterGroup}
        onPointerLeave={onLeaveGroup}
        onPointerDown={onPointerDown}
        onKeyDown={onKeyDown}
    >
        {items.map((item, index) => (
            <Fragment key={item.label}>
                {item.separatorBefore && <li className="desktop-tile-menu__separator" role="separator" aria-hidden="true" />}
                <li role="none" onPointerEnter={() => onEnterItem?.(index)}>
                    {item.heading ? (
                        <span className="desktop-tile-menu__heading">{item.label}</span>
                    ) : (
                        <button
                            type="button"
                            role={item.checked === undefined ? "menuitem" : item.radio ? "menuitemradio" : "menuitemcheckbox"}
                            className={item.danger ? "desktop-tile-menu__danger" : undefined}
                            aria-checked={item.checked}
                            disabled={item.disabled && item.closeOnSelect !== false}
                            aria-disabled={item.disabled}
                            title={item.description}
                            aria-description={item.description}
                            aria-haspopup={item.children ? "menu" : undefined}
                            aria-expanded={item.children ? (nested ? undefined : activeIndex === index) : undefined}
                            data-expanded={!nested && activeIndex === index ? "true" : undefined}
                            onKeyDown={(event) => onItemKeyDown?.(event, index)}
                            onClick={() => onSelect(index)}
                        >
                            <span className="desktop-tile-menu__check" aria-hidden="true">
                                {item.checked ? (item.radio ? "●" : "✓") : ""}
                            </span>
                            {item.label}
                            {item.children && (
                                <svg className="desktop-tile-menu__arrow" width="16" height="16" viewBox="0 0 16 16" aria-hidden="true">
                                    <path d="m6 4 4 4-4 4" fill="none" stroke="currentColor" />
                                </svg>
                            )}
                        </button>
                    )}
                </li>
            </Fragment>
        ))}
    </menu>
);

/** 菜单项的可序列化副本；宿主把命令发到弹层时不能携带闭包。 */
export interface TileMenuPayloadItem {
    label: string;
    checked?: boolean;
    radio?: boolean;
    disabled?: boolean;
    description?: string;
    children?: TileMenuPayloadItem[];
    closeOnSelect?: boolean;
    separatorBefore?: boolean;
    heading?: boolean;
    danger?: boolean;
}

/** 去掉回调，只留呈现字段；子菜单递归同样处理。 */
export const menuPayload = (items: DesktopTileMenuItem[]): TileMenuPayloadItem[] =>
    items.map((item) => ({
        label: item.label,
        ...(item.checked === undefined ? {} : { checked: item.checked }),
        ...(item.radio === undefined ? {} : { radio: item.radio }),
        ...(item.disabled === undefined ? {} : { disabled: item.disabled }),
        ...(item.description === undefined ? {} : { description: item.description }),
        ...(item.children === undefined ? {} : { children: menuPayload(item.children) }),
        ...(item.closeOnSelect === undefined ? {} : { closeOnSelect: item.closeOnSelect }),
        ...(item.separatorBefore === undefined ? {} : { separatorBefore: item.separatorBefore }),
        ...(item.heading === undefined ? {} : { heading: item.heading }),
        ...(item.danger === undefined ? {} : { danger: item.danger }),
    }));

/** 按命令路径取出宿主侧的原闭包，业务逻辑不迁进弹层。 */
export const menuItemAtPath = (items: DesktopTileMenuItem[], path: readonly number[]): DesktopTileMenuItem | undefined => {
    let current: DesktopTileMenuItem | undefined;
    let level = items;
    for (const index of path) {
        current = level[index];
        if (!current) return undefined;
        level = current.children ?? [];
    }
    return current;
};
