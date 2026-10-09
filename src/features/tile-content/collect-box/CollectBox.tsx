import { useEffect, useMemo, useRef, useState, type MouseEvent, type PointerEvent } from "react";
import { createPortal } from "react-dom";
import { usePreferences } from "../../settings/PreferencesProvider";
import { DesktopTileMenu } from "../../tiles/DesktopTileMenu";
import { DesktopTileViewport } from "../../tiles/DesktopTileViewport";
import Vditor from "vditor";
import "vditor/dist/index.css";
import {
    createCollectItem,
    deleteCollectItem,
    getCollectItems,
    listenCollectChanges,
    openCollectEditor,
    openCollectItem,
    readCollectImage,
    openCollectLink,
    restoreCollectItem,
    setCollectItemPinned,
    type CollectItem,
} from "../../../bridge/collect";
import { startDesktopItemDrag, cancelDesktopItemDragPreparation } from "../../../bridge/desktopItems";
import { publishMessage } from "../../notifications/messageNotification";
import { settingsErrorMessage } from "../../../bridge/settings";
import { prepareCollectDrag } from "../../../bridge/collect";
import {
    readCollectBoxState,
    mergeCollectItems,
    orderCollectItems,
    filterCollectItems,
    type CollectBoxFilter,
    type CollectBoxLayout,
} from "./collectItemState";
import type { DesktopTileMenuItem } from "../../tiles/DesktopTileMenu";
import { collectExcerpt } from "./collectMarkdown";
import pinIcon from "../../../../resources/collect-pin.svg?raw";
import "./CollectBox.css";

/** 收集箱操作失败保留当前数据，并通过已有消息管道说明原因。 */
const handleFailure = (error: unknown) => publishMessage({ type: "error", content: settingsErrorMessage(error), source: "collect-box" });

const title = (item: CollectItem) => item.content.match(/^#{1,6}\s+(.+)$/m)?.[1]?.trim() || item.name.replace(/\.md$/i, "");

/** 原生读取结果仅用于显示，不把磁盘路径交给浏览器导航。 */
const CollectImage = ({ item, collapsed, width = 78, enlarged = false }: { item: CollectItem; collapsed: boolean; width?: number; enlarged?: boolean }) => {
    const container = useRef<HTMLDivElement>(null); // 固定图像槽用于可见性和物理尺寸观测。
    const [source, setSource] = useState<string>();
    const [failed, setFailed] = useState(false); // 失败和加载共用固定图像区域。
    useEffect(() => {
        let active = true;
        setSource(undefined);
        setFailed(!item.available);
        const element = container.current; // 生命周期内保持同一观测节点。
        if (!element || collapsed || !item.available || !item.path) return;
        let visible = false; // 首次命中视口后才读取。
        let requested = -1; // 相同显示尺寸或 GIF 原图不重复请求。
        let generation = 0; // 丢弃旧尺寸和离屏后的返回。
        const request = () => {
            if (!visible || window.__DESKTOP_TOOLS_NATIVE__?.isRefreshPaused?.()) return;
            const bounds = element.getBoundingClientRect(); // 与桌面图像使用相同的 DPI 换算。
            const pixels =
                enlarged && /\.gif$/i.test(item.path!)
                    ? 0
                    : Math.max(1, Math.min(enlarged ? 768 : 384, Math.ceil(Math.max(bounds.width, bounds.height) * window.devicePixelRatio))); // GIF 悬停复用原图读取，0 仅作为前端请求标识。
            if (pixels === requested) return;
            requested = pixels;
            const current = ++generation; // 当前读取代次。
            void readCollectImage(item.name, item.path!, pixels || undefined)
                .then((value) => {
                    if (active && visible && current === generation) {
                        setSource(value);
                        setFailed(false);
                    }
                })
                .catch((error: unknown) => {
                    if (active && visible && current === generation) {
                        setFailed(true);
                        handleFailure(error);
                    }
                });
        };
        const intersection = new IntersectionObserver(([entry]) => {
            visible = entry?.isIntersecting ?? false;
            if (visible) request();
            else {
                generation++;
                requested = -1;
                setSource(undefined);
            }
        }); // 离屏释放前端图片，后端仍使用既有有界缓存。
        const resize = new ResizeObserver(request); // 不轮询尺寸。
        intersection.observe(element);
        resize.observe(element);
        window.addEventListener("resize", request);
        window.addEventListener("desktop-refresh-changed", request);
        return () => {
            active = false;
            intersection.disconnect();
            resize.disconnect();
            window.removeEventListener("resize", request);
            window.removeEventListener("desktop-refresh-changed", request);
        };
    }, [item.name, item.path, item.available, collapsed, enlarged]);
    return (
        <div
            ref={container}
            className={enlarged ? "collect-box__image collect-box__image--enlarged" : "collect-box__image"}
            style={enlarged ? undefined : { width, flexBasis: width, height: (width * 2) / 3 }}
        >
            {source ? (
                <img
                    draggable={false}
                    className="collect-box__image-content"
                    src={source}
                    alt={item.displayName}
                    onError={() => {
                        setSource(undefined);
                        setFailed(true);
                    }}
                />
            ) : (
                <div className="collect-box__image-content collect-box__image--placeholder" role="img" aria-label={failed ? "图片不可用" : "图片正在加载"}>
                    {failed ? "图片不可用" : "正在加载"}
                </div>
            )}
        </div>
    );
};

/** 折叠标题轨道可调用的收集箱命令。 */
export interface CollectBoxToolbarActions {
    ready: boolean; // 首次读取成功后才启用写入命令。
    create: () => void;
    sortItems: DesktopTileMenuItem[]; // 与围栏相同的公共排序菜单。
    layoutItems: DesktopTileMenuItem[]; // 格子右键菜单排列选项。
    restore: () => void; // 格子菜单恢复最近删除项。
}

/** 呈现共享收集箱的最小条目列表。 */
export const CollectBox = ({
    onLatestChange,
    onToolbarReady,
    instanceState,
    onStateChange,
    onBeginInputDrag,
    onEndInputDrag,
    collapsed = false,
}: {
    collapsed?: boolean; // 使用外壳实际展开状态，折叠时不操作隐藏选择。
    instanceState?: unknown;
    onStateChange?: (state: { layout: CollectBoxLayout; filters: CollectBoxFilter }) => Promise<void>;
    onBeginInputDrag?: () => Promise<void>;
    onEndInputDrag?: () => Promise<void>;
    onLatestChange?: (value: { title: string; summary: string }) => void;
    onToolbarReady?: (actions: CollectBoxToolbarActions) => void;
}) => {
    const { preferences } = usePreferences(); // 与设置页共用权威偏好和变更事件。
    const [items, setItems] = useState<CollectItem[]>([]);
    const [ready, setReady] = useState(false); // 刷新不清空已加载内容。
    const readyRef = useRef(false); // 长期事件回调读取最新就绪状态。
    const [readError, setReadError] = useState<string>(); // 首次失败不能显示为空集合。
    const [itemMenu, setItemMenu] = useState<{ x: number; y: number; name: string }>(); // 当前条目的局部右键菜单。
    const [preview, setPreview] = useState<{ x: number; y: number; item: CollectItem }>(); // 当前悬浮预览。
    const [sort, setSort] = useState<"time" | "name">("time");
    const { layout, filters } = useMemo(() => readCollectBoxState(instanceState), [instanceState]);
    const [savingView, setSavingView] = useState(false);
    const saveView = (next: { layout: CollectBoxLayout; filters: CollectBoxFilter }) => {
        if (!onStateChange || savingView) return;
        setSavingView(true);
        void onStateChange(next)
            .catch(handleFailure)
            .finally(() => setSavingView(false));
    };
    const setLayout = (next: CollectBoxLayout) => saveView({ layout: next, filters });
    const setFilters = (next: CollectBoxFilter | ((current: CollectBoxFilter) => CollectBoxFilter)) =>
        saveView({ layout, filters: typeof next === "function" ? next(filters) : next });
    const selectionAnchor = useRef<string | undefined>(undefined);
    const [selected, setSelected] = useState<string[]>([]);
    const dragCleanup = useRef<(() => void) | undefined>(undefined);
    const suppressDragClick = useRef(false); // 拖动后的点击不得改变选择或打开条目。
    useEffect(() => () => dragCleanup.current?.(), []);
    const handleDragPointerDown = (event: PointerEvent<HTMLElement>, item: CollectItem) => {
        if (event.button !== 0 || !readyRef.current || collapsed || !item.available || !onBeginInputDrag || !onEndInputDrag) return;
        dragCleanup.current?.();
        suppressDragClick.current = false;
        const names = ordered
            .filter((entry) => (selected.includes(item.name) ? selected.includes(entry.name) : entry.name === item.name))
            .map((entry) => entry.name); // 导出按可见列表顺序。
        const start = { x: event.clientX, y: event.clientY };
        let moved = false;
        let finished = false;
        let preparing = false; // 每次指针会话只允许一份准备请求。
        let prepared: Awaited<ReturnType<typeof prepareCollectDrag>> | undefined;
        const cleanup = () => {
            if (finished) return;
            finished = true;
            window.removeEventListener("pointermove", move, true);
            window.removeEventListener("pointerup", finish, true);
            window.removeEventListener("pointercancel", finish, true);
            window.removeEventListener("keydown", escape);
            window.removeEventListener("blur", cleanup);
            if (prepared) void cancelDesktopItemDragPreparation(prepared.token).catch(() => undefined);
            if (dragCleanup.current === cleanup) dragCleanup.current = undefined;
        };
        const startDrag = () => {
            if (!prepared || !moved || finished) return;
            const token = prepared.token;
            prepared = undefined;
            void onBeginInputDrag()
                .then(() => {
                    if (!finished) return startDesktopItemDrag(token, { left: 0, top: 0 });
                })
                .catch(handleFailure)
                .finally(() => {
                    void cancelDesktopItemDragPreparation(token).catch(() => undefined);
                    void onEndInputDrag().catch(handleFailure);
                    cleanup();
                });
        };
        const move = (next: globalThis.PointerEvent) => {
            if (next.pointerId !== event.pointerId) return;
            moved ||= Math.abs(next.clientX - start.x) > 4 || Math.abs(next.clientY - start.y) > 4;
            if (moved && !preparing) {
                preparing = true;
                suppressDragClick.current = true;
                closePreview();
                setSelected(names);
                void prepareCollectDrag(names)
                    .then((value) => {
                        if (finished) {
                            if (value) void cancelDesktopItemDragPreparation(value.token).catch(() => undefined);
                        } else {
                            prepared = value ?? undefined;
                            if (prepared) startDrag();
                            else cleanup();
                        }
                    })
                    .catch((error: unknown) => {
                        cleanup();
                        handleFailure(error);
                    });
            }
        };
        const finish = (next: globalThis.PointerEvent) => {
            if (next.pointerId === event.pointerId) cleanup();
        };
        const escape = (next: KeyboardEvent) => {
            if (next.key === "Escape") cleanup();
        };
        dragCleanup.current = cleanup;
        window.addEventListener("pointermove", move, true);
        window.addEventListener("pointerup", finish, true);
        window.addEventListener("pointercancel", finish, true);
        window.addEventListener("keydown", escape);
        window.addEventListener("blur", cleanup);
    };
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
        setReadError(undefined);
        void getCollectItems()
            .then((next) => {
                if (revision !== readRevision.current) return;
                setItems((current) => mergeCollectItems(current, next, deletedName));
                readyRef.current = true;
                setReady(true);
            })
            .catch((error: unknown) => {
                if (revision === readRevision.current) {
                    setReadError(settingsErrorMessage(error));
                    handleFailure(error);
                }
            });
    };
    const closePreview = () => {
        clearTimeout(hoverTimer.current);
        setPreview(undefined);
    };
    useEffect(() => {
        window.addEventListener("blur", closePreview);
        window.addEventListener("desktop-tile-menu-opening", closePreview);
        return () => {
            window.removeEventListener("blur", closePreview);
            window.removeEventListener("desktop-tile-menu-opening", closePreview);
        };
    }, []);
    const handlePreviewEnter = (event: PointerEvent<HTMLElement>, item: CollectItem) => {
        if (collapsed || !item.available || dragCleanup.current || itemMenu || pendingDelete) return;
        clearTimeout(hoverTimer.current);
        const bounds = event.currentTarget.getBoundingClientRect();
        const x = bounds.right + 8 + 520 <= window.innerWidth ? bounds.right + 8 : Math.max(8, bounds.left - 528);
        const y = Math.max(8, Math.min(bounds.top, window.innerHeight - 428));
        hoverTimer.current = setTimeout(() => setPreview({ item, x, y }), 500);
    };
    const [pendingDelete, setPendingDelete] = useState<string[]>();
    const [deleting, setDeleting] = useState(false);
    const deletingRef = useRef(false); // 重复点击和关闭回调即时读取提交状态。
    const deleteDialog = useRef<HTMLDialogElement>(null); // 原生模态表面负责焦点约束。
    const deleteCancel = useRef<HTMLButtonElement>(null); // 清理确认默认聚焦取消。
    useEffect(() => {
        const dialog = deleteDialog.current; // 保持本次模态实例。
        if (!pendingDelete || !dialog) return;
        const previous = document.activeElement as HTMLElement | null; // 关闭后还原列表焦点。
        dialog.showModal();
        deleteCancel.current?.focus();
        return () => {
            dialog.close();
            previous?.focus();
        };
    }, [pendingDelete]);
    useEffect(() => {
        if (!collapsed) return;
        closePreview();
        dragCleanup.current?.();
        setItemMenu(undefined);
        if (!deletingRef.current) setPendingDelete(undefined);
    }, [collapsed]);
    const handleDelete = (names: string[]) => {
        if (deletingRef.current || !readyRef.current) return;
        deletingRef.current = true;
        setDeleting(true);
        void (async () => {
            for (const name of names) {
                try {
                    await deleteCollectItem(name);
                    setSelected((current) => current.filter((entry) => entry !== name));
                    reload(name);
                } catch (error) {
                    handleFailure(error);
                }
            }
        })().finally(() => {
            deletingRef.current = false;
            setDeleting(false);
            setPendingDelete(undefined);
        });
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
            readyRef.current = false;
            window.removeEventListener("desktop-refresh-changed", handleResume);
            void subscription.then((stop) => stop()).catch(() => undefined);
            clearTimeout(hoverTimer.current);
        };
    }, []);
    createRef.current = () => {
        if (!readyRef.current) return;
        void createCollectItem()
            .then((item) => {
                setItems((current) => [item, ...current.filter((entry) => entry.name !== item.name)]);
                return openCollectEditor(item.name);
            })
            .catch(handleFailure);
    };
    useEffect(() => {
        onToolbarReady?.({
            ready,
            create: () => createRef.current(),
            sortItems: [
                { label: "按名称排序", radio: true, checked: sort === "name", onSelect: () => setSort("name") },
                { label: "按时间排序", radio: true, checked: sort === "time", onSelect: () => setSort("time") },
                { label: "显示类型", heading: true, separatorBefore: true },
                ...(["text", "image", "file"] as const).map((kind) => ({
                    label: kind === "text" ? "文本" : kind === "image" ? "图片" : "文件映射",
                    checked: filters[kind],
                    disabled: savingView,
                    closeOnSelect: false,
                    onSelect: () => setFilters((current) => ({ ...current, [kind]: !current[kind] })),
                })),
                {
                    label: "重置筛选",
                    separatorBefore: true,
                    disabled: savingView,
                    closeOnSelect: false,
                    onSelect: () => setFilters({ text: true, image: true, file: true }),
                },
            ],
            layoutItems: [
                { label: "混排", radio: true, checked: layout === "mixed", disabled: savingView, onSelect: () => setLayout("mixed") },
                { label: "分离", radio: true, checked: layout === "separated", disabled: savingView, onSelect: () => setLayout("separated") },
            ],
            restore: () => {
                if (!readyRef.current) return;
                void restoreCollectItem()
                    .then(() => reload())
                    .catch(handleFailure);
            },
        });
    }, [onToolbarReady, sort, layout, filters, savingView, ready]);
    useEffect(() => setSelected((current) => current.filter((name) => items.some((item) => item.name === name && filters[item.kind]))), [items, filters]);
    const visible = filterCollectItems(items, filters);
    const ordered = orderCollectItems(visible, layout, sort);
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
        onLatestChange?.(
            latest
                ? {
                      title: latest.kind === "text" ? title(latest) : latest.displayName,
                      summary: latest.kind === "text" ? collectExcerpt(latest.content) : latest.kind === "image" ? "图片" : "文件映射",
                  }
                : { title: "收集箱", summary: "" },
        );
    }, [items, onLatestChange]);
    return (
        <div
            ref={rootRef}
            className="official-component__collect"
            data-collect-box="true"
            data-collect-ready={ready ? "true" : "false"}
            onClickCapture={handleLinkClick}
            onAuxClickCapture={handleLinkClick}
            onKeyDown={(event) => {
                if (collapsed || !(event.target as HTMLElement).closest('[role="listbox"]')) return;
                if (event.key === "Escape") {
                    event.preventDefault();
                    event.stopPropagation();
                    closePreview();
                    setSelected([]);
                    selectionAnchor.current = undefined;
                    return;
                }
                if (event.key === "Delete" && selected.length && ready) {
                    event.preventDefault();
                    event.stopPropagation();
                    closePreview();
                    setPendingDelete(selected);
                    return;
                }
                if (["ArrowUp", "ArrowDown", "Home", "End"].includes(event.key)) {
                    event.preventDefault();
                    event.stopPropagation();
                    closePreview();
                    const entries = [...(rootRef.current?.querySelectorAll<HTMLElement>('[role="option"]') ?? [])]; // 可见顺序决定焦点移动。
                    const index = entries.indexOf(document.activeElement as HTMLElement);
                    entries[
                        event.key === "Home"
                            ? 0
                            : event.key === "End"
                              ? entries.length - 1
                              : Math.max(0, Math.min(entries.length - 1, index + (event.key === "ArrowDown" ? 1 : -1)))
                    ]?.focus();
                    return;
                }
                if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "a") {
                    event.preventDefault();
                    event.stopPropagation();
                    closePreview();
                    setSelected(ordered.map((item) => item.name));
                    return;
                }
                if (event.key !== "ContextMenu" && !(event.shiftKey && event.key === "F10")) return;
                const target = (event.target as HTMLElement).closest<HTMLElement>("[data-collect-name]"); // 条目菜单快捷键不交给格子外壳。
                if (!target?.dataset.collectName) return;
                event.preventDefault();
                event.stopPropagation();
                closePreview();
                window.dispatchEvent(new Event("desktop-tile-menu-opening"));
                const bounds = target.getBoundingClientRect(); // 键盘打开时以当前条目为锚点。
                if (!selected.includes(target.dataset.collectName)) setSelected([target.dataset.collectName]);
                setItemMenu({ x: bounds.left, y: bounds.bottom, name: target.dataset.collectName });
            }}
            onContextMenu={(event) => {
                const target = (event.target as HTMLElement).closest<HTMLElement>("[data-collect-name]");
                if (!target || target.dataset.collectName === undefined) return;
                event.preventDefault();
                event.stopPropagation();
                closePreview();
                window.dispatchEvent(new Event("desktop-tile-menu-opening"));
                if (!selected.includes(target.dataset.collectName)) setSelected([target.dataset.collectName]);
                setItemMenu({ x: event.clientX, y: event.clientY, name: target.dataset.collectName });
            }}
        >
            {!ready ? (
                <div className="collect-box__empty" role={readError ? "alert" : "status"}>
                    <p>{readError ? "无法读取收集箱" : "正在读取收集项"}</p>
                    {readError && (
                        <>
                            <p>{readError}</p>
                            <button type="button" onClick={() => reload()}>
                                重试
                            </button>
                        </>
                    )}
                </div>
            ) : items.length === 0 ? (
                <div className="collect-box__empty">
                    <p>暂无收集项</p>
                </div>
            ) : ordered.length === 0 ? (
                <div className="collect-box__empty">
                    <p>无匹配收集项</p>
                    <button type="button" onClick={() => setFilters({ text: true, image: true, file: true })}>
                        重置筛选
                    </button>
                </div>
            ) : (
                <DesktopTileViewport as="ul" role="listbox" aria-label="收集项" aria-multiselectable="true" tabIndex={0}>
                    {ordered.map((item, index) => (
                        <li
                            key={item.name}
                            data-collect-name={item.name}
                            data-kind={item.kind}
                            data-unavailable={!item.available || undefined}
                            title={item.kind === "file" ? (item.path ?? item.displayName) : item.kind === "text" ? title(item) : item.displayName}
                            role="option"
                            data-group-start={layout === "separated" && index > 0 && ordered[index - 1].kind !== item.kind ? "true" : undefined}
                            tabIndex={0}
                            aria-selected={selected.includes(item.name)}
                            className={selected.includes(item.name) ? "collect-box__item collect-box__item--selected" : "collect-box__item"}
                            onPointerDown={(event) => handleDragPointerDown(event, item)}
                            onPointerEnter={(event) => {
                                if (item.kind !== "file") handlePreviewEnter(event, item);
                            }}
                            onPointerLeave={closePreview}
                            onClick={(event) => {
                                if (suppressDragClick.current) return;
                                closePreview();
                                if (event.shiftKey && selectionAnchor.current) {
                                    const anchor = ordered.findIndex((entry) => entry.name === selectionAnchor.current);
                                    const target = ordered.findIndex((entry) => entry.name === item.name);
                                    if (anchor >= 0) {
                                        const range = ordered.slice(Math.min(anchor, target), Math.max(anchor, target) + 1).map((entry) => entry.name);
                                        setSelected((current) => (event.ctrlKey ? [...new Set([...current, ...range])] : range));
                                        return;
                                    }
                                }
                                selectionAnchor.current = item.name;
                                setSelected((current) =>
                                    event.ctrlKey || event.metaKey
                                        ? current.includes(item.name)
                                            ? current.filter((name) => name !== item.name)
                                            : [...current, item.name]
                                        : [item.name],
                                );
                            }}
                            onDoubleClick={() => {
                                if (suppressDragClick.current) return;
                                if (item.available) void openCollectItem(item.name).catch(handleFailure);
                            }}

                            onKeyDown={(event) => {
                                if (event.key === "Enter" && item.available) {
                                    event.preventDefault();
                                    void openCollectItem(item.name).catch(handleFailure);
                                }
                                if (event.key === " ") {
                                    event.preventDefault();
                                    event.stopPropagation();
                                    closePreview();
                                    setSelected((current) =>
                                        current.includes(item.name) ? current.filter((name) => name !== item.name) : [...current, item.name],
                                    );
                                }
                            }}
                        >
                            {item.kind === "image" && <CollectImage item={item} collapsed={collapsed} width={preferences?.collectImagePreviewWidth ?? 78} />}
                            <div className="collect-box__item-body">
                                <div className="official-component__collect-title">
                                    {item.pinned && <span className="collect-box__icon" aria-label="已置顶" dangerouslySetInnerHTML={{ __html: pinIcon }} />}
                                    <span className="collect-box__title-text">
                                        {item.kind === "text" ? title(item) : item.displayName}
                                        {!item.available && "（不可用）"}
                                    </span>
                                </div>
                                <div className="official-component__collect-summary-text">
                                    {!item.available
                                        ? "源路径无法访问"
                                        : item.kind === "image"
                                          ? "图片"
                                          : item.kind === "file"
                                            ? item.path || item.displayName
                                            : collectExcerpt(item.content) || "（空白收集项）"}
                                </div>
                            </div>
                        </li>
                    ))}
                </DesktopTileViewport>
            )}
            <div className="collect-box__drop-band" aria-label="新建收集项投放区域" />
            <div className="collect-box__status" role="status">
                {!ready
                    ? readError
                        ? "读取失败"
                        : "正在读取"
                    : selected.length
                      ? `已选 ${selected.length} / ${ordered.length} 项`
                      : ordered.length !== items.length
                        ? `显示 ${ordered.length} / ${items.length} 项`
                        : `${items.length} 项`}
            </div>
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
                            item?.focus();
                        }}
                        items={[
                            {
                                label: "打开",
                                disabled: !items.some((item) => item.name === itemMenu.name && item.available),
                                onSelect: () => void openCollectItem(itemMenu.name).catch(handleFailure),
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
                                disabled: !items.some((item) => item.name === itemMenu.name),
                                onSelect: () => setPendingDelete(selected.includes(itemMenu.name) ? selected : [itemMenu.name]),
                            },
                        ]}
                    />,
                    document.body,
                )}
            {pendingDelete &&
                createPortal(
                    <div className="collect-box__modal-hit" data-hit-id="collect-delete-backdrop" data-hit-kind="component" data-hit-priority="110">
                        <dialog
                            ref={deleteDialog}
                            className="collect-box__delete-dialog"
                            aria-label={`删除这 ${pendingDelete.length} 个收集项？`}
                            onPointerDown={(event) => event.stopPropagation()}
                            onKeyDown={(event) => event.stopPropagation()}
                            onCancel={(event) => {
                                event.preventDefault();
                                if (!deletingRef.current) setPendingDelete(undefined);
                            }}
                        >
                            <h2>删除这 {pendingDelete.length} 个收集项？</h2>
                            <p>
                                将从所有收集箱移除。
                                {(["text", "image", "file"] as const)
                                    .map(
                                        (kind) =>
                                            `${kind === "text" ? "文本" : kind === "image" ? "图片" : "文件映射"} ${items.filter((item) => item.kind === kind && pendingDelete.includes(item.name)).length} 项`,
                                    )
                                    .join("、")}
                                。
                            </p>
                            <p>文件映射不删除源文件。</p>
                            <div className="collect-box__delete-actions">
                                <button ref={deleteCancel} type="button" disabled={deleting} onClick={() => setPendingDelete(undefined)}>
                                    取消
                                </button>
                                <button type="button" disabled={deleting} onClick={() => handleDelete(pendingDelete)}>
                                    {deleting ? "正在删除" : "删除"}
                                </button>
                            </div>
                        </dialog>
                    </div>,
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
                        {preview.item.kind === "image" ? (
                            <CollectImage key={preview.item.name} item={preview.item} collapsed={collapsed} enlarged />
                        ) : (
                            <div ref={previewContentRef} className="collect-box__preview-content" />
                        )}
                    </div>,
                    document.body,
                )}
        </div>
    );
};
