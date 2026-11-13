import { useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent, type PointerEvent } from "react";

import type { PopupLayerSpace } from "../../contracts/popupLayer";
import { TileMenuList, type TileMenuPayloadItem } from "../tiles/TileMenuList";

/** 二级菜单展开延时；与宿主内菜单保持一致的手感。 */
const SUBMENU_DELAY_MS = 200;
/** 二级菜单滚动条预留宽度：出现滚动条时宽度会比测量值略增。 */
const SUBMENU_SLACK = 16;

/** 一个父项的二级菜单摆放：相对根菜单顶部的偏移与可用高度。 */
interface ChildPlacement {
    left: number; // 二级菜单在文档里的横坐标，紧贴根菜单。
    top: number; // 相对根菜单顶部。
    height: number; // 已按可用空间收敛的高度。
    width: number; // 二级菜单实测宽度，用来贴边；没有子菜单时为 0。
}

/** 一次测量的结果：根菜单盒、各层联合盒与展开方向。 */
interface MenuLayout {
    rootWidth: number; // 根菜单宽度。
    rootHeight: number; // 根菜单高度。
    childWidth: number; // 二级菜单宽度（含滚动条预留）。
    unionWidth: number; // 所有层级菜单的联合宽度。
    unionHeight: number; // 所有层级菜单的联合高度。
    placements: ChildPlacement[]; // 每个父项的二级菜单摆放，下标与 items 对齐。
    flipped: boolean; // 二级菜单是否向左展开。
    liftUp: boolean; // 锚点下方空间不足时，整个菜单向上展开。
}

/** 弹层页面上报的摆放（DIP）。 */
export interface MenuFrame {
    width: number;
    height: number;
    anchorOffsetX: number;
    anchorOffsetY: number;
}

/**
 * 置顶弹层里的两级菜单：先量后显，再由宿主定位窗口。
 *
 * 与宿主内菜单的差别只在布局来源：这里不按窗口视口收敛，而按锚点四周可用空间决定
 * 二级菜单方向与最大高度，因此窗口尺寸与内容一致，透明区域不额外吞掉点击。
 */
export const PopupMenu = ({
    items,
    available,
    focusFirst = false,
    onLayout,
    onSelect,
    onDismiss,
}: {
    items: TileMenuPayloadItem[]; // 菜单命令的呈现字段。
    available: PopupLayerSpace; // 锚点四周可用空间（DIP）。
    focusFirst?: boolean; // 键盘打开时把焦点放到第一项；指针打开只聚焦菜单容器，不显示选中框。
    onLayout: (frame: MenuFrame) => void; // 测量完成后的窗口摆放。
    onSelect: (indexPath: number[], keepOpen: boolean) => void; // 选中命令：路径，以及命令是否要求菜单保持打开。
    onDismiss: () => void; // 取消关闭。
}) => {
    const root = useRef<HTMLMenuElement>(null); // 一级菜单容器。
    const child = useRef<HTMLMenuElement>(null); // 展开中的二级菜单容器。
    const probes = useRef<(HTMLMenuElement | null)[]>([]); // 隐藏测量的二级菜单容器。
    const parentTops = useRef<number[]>([]); // 每个父项相对根菜单顶部的偏移。
    const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined); // 悬停展开延迟。
    const reported = useRef(""); // 已上报的摆放，避免重复调用原生定位。
    const focusChild = useRef(false); // 键盘或点击展开后把焦点移进二级菜单。
    const [active, setActive] = useState<number>(); // 当前展开的父项。
    const [layout, setLayout] = useState<MenuLayout>(); // 测量后的摆放。
    const onDismissRef = useRef(onDismiss); // 长生命周期监听读取最新关闭回调。
    onDismissRef.current = onDismiss;

    /** 当前层命令：一级直接取 items，二级取展开项的 children。 */
    const entriesOf = (nested: boolean): TileMenuPayloadItem[] => (nested && active !== undefined ? (items[active]?.children ?? []) : items);

    /** 选中命令：父项展开子菜单，其余项直接回传路径。 */
    const handleSelect = (index: number, nested: boolean) => {
        const entries = entriesOf(nested); // 当前层命令。
        const item = entries[index]; // 当前选中项。
        if (!item || item.disabled) return;
        clearTimeout(timer.current);
        if (item.children?.length) {
            if (nested) return; // 二级菜单不再嵌套。
            focusChild.current = true;
            setActive(index);
            return;
        }
        onSelect(nested && active !== undefined ? [active, index] : [index], item.closeOnSelect === false);
    };

    /** 指针进入：一级延时展开，二级不改变父项。 */
    const handleEnterItem = (index: number, nested: boolean) => {
        clearTimeout(timer.current);
        if (nested) return;
        const item = items[index]; // 当前父项。
        timer.current = setTimeout(() => setActive(item?.children?.length ? index : undefined), SUBMENU_DELAY_MS);
    };

    /** 键盘导航：方向键在层内移动，Esc 逐级关闭，Tab 关闭整组。 */
    const handleKeyDown = (event: KeyboardEvent<HTMLMenuElement>, nested: boolean) => {
        event.stopPropagation();
        if (event.key === "Escape" || (nested && event.key === "ArrowLeft")) {
            event.preventDefault();
            if (!nested) onDismissRef.current();
            else if (active !== undefined) {
                const index = active; // 返回前保存父项位置。
                setActive(undefined);
                root.current?.querySelectorAll<HTMLElement>('li[role="none"]')[index]?.querySelector<HTMLButtonElement>("button")?.focus();
            }
            return;
        }
        if (event.key === "Tab") {
            onDismissRef.current();
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

    /** 右方向键展开当前父项并把焦点移进二级菜单。 */
    const handleItemKeyDown = (event: KeyboardEvent<HTMLButtonElement>, index: number, nested: boolean) => {
        const item = entriesOf(nested)[index];
        if (event.key !== "ArrowRight" || !item?.children?.length || item.disabled || nested) return;
        event.preventDefault();
        event.stopPropagation();
        clearTimeout(timer.current);
        focusChild.current = true;
        setActive(index);
    };

    /** 菜单外的按下与窗口失焦都按取消处理。 */
    useEffect(() => {
        const handleOutside = (event: Event) => {
            if (event.target instanceof Node && (root.current?.contains(event.target) || child.current?.contains(event.target))) return;
            onDismissRef.current();
        };
        const handleClose = () => onDismissRef.current(); // 窗口失焦即取消。
        document.addEventListener("pointerdown", handleOutside, true);
        window.addEventListener("blur", handleClose);
        return () => {
            document.removeEventListener("pointerdown", handleOutside, true);
            window.removeEventListener("blur", handleClose);
            clearTimeout(timer.current);
        };
    }, []);

    /** 测量根菜单与全部二级菜单，算出窗口尺寸与展开方向。 */
    useLayoutEffect(() => {
        const element = root.current;
        if (!element) return;
        const rootRect = element.getBoundingClientRect();
        const rootWidth = Math.max(rootRect.width, 1);
        const rootHeight = Math.max(rootRect.height, 1);
        const parents = [...element.querySelectorAll<HTMLElement>('li[role="none"]')]; // 与 items 一一对应（含标题项）。
        const tops = parents.map((parent) => parent.getBoundingClientRect().top - rootRect.top);
        parentTops.current = tops;
        const totalHeight = available.top + available.bottom; // 纵向总可用空间。
        const totalWidth = available.left + available.right; // 横向总可用空间。
        let maxChildWidth = 0; // 最宽二级菜单决定是否需要向左展开与联合宽度。
        const placements: ChildPlacement[] = items.map(() => ({ left: 0, top: 0, height: 0, width: 0 })); // 逐项预算，展开时不再重新测量。
        items.forEach((item, index) => {
            if (!item.children?.length) return;
            const probe = probes.current[index]; // 同一子菜单的隐藏测量副本。
            if (!probe) return;
            const rect = probe.getBoundingClientRect();
            const width = Math.max(rect.width, 1);
            const height = Math.max(rect.height, 1);
            maxChildWidth = Math.max(maxChildWidth, width);
            const top = tops[index] ?? 0;
            const below = Math.max(available.bottom - top, 0); // 父项下方还能放多少。
            const above = Math.max(available.top + top, 0); // 父项上方（含父项本身）能放多少。
            // 优先向下对齐父项；下方放不下而上方更宽裕时改为底边对齐父项向上展开。
            const fits = below >= Math.min(height, below) && below >= above;
            placements[index] = {
                left: 0, // 展开方向确定后再补。
                top: fits ? top : Math.max(top + Math.min(height, 28) - height, 0),
                height: Math.min(height, Math.max(fits ? below : above, 1)),
                width,
            };
        });
        const childWidth = maxChildWidth > 0 ? maxChildWidth + SUBMENU_SLACK : 0; // 滚动条与边框的实际占位。
        const flipped = childWidth > 0 && rootWidth - 1 + childWidth > available.right && available.left >= childWidth;
        // 下方空间放不下根菜单而上方的空间更大时向上展开：窗口底边对齐锚点。
        const liftUp = rootHeight > available.bottom && available.top > available.bottom;
        const rootLeft = flipped ? childWidth : 0; // 向左展开时根菜单右移，给二级菜单让位。
        // 二级菜单紧贴根菜单：左侧展开贴根菜单左边，右侧展开贴根菜单右边（按各自实际宽度收边）。
        for (const placement of placements) {
            if (!placement.width) continue;
            placement.left = flipped ? rootLeft - placement.width + 1 : rootLeft + rootWidth - 1;
        }
        const childBottom = placements.reduce((bottom, placement) => Math.max(bottom, placement.top + placement.height), 0);
        const next: MenuLayout = {
            rootWidth,
            rootHeight,
            childWidth,
            unionWidth: Math.min(rootLeft + rootWidth + (flipped ? 0 : childWidth), Math.max(totalWidth, rootWidth)),
            unionHeight: Math.min(Math.max(rootHeight, childBottom), Math.max(totalHeight, rootHeight)),
            placements,
            flipped,
            liftUp,
        };
        setLayout((current) =>
            current &&
            current.rootWidth === next.rootWidth &&
            current.rootHeight === next.rootHeight &&
            current.childWidth === next.childWidth &&
            current.unionWidth === next.unionWidth &&
            current.unionHeight === next.unionHeight &&
            current.flipped === next.flipped &&
            current.liftUp === next.liftUp &&
            current.placements.every(
                (placement, index) =>
                    placement.left === next.placements[index]?.left &&
                    placement.top === next.placements[index]?.top &&
                    placement.height === next.placements[index]?.height,
            )
                ? current
                : next,
        );
    }, [items, available]);

    /** 根菜单在文档中的横坐标：向左展开时把根菜单右移，给二级菜单让位。 */
    const rootLeft = layout?.flipped ? layout.childWidth : 0;

    /**
     * 上报一次联合盒：创建窗口时就覆盖根菜单与全部二级菜单，展开/收起只切换可见性，
     * 窗口不再改尺寸与坐标，避免二级菜单开合时整窗移动或闪烁。
     */
    useLayoutEffect(() => {
        if (!layout) return;
        const lift = layout.liftUp ? layout.rootHeight : 0; // 向上展开时窗口底边贴锚点。
        const frame = { width: layout.unionWidth, height: layout.unionHeight, anchorOffsetX: rootLeft, anchorOffsetY: lift };
        const key = `${frame.width.toFixed(2)}x${frame.height.toFixed(2)}@${frame.anchorOffsetX.toFixed(2)},${frame.anchorOffsetY.toFixed(2)}`;
        if (reported.current === key) return;
        reported.current = key;
        onLayout(frame);
    }, [layout, rootLeft, onLayout]);

    /** 展开子菜单后只把焦点移进去；位置与尺寸已在创建时预算好。 */
    useLayoutEffect(() => {
        if (active === undefined) return;
        if (focusChild.current) child.current?.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus();
        focusChild.current = false;
    }, [active]);

    /** 命令表刷新后焦点可能落回页面：键盘打开放回第一项，指针打开只回到菜单容器。 */
    useEffect(() => {
        const current = document.activeElement;
        if (current instanceof Node && (root.current?.contains(current) || child.current?.contains(current))) return;
        if (focusFirst) root.current?.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus();
        else root.current?.focus();
    }, [focusFirst, items]);

    /** 窗口显示后设置初始焦点：键盘打开落在第一项，指针打开落在菜单容器（方向键仍然可用）。 */
    useEffect(() => {
        if (!layout) return;
        const focus = () => {
            if (focusFirst) root.current?.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus();
            else root.current?.focus();
        };
        focus();
        window.addEventListener("focus", focus);
        return () => window.removeEventListener("focus", focus);
    }, [layout, focusFirst]);

    /** 两级菜单之间转移不触发取消。 */
    const handleLeave = (event: PointerEvent<HTMLElement>) => {
        const next = event.relatedTarget;
        if (next instanceof Node && (root.current?.contains(next) || child.current?.contains(next))) return;
        clearTimeout(timer.current);
        setActive(undefined);
    };

    return (
        <>
            <TileMenuList
                items={items}
                label="格子菜单"
                menuRef={root}
                style={{ left: rootLeft, top: 0, maxHeight: Math.max(available.bottom, available.top) }}
                activeIndex={active}
                onEnterItem={(index) => handleEnterItem(index, false)}
                onLeaveGroup={handleLeave}
                onPointerDown={(event) => event.stopPropagation()}
                onSelect={(index) => handleSelect(index, false)}
                onItemKeyDown={(event, index) => handleItemKeyDown(event, index, false)}
                onKeyDown={(event) => handleKeyDown(event, false)}
            />
            {active !== undefined && items[active]?.children?.length ? (
                <TileMenuList
                    items={items[active].children ?? []}
                    label={items[active].label}
                    nested
                    menuRef={child}
                    style={{
                        left: layout?.placements[active]?.left ?? 0,
                        top: layout?.placements[active]?.top ?? 0,
                        maxHeight: Math.max(layout?.placements[active]?.height ?? 0, 1),
                    }}
                    onEnterItem={() => clearTimeout(timer.current)}
                    onLeaveGroup={handleLeave}
                    onPointerDown={(event) => event.stopPropagation()}
                    onSelect={(index) => handleSelect(index, true)}
                    onItemKeyDown={(event, index) => handleItemKeyDown(event, index, true)}
                    onKeyDown={(event) => handleKeyDown(event, true)}
                />
            ) : null}
            {/* 隐藏测量副本：二级菜单内容在窗口显示前就要确定尺寸。 */}
            <div className="popup-layer__probe" aria-hidden="true">
                {items.map((item, index) =>
                    item.children?.length ? (
                        <TileMenuList
                            key={item.label}
                            items={item.children}
                            label="测量"
                            nested
                            menuRef={(element) => {
                                probes.current[index] = element;
                            }}
                            onSelect={() => undefined}
                        />
                    ) : null,
                )}
            </div>
        </>
    );
};
