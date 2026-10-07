import type { CSSProperties, MouseEventHandler, PointerEventHandler } from "react";

import type { DesktopItemSnapshot } from "../../contracts/desktopItem";
import { usePreferences } from "../settings/PreferencesProvider";
import { projectDesktopItems, type DesktopItemViewModel } from "./desktopItemViewModel";
import "./DesktopItemGrid.css";
import { registerDesktopItemImage } from "./desktopItemReadiness";
import { useDesktopItemImage } from "./useDesktopItemImage";

/** 定义桌面项目网格接收的快照与错误状态。 */
interface DesktopItemGridProps {
    snapshot?: DesktopItemSnapshot;
    failure?: string;
    desktopHost?: boolean;
    onItemInput?: (itemId: string) => void;
}

/** 定义可在未归类区域和围栏中复用的桌面项目单元。 */
interface DesktopItemTileProps {
    item: DesktopItemViewModel;
    desktopHost?: boolean;
    className?: string;
    style?: CSSProperties;
    onItemInput?: (itemId: string) => void;
    onItemPointerDown?: PointerEventHandler<HTMLLIElement>;
    onItemDoubleClick?: MouseEventHandler<HTMLLIElement>;
    onItemContextMenu?: MouseEventHandler<HTMLLIElement>;
    fenceMemberId?: string;
    selected?: boolean;
    keyboardFocused?: boolean;
    renaming?: boolean;
    renameValue?: string;
    onRenameValueChange?: (value: string) => void;
    onRenameCommit?: () => void;
    onRenameCancel?: () => void;
}

/** 呈现一个由 Windows Shell 提供身份与系统图像的桌面项目。 */
export const DesktopItemTile = ({
    item,
    desktopHost = false,
    className,
    style,
    onItemInput,
    onItemPointerDown,
    onItemDoubleClick,
    onItemContextMenu,
    fenceMemberId,
    selected = false,
    keyboardFocused = false,
    renaming = false,
    renameValue = "",
    onRenameValueChange,
    onRenameCommit,
    onRenameCancel,
}: DesktopItemTileProps) => {
    const { preferences } = usePreferences(); // 当前窗口共享的项目打开方式。
    const image = useDesktopItemImage(item); // 首帧图标与后台缩略图共用节点。
    return (
        <li
            className={`desktop-item${className ? ` ${className}` : ""}`}
            data-availability={item.isAvailable ? "available" : "unavailable"}
            data-item-id={item.key}
            data-selected={selected ? "true" : undefined}
            data-keyboard-focus={keyboardFocused ? "true" : undefined}
            data-hit-id={desktopHost ? item.key : undefined}
            data-hit-kind={desktopHost ? "desktop-item" : undefined}
            data-hit-priority={desktopHost ? "70" : undefined}
            data-fence-member-id={fenceMemberId}
            style={style}
            onPointerDown={(event) => {
                onItemInput?.(item.key);
                onItemPointerDown?.(event);
            }}
            onDoubleClick={(event) => {
                if (renaming || event.ctrlKey || event.shiftKey || event.altKey || event.metaKey) return;
                if (preferences?.doubleClickToOpen ?? true) onItemDoubleClick?.(event);
            }}
            onContextMenu={onItemContextMenu}
        >
            <div ref={image.container} className="desktop-item__image" aria-hidden="true">
                {image.source && <img ref={registerDesktopItemImage} src={image.source} alt="" draggable={false} />}
            </div>
            {renaming ? (
                <input
                    className="desktop-item__rename"
                    value={renameValue}
                    aria-label={`重命名 ${item.label}`}
                    autoFocus
                    onFocus={(event) => event.currentTarget.select()}
                    onChange={(event) => onRenameValueChange?.(event.currentTarget.value)}
                    onPointerDown={(event) => event.stopPropagation()}
                    onBlur={(event) => {
                        if (event.currentTarget.dataset.cancelled !== "true") onRenameCommit?.();
                    }}
                    onKeyDown={(event) => {
                        if (event.key === "Enter") {
                            event.preventDefault();
                            event.currentTarget.blur();
                        } else if (event.key === "Escape") {
                            event.preventDefault();
                            event.currentTarget.dataset.cancelled = "true";
                            onRenameCancel?.();
                            event.currentTarget.blur();
                        }
                    }}
                />
            ) : (
                <span className="desktop-item__label" title={item.label}>
                    {item.label}
                </span>
            )}
            {!item.isAvailable && <span className="desktop-item__status">等待恢复</span>}
        </li>
    );
};

/** 呈现 Windows Shell 提供的真实桌面项目集合。 */
export const DesktopItemGrid = ({ snapshot, failure, desktopHost = false, onItemInput }: DesktopItemGridProps) => {
    // 当前快照对应的页面展示模型。
    const items = snapshot ? projectDesktopItems(snapshot) : [];

    return (
        <section className={`desktop-items${desktopHost ? " desktop-items--desktop-host" : ""}`} aria-labelledby="desktop-items-title">
            <header className="desktop-items__header">
                <div>
                    <p className="desktop-items__eyebrow">Windows Shell</p>
                    <h2 id="desktop-items-title">桌面项目</h2>
                </div>
                <p className="desktop-items__summary">
                    {items.length} 个项目 · 修订 {snapshot?.revision ?? "--"}
                </p>
            </header>

            {failure && (
                <p className="desktop-items__error" role="alert">
                    {failure}
                </p>
            )}

            {snapshot && items.length === 0 && <p className="desktop-items__empty">桌面当前没有可显示项目</p>}

            <ul className="desktop-items__grid">
                {items.map((item) => (
                    <DesktopItemTile key={item.key} item={item} desktopHost={desktopHost} onItemInput={onItemInput} />
                ))}
            </ul>
        </section>
    );
};
