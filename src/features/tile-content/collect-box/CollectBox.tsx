import { useEffect, useRef, useState, type MouseEvent, type PointerEvent } from "react";
import { createPortal } from "react-dom";
import { DesktopTileMenu } from "../../tiles/DesktopTileMenu";
import Vditor from "vditor";
import "vditor/dist/index.css";
import {
    createCollectItem,
    deleteCollectItem,
    getCollectItems,
    listenCollectChanges,
    openCollectEditor,
    openCollectLink,
    restoreCollectItem,
    setCollectItemPinned,
    type CollectItem,
} from "../../../bridge/collect";
import { settingsErrorMessage } from "../../../bridge/settings";
import { publishMessage } from "../../notifications/messageNotification";
import { mergeCollectItems } from "./collectItemState";
import type { DesktopTileMenuItem } from "../../tiles/DesktopTileMenu";
import { collectExcerpt } from "./collectMarkdown";
import pinIcon from "../../../../resources/collect-pin.svg?raw";
import "./CollectBox.css";

/** 收集箱操作失败保留当前数据，并通过已有消息管道说明原因。 */
const handleFailure = (error: unknown) => publishMessage({ type: "error", content: settingsErrorMessage(error), source: "collect-box" });

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
    const [itemMenu, setItemMenu] = useState<{ x: number; y: number; name: string }>(); // 当前条目的局部右键菜单。
    const [preview, setPreview] = useState<{ x: number; y: number; item: CollectItem }>(); // 当前悬浮预览。
    const [sort, setSort] = useState<"time" | "name">("time");
    const rootRef = useRef<HTMLDivElement>(null);
    const previewContentRef = useRef<HTMLDivElement>(null);
    const previewRenderVersion = useRef(0);
    const hoverTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
    const handleLinkClick = (event: MouseEvent) => {
        if (event.button !== 0 && event.button !== 1) return;
        const link = (event.target as Element).closest<HTMLAnchorElement>("a[href]"); // 包含键盘激活和鼠标中键。
        if (!link) return;
        event.preventDefault();
        event.stopPropagation();
        void openCollectLink(link.getAttribute("href") ?? "").catch(handleFailure);
    }; // 链接打开不替换桌面宿主。
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
    const closePreview = () => {
        clearTimeout(hoverTimer.current);
        setPreview(undefined);
    };
    const handlePreviewEnter = (event: PointerEvent<HTMLElement>, item: CollectItem) => {
        clearTimeout(hoverTimer.current);
        const bounds = event.currentTarget.getBoundingClientRect();
        const x = bounds.right + 8 + 520 <= window.innerWidth ? bounds.right + 8 : Math.max(8, bounds.left - 528);
        const y = Math.max(8, Math.min(bounds.top, window.innerHeight - 428));
        hoverTimer.current = setTimeout(() => setPreview({ item, x, y }), 500);
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
            clearTimeout(hoverTimer.current);
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
        const element = previewContentRef.current;
        if (!element || !preview) return;
        let active = true;
        const version = ++previewRenderVersion.current;
        const render = (
            Vditor as unknown as {
                preview: (
                    target: HTMLElement,
                    markdown: string,
                    options: { mode: "dark" | "light"; cdn: string; lang: "zh_CN"; transform: (html: string) => string },
                ) => Promise<void>;
            }
        ).preview;
        const rendered = document.createElement("div");
        element.replaceChildren();
        void render(rendered, preview.item.content || "（空白收集项）", {
            mode: document.documentElement.dataset.theme === "light" ? "light" : "dark",
            cdn: "/vditor",
            lang: "zh_CN",
            transform: (html) => {
                const document = new DOMParser().parseFromString(html, "text/html");
                document.querySelectorAll("img").forEach((image) => {
                    const placeholder = document.createElement("span");
                    const alt = image.getAttribute("alt")?.trim();
                    placeholder.textContent = alt ? `[图片：${alt}]` : "[图片]";
                    image.replaceWith(placeholder);
                });
                return document.body.innerHTML;
            },
        }).then(() => {
            if (active && version === previewRenderVersion.current) {
                element.classList.add("vditor-reset");
                element.replaceChildren(...rendered.childNodes);
            }
        });
        return () => {
            active = false;
            previewRenderVersion.current++;
            element.replaceChildren();
        };
    }, [preview]);
    useEffect(() => {
        const latest = items.reduce<CollectItem | undefined>(
            (current, item) => (!current || item.createdUnixMs > current.createdUnixMs ? item : current),
            undefined,
        );
        onLatestChange?.(latest ? { title: title(latest), summary: collectExcerpt(latest.content) } : { title: "收集箱", summary: "" });
    }, [items, onLatestChange]);
    return (
        <div
            ref={rootRef}
            className="official-component__collect"
            data-collect-box="true"
            onClickCapture={handleLinkClick}
            onAuxClickCapture={handleLinkClick}
            onKeyDown={(event) => {
                if (event.key !== "ContextMenu" && !(event.shiftKey && event.key === "F10")) return;
                const target = (event.target as HTMLElement).closest<HTMLElement>("[data-collect-name]"); // 条目菜单快捷键不交给格子外壳。
                if (!target?.dataset.collectName) return;
                event.preventDefault();
                event.stopPropagation();
                closePreview();
                window.dispatchEvent(new Event("desktop-tile-menu-opening"));
                const bounds = target.getBoundingClientRect(); // 键盘打开时以当前条目为锚点。
                setItemMenu({ x: bounds.left, y: bounds.bottom, name: target.dataset.collectName });
            }}
            onContextMenu={(event) => {
                const target = (event.target as HTMLElement).closest<HTMLElement>("[data-collect-name]");
                if (!target || target.dataset.collectName === undefined) return;
                event.preventDefault();
                event.stopPropagation();
                closePreview();
                window.dispatchEvent(new Event("desktop-tile-menu-opening"));
                setItemMenu({ x: event.clientX, y: event.clientY, name: target.dataset.collectName });
            }}
        >
            {items.length === 0 ? (
                <p>暂无收集项</p>
            ) : (
                <ul>
                    {ordered.map((item) => (
                        <li
                            key={item.name}
                            data-collect-name={item.name}
                            role="button"
                            tabIndex={item.available ? 0 : -1}
                            aria-disabled={!item.available}
                            onPointerEnter={(event) => handlePreviewEnter(event, item)}
                            onPointerLeave={closePreview}
                            onClick={() => {
                                if (item.available) void openCollectEditor(item.name).catch(handleFailure);
                            }}
                            onKeyDown={(event) => {
                                if ((event.key === "Enter" || event.key === " ") && item.available) {
                                    event.preventDefault();
                                    void openCollectEditor(item.name).catch(handleFailure);
                                }
                            }}
                        >
                            <div className="official-component__collect-title">
                                {item.pinned && <span className="collect-box__icon" aria-label="已置顶" dangerouslySetInnerHTML={{ __html: pinIcon }} />}
                                <span className="collect-box__title-text">{item.available === false ? `${title(item)}（不可用）` : title(item)}</span>
                            </div>
                            <div className="official-component__collect-summary-text">{collectExcerpt(item.content) || "（空白收集项）"}</div>
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
                                label: "打开",
                                disabled: !items.some((item) => item.name === itemMenu.name && item.available),
                                onSelect: () => void openCollectEditor(itemMenu.name).catch(handleFailure),
                            },
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
            {preview &&
                createPortal(
                    <div
                        className="collect-box__preview"
                        style={{ left: preview.x, top: preview.y }}
                        onClickCapture={handleLinkClick}
                        onAuxClickCapture={handleLinkClick}
                    >
                        <div ref={previewContentRef} className="collect-box__preview-content" />
                    </div>,
                    document.body,
                )}
        </div>
    );
};
