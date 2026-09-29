import { useContext, useRef, type ComponentPropsWithoutRef, type ReactNode } from "react";
import { DesktopTileContext } from "../DesktopTileBase";

import "./DesktopTileHeader.css";

/** 公共标题栏提供编辑入口和由内容组合的工具区。 */
interface DesktopTileHeaderProps extends Omit<ComponentPropsWithoutRef<"header">, "title"> {
    title: string; // 当前标题，允许空字符串。
    subtitle?: string; // 可选的折叠态摘要。
    editing: boolean; // 宿主管理的编辑状态。
    onRenameEnd: () => void; // 退出标题编辑。
    onRename: (title: string) => Promise<unknown>; // 保存成功后关闭编辑器。
    onRenameStart?: () => void; // 可选的双击和键盘重命名入口。
    commands?: ReactNode | ((sortMenuOpen: boolean) => ReactNode); // 内容工具可读取公共排序菜单状态。
}

/** 共用标题编辑的取消、输入法与保存边界。 */
export const DesktopTileHeader = ({
    title,
    subtitle,
    editing,
    onRenameEnd,
    onRename,
    onRenameStart,
    commands,
    onPointerDown,
    ...props
}: DesktopTileHeaderProps) => {
    const cancelled = useRef(false); // Escape 取消时阻止失焦提交。
    const shell = useContext(DesktopTileContext); // 外壳持有显隐状态，标题只负责呈现。
    return (
        <header
            {...props}
            onPointerDown={(event) => {
                shell.onHeaderPointerDown?.(event);
                if (!event.defaultPrevented) onPointerDown?.(event);
            }}
            className="desktop-tile-header"
            data-visible={shell.titleVisible}
            data-title-mode={shell.titleMode}
            data-button-mode={shell.buttonMode}
        >
            {editing ? (
                <input
                    className="desktop-tile-header__editor"
                    aria-label="格子名称"
                    defaultValue={title}
                    autoFocus
                    onPointerDown={(event) => event.stopPropagation()}
                    onFocus={(event) => {
                        cancelled.current = false;
                        event.currentTarget.select();
                    }}
                    onKeyDown={(event) => {
                        event.stopPropagation();
                        if (event.key === "Escape") {
                            cancelled.current = true;
                            onRenameEnd();
                        }
                        if (event.key === "Enter" && !event.nativeEvent.isComposing) event.currentTarget.blur();
                    }}
                    onBlur={(event) => {
                        if (cancelled.current) return;
                        const nextTitle = event.currentTarget.value.trim(); // 标题只去除首尾空白。
                        if (nextTitle === title) {
                            onRenameEnd();
                            return;
                        }
                        void onRename(nextTitle)
                            .then(onRenameEnd)
                            .catch(() => undefined);
                    }}
                />
            ) : (
                <div className="desktop-tile-header__text" data-has-subtitle={Boolean(subtitle)}>
                    <h2
                        tabIndex={onRenameStart ? 0 : undefined}
                        onDoubleClick={onRenameStart}
                        onKeyDown={(event) => {
                            if (event.key !== "F2" || !onRenameStart) return;
                            event.stopPropagation();
                            onRenameStart();
                        }}
                    >
                        {title || "\u00a0"}
                    </h2>
                    {subtitle && <p className="desktop-tile-header__subtitle">{subtitle}</p>}
                </div>
            )}
            {commands && <div className="desktop-tile-header__commands">{typeof commands === "function" ? commands(shell.sortMenuOpen) : commands}</div>}
        </header>
    );
};
