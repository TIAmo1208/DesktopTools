import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { DesktopTileMenu } from "../../tiles/DesktopTileMenu";
import Markdown from "markdown-to-jsx";
import {
    createCollectItem,
    deleteCollectItem,
    getCollectItems,
    listenCollectChanges,
    openCollectEditor,
    restoreCollectItem,
    setCollectItemPinned,
    type CollectItem,
} from "../../../bridge/collect";
import { usePreferences } from "../../settings/PreferencesProvider";
import { settingsErrorMessage } from "../../../bridge/settings";
import { publishMessage } from "../../notifications/messageNotification";
import { mergeCollectItems } from "./collectItemState";
import type { DesktopTileMenuItem } from "../../tiles/DesktopTileMenu";
import editIcon from "../../../../resources/collect-edit.svg?raw";
import pinIcon from "../../../../resources/collect-pin.svg?raw";
import expandIcon from "../../../../resources/fence-toggle-expand.svg?raw";
import collapseIcon from "../../../../resources/fence-toggle-collapse.svg?raw";
import "./CollectBox.css";

/** 收集箱操作失败保留当前数据，并通过已有消息管道说明原因。 */
const handleFailure = (error: unknown) => publishMessage({ type: "error", content: settingsErrorMessage(error), source: "collect-box" });

const excerpt = (content: string) =>
    content
        .replace(/^#{1,6}\s+[^\r\n]*(?:\r?\n|$)/m, "")
        .replace(/```[\s\S]*?```/g, "")
        .replace(/<[^>]*>/g, "")
        .replace(/\s+/g, " ")
        .trim()
        .slice(0, 240);
const title = (item: CollectItem) => item.content.match(/^#{1,6}\s+(.+)$/m)?.[1]?.trim() || item.name.replace(/\.md$/i, "");

/** 折叠标题轨道可调用的收集箱命令。 */
export interface CollectBoxToolbarActions {
    create: () => void;
    sortItems: DesktopTileMenuItem[]; // 与围栏相同的公共排序菜单。
    restore: () => void; // 格子菜单恢复最近删除项。
}

/** 呈现共享收集箱的最小条目列表。 */
export const CollectBox = ({
    onLatestChange,
    onToolbarReady,
}: {
    onLatestChange?: (value: { title: string; summary: string }) => void;
    onToolbarReady?: (actions: CollectBoxToolbarActions) => void;
}) => {
    const [items, setItems] = useState<CollectItem[]>([]);
    const [expanded, setExpanded] = useState<string>();
    const [itemMenu, setItemMenu] = useState<{ x: number; y: number; name: string }>(); // 当前条目的局部右键菜单。
    const [sort, setSort] = useState<"time" | "name">("time");
    const rootRef = useRef<HTMLDivElement>(null);
    const { preferences } = usePreferences();
    const createRef = useRef<() => void>(() => undefined); // 折叠标题轨道调用的新建动作。
    const readRevision = useRef(0); // 拒绝删除前或较早事件触发的读取。
    const reload = (deletedName?: string) => {
        const revision = ++readRevision.current; // 当前最新读取。
        if (deletedName) setItems((current) => current.filter((item) => item.name !== deletedName));
        if (window.__DESKTOP_TOOLS_NATIVE__?.isRefreshPaused?.()) return;
        void getCollectItems()
            .then((next) => {
                if (revision === readRevision.current) setItems((current) => mergeCollectItems(current, next, deletedName));
            })
            .catch(handleFailure);
    };
    const handleDelete = (name: string) => {
        void deleteCollectItem(name)
            .then(() => reload(name))
            .catch(handleFailure);
    }; // 后端确认成功后移除本格子缓存，其他格子通过共享事件移除。
    useEffect(() => {
        let active = true; // 监听注册晚于卸载时不再开始读取。
        const subscription = listenCollectChanges(reload, handleFailure);
        void subscription.then(() => {
            if (active) reload();
        });
        const handleResume = () => reload(); // 暂停结束后读取最新事实，不轮询。
        window.addEventListener("desktop-refresh-changed", handleResume);
        return () => {
            active = false;
            readRevision.current++;
            window.removeEventListener("desktop-refresh-changed", handleResume);
            void subscription.then((stop) => stop()).catch(() => undefined);
        };
    }, []);
    createRef.current = () => {
        void createCollectItem()
            .then((item) => {
                setItems((current) => [item, ...current.filter((entry) => entry.name !== item.name)]);
                return openCollectEditor(item.name);
            })
            .catch(handleFailure);
    };
    useEffect(() => {
        onToolbarReady?.({
            create: () => createRef.current(),
            sortItems: [
                { label: "按名称排序", radio: true, checked: sort === "name", onSelect: () => setSort("name") },
                { label: "按时间排序", radio: true, checked: sort === "time", onSelect: () => setSort("time") },
            ],
            restore: () =>
                void restoreCollectItem()
                    .then(() => reload())
                    .catch(handleFailure),
        });
    }, [onToolbarReady, sort]);
    const ordered = [...items].sort((a, b) =>
        b.pinned === a.pinned ? (sort === "time" ? b.createdUnixMs - a.createdUnixMs : a.name.localeCompare(b.name)) : b.pinned ? 1 : -1,
    );
    useEffect(() => {
        const latest = items.reduce<CollectItem | undefined>(
            (current, item) => (!current || item.createdUnixMs > current.createdUnixMs ? item : current),
            undefined,
        );
        onLatestChange?.(latest ? { title: title(latest), summary: excerpt(latest.content) } : { title: "收集箱", summary: "" });
    }, [items, onLatestChange]);
    return (
        <div
            ref={rootRef}
            className="official-component__collect"
            data-collect-box="true"
            onKeyDown={(event) => {
                if (event.key !== "ContextMenu" && !(event.shiftKey && event.key === "F10")) return;
                const target = (event.target as HTMLElement).closest<HTMLElement>("[data-collect-name]"); // 条目菜单快捷键不交给格子外壳。
                if (!target?.dataset.collectName) return;
                event.preventDefault();
                event.stopPropagation();
                window.dispatchEvent(new Event("desktop-tile-menu-opening"));
                const bounds = target.getBoundingClientRect(); // 键盘打开时以当前条目为锚点。
                setItemMenu({ x: bounds.left, y: bounds.bottom, name: target.dataset.collectName });
            }}
            onContextMenu={(event) => {
                const target = (event.target as HTMLElement).closest<HTMLElement>("[data-collect-name]");
                if (!target || target.dataset.collectName === undefined) return;
                event.preventDefault();
                event.stopPropagation();
                window.dispatchEvent(new Event("desktop-tile-menu-opening"));
                setItemMenu({ x: event.clientX, y: event.clientY, name: target.dataset.collectName });
            }}
        >
            {items.length === 0 ? (
                <p>暂无收集项</p>
            ) : (
                <ul>
                    {ordered.map((item) => (
                        <li key={item.name} data-collect-name={item.name}>
                            <button
                                type="button"
                                className="official-component__collect-title"
                                aria-expanded={expanded === item.name}
                                onClick={() => {
                                    setExpanded(expanded === item.name ? undefined : item.name);
                                }}
                            >
                                {item.pinned && <span className="collect-box__icon" aria-label="已置顶" dangerouslySetInnerHTML={{ __html: pinIcon }} />}
                                <span className="collect-box__title-text">{item.available === false ? `${title(item)}（不可用）` : title(item)}</span>
                                <span
                                    className="collect-box__icon"
                                    aria-hidden="true"
                                    dangerouslySetInnerHTML={{ __html: expanded === item.name ? collapseIcon : expandIcon }}
                                />
                            </button>
                            {expanded === item.name && (
                                <div className="official-component__collect-summary">
                                    <div className="official-component__collect-summary-text">
                                        <Markdown options={{ disableParsingRawHTML: true, overrides: { img: { component: () => <span>[图片]</span> } } }}>
                                            {excerpt(item.content) || "（空白收集项）"}
                                        </Markdown>
                                    </div>
                                    <button
                                        type="button"
                                        title="打开编辑窗"
                                        aria-label="打开编辑窗"
                                        disabled={!item.available}
                                        onClick={() => void openCollectEditor(item.name).catch(handleFailure)}
                                    >
                                        <span className="collect-box__icon" aria-hidden="true" dangerouslySetInnerHTML={{ __html: editIcon }} />
                                    </button>
                                </div>
                            )}
                        </li>
                    ))}
                </ul>
            )}
            {itemMenu &&
                createPortal(
                    <DesktopTileMenu
                        left={itemMenu.x}
                        top={itemMenu.y}
                        onClose={() => {
                            setItemMenu(undefined);
                            const item = [...(rootRef.current?.querySelectorAll<HTMLElement>("[data-collect-name]") ?? [])].find(
                                (element) => element.dataset.collectName === itemMenu.name,
                            ); // 恢复原条目键盘焦点。
                            item?.querySelector<HTMLButtonElement>("button")?.focus();
                        }}
                        items={[
                            {
                                label: items.find((item) => item.name === itemMenu.name)?.pinned ? "取消置顶" : "置顶",
                                disabled: !items.some((item) => item.name === itemMenu.name && item.available),
                                onSelect: () => {
                                    const item = items.find((candidate) => candidate.name === itemMenu.name); // 使用当前文件状态。
                                    if (item?.available)
                                        void setCollectItemPinned(item.name, !item.pinned)
                                            .then(() => reload())
                                            .catch(handleFailure);
                                },
                            },
                            {
                                label: "删除",
                                disabled: !items.some((item) => item.name === itemMenu.name && item.available),
                                onSelect: () => handleDelete(itemMenu.name),
                            },
                        ]}
                    />,
                    document.body,
                )}
        </div>
    );
};
