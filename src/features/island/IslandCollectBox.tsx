import { useCallback, useEffect, useRef, useState, type PointerEvent } from "react";
import { cancelDesktopItemDragPreparation, startDesktopItemDrag } from "../../bridge/desktopItems";
import { getCollectItems, listenCollectChanges, openCollectItem, prepareCollectDrag, readCollectImage, type CollectItem } from "../../bridge/collect";
import { settingsErrorMessage } from "../../bridge/settings";
import { islandCollectItems, islandCollectLimit, islandCollectSummary, islandCollectTitle, readIslandCollectCache, writeIslandCollectCache } from "./islandCollect";
import "./islandCollect.css";

type IslandCollectMode = "collapsed" | "hover" | "expanded"; // 与浮岛三态一致，不新增第四种展示档位。

/** 浮岛内的图片只读缩略图；原图与附件归属仍由收集业务维护。 */
const IslandCollectThumb = ({ item, fill = false }: { item: CollectItem; fill?: boolean }) => {
    const [source, setSource] = useState<string>(); // 后端读取的 data URL。
    const container = useRef<HTMLSpanElement>(null);
    useEffect(() => {
        setSource(undefined);
        const element = container.current;
        if (!element || item.kind !== "image" || !item.available || !item.path) return;
        let active = true; // 离屏、切换条目或卸载后拒绝迟到结果。
        let requested = false; // 首次进入可见区后只请求一次。
        const load = () => {
            if (requested || !item.path) return;
            requested = true;
            void readCollectImage(item.name, item.path, 192)
                .then((value) => {
                    if (active) setSource(value);
                })
                .catch(() => {
                    if (active) setSource(undefined);
                });
        };
        const observer = new IntersectionObserver(([entry]) => {
            if (entry?.isIntersecting) {
                load();
                observer.disconnect();
            }
        }); // 展开列表可能很长，只读取进入浮岛视口的缩略图。
        observer.observe(element);
        return () => {
            active = false;
            observer.disconnect();
        };
    }, [item.name, item.path, item.kind, item.available]);
    if (item.kind !== "image") return null;
    return (
        <span ref={container} className={fill ? "island-collect__thumb island-collect__thumb--fill" : "island-collect__thumb"} aria-hidden="true">
            {source ? <img src={source} alt="" draggable={false} /> : null}
        </span>
    );
};

/** 浮岛收集箱卡：胶囊取最新一条，hover 取两条，展开为可拖出、可单击打开的列表。 */
export const IslandCollectBox = ({ mode, collapsed = false, collapsedCount = 3, onDragEnd }: { mode: IslandCollectMode; collapsed?: boolean; collapsedCount?: number; onDragEnd?: () => void }) => {
    const [items, setItems] = useState<CollectItem[]>(() => [...(readIslandCollectCache() ?? [])]); // 预读命中时首帧就有条目。
    const [ready, setReady] = useState(() => readIslandCollectCache() !== undefined); // 首次读取成功前不把失败当成空收集箱。
    const [failure, setFailure] = useState<string>();
    const revision = useRef(0); // 拒绝刷新、删除与卸载后的迟到读取。
    const dragCleanup = useRef<(() => void) | undefined>(undefined); // 当前条目拖出的窗口级清理。
    const suppressClick = useRef(false); // 拖出结束后不把同一次指针释放当成打开。

    const load = useCallback(() => {
        if (window.__DESKTOP_TOOLS_NATIVE__?.isRefreshPaused?.()) return; // 宿主暂停时不重复读取磁盘。
        const current = ++revision.current;
        void getCollectItems()
            .then((next) => {
                if (current !== revision.current) return;
                setItems(next);
                writeIslandCollectCache(next); // 后续展开直接复用最近一次读取结果。
                setReady(true);
                setFailure(undefined);
            })
            .catch((error: unknown) => {
                if (current === revision.current) setFailure(settingsErrorMessage(error));
            });
    }, []);

    useEffect(() => {
        load();
        let active = true; // 异步订阅完成前卸载也必须释放监听。
        let stop: (() => void) | undefined;
        void listenCollectChanges(
            () => load(),
            (error) => setFailure(settingsErrorMessage(error)),
        ).then((unlisten) => {
            if (active) stop = unlisten;
            else unlisten();
        });
        const handleRefresh = () => load();
        window.addEventListener("desktop-refresh-changed", handleRefresh);
        return () => {
            active = false;
            revision.current++;
            stop?.();
            window.removeEventListener("desktop-refresh-changed", handleRefresh);
            dragCleanup.current?.();
        };
    }, [load]);

    const handleOpen = (item: CollectItem) => {
        if (!item.available) return;
        void openCollectItem(item.name).catch((error: unknown) => console.warn("浮岛收集项打开失败", settingsErrorMessage(error)));
    };

    /** 展开态条目拖出：准备完整的收集项表示，再交给系统 OLE 拖动。 */
    const handleItemPointerDown = (event: PointerEvent<HTMLElement>, item: CollectItem) => {
        if (mode !== "expanded" || event.button !== 0 || !item.available) return;
        dragCleanup.current?.();
        suppressClick.current = false;
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
            if (!prepared || finished) return;
            const token = prepared.token;
            prepared = undefined;
            suppressClick.current = true;
            void startDesktopItemDrag(token, { left: 0, top: 0 })
                .catch((error: unknown) => console.warn("浮岛收集项拖出失败", settingsErrorMessage(error)))
                .finally(() => {
                    cleanup();
                    onDragEnd?.(); // 原生拖动期间页面收不到 pointerout，结束后由岛按真实光标位置判态。
                });
        };
        const move = (next: globalThis.PointerEvent) => {
            if (next.pointerId !== event.pointerId || finished) return;
            moved ||= Math.abs(next.clientX - start.x) > 4 || Math.abs(next.clientY - start.y) > 4;
            if (!moved || preparing) return;
            preparing = true;
            suppressClick.current = true;
            void prepareCollectDrag([item.name])
                .then((value) => {
                    if (finished) {
                        if (value) void cancelDesktopItemDragPreparation(value.token).catch(() => undefined);
                        return;
                    }
                    prepared = value ?? undefined;
                    if (prepared) startDrag();
                    else cleanup();
                })
                .catch((error: unknown) => {
                    cleanup();
                    console.warn("浮岛收集项拖出准备失败", settingsErrorMessage(error));
                });
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

    // 加载与空态也占展开卡内容宽，首次实测就得到最终宽度，展开动画结束后不再二次调宽。
    const status = (text: string) =>
        mode === "expanded" ? (
            <div className="island-collect island-collect--expanded">
                <p className="island-collect__status">{text}</p>
            </div>
        ) : (
            <p className="island-collect__status">{text}</p>
        );
    if (!ready && !items.length) return status(failure ?? "正在读取收集箱");
    if (failure && !items.length) return status(failure);
    // 展开态下卡内收起只列最近几条，仍保持 300 DIP 内容宽与三列混排；其余状态沿用三态上限。
    const rows = islandCollectItems(items, mode === "expanded" ? (collapsed ? collapsedCount : islandCollectLimit(mode)) : islandCollectLimit(mode));
    if (mode === "collapsed") {
        const first = rows[0];
        return (
            <div
                className="island-collect island-collect--collapsed"
                data-count={items.length ? String(items.length) : "箱"}
                aria-label={first ? `收集箱，最新：${islandCollectTitle(first)}` : "收集箱为空"}
            >
                <span className="island-collect__line">{first ? islandCollectTitle(first) : "收集箱"}</span>
            </div>
        );
    }
    if (mode === "hover") {
        return (
            <div className="island-collect island-collect--hover" aria-label="收集箱最近条目">
                {rows.length ? rows.map((item) => <span className="island-collect__line" key={item.name}>{islandCollectTitle(item)}</span>) : <span className="island-collect__line">收集箱为空</span>}
            </div>
        );
    }
    return (
        <div className="island-collect island-collect--expanded">
            <div className="island-collect__heading">
                <span>收集箱</span>
                <small>{items.length} 项</small>
            </div>
            {rows.length ? (
                <div className="island-collect__list">
                    {rows.map((item) => (
                        <button
                            type="button"
                            className={item.kind === "image" ? "island-collect__item island-collect__item--image" : "island-collect__item"}
                            key={item.name}
                            disabled={!item.available}
                            data-island-no-drag="true"
                            aria-label={item.available ? `打开 ${islandCollectTitle(item)}` : "收集项不可用"}
                            onPointerDown={(event) => handleItemPointerDown(event, item)}
                            onClick={() => {
                                if (suppressClick.current) {
                                    suppressClick.current = false;
                                    return;
                                }
                                handleOpen(item);
                            }}
                        >
                            {item.kind === "image" ? (
                                <IslandCollectThumb item={item} fill />
                            ) : (
                                <>
                                    <IslandCollectThumb item={item} />
                                    <span className="island-collect__body">
                                        <span className="island-collect__title">{islandCollectTitle(item)}</span>
                                        <small className="island-collect__summary">{islandCollectSummary(item)}</small>
                                    </span>
                                </>
                            )}
                        </button>
                    ))}
                </div>
            ) : (
                <p className="island-collect__status">收集箱为空</p>
            )}
        </div>
    );
};
