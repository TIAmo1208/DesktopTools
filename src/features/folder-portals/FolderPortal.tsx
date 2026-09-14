import { useCallback, useEffect, useImperativeHandle, useMemo, useState, type MouseEvent, type Ref } from "react";

import {
    deleteFolderPortalItems,
    getFolderPortalSnapshot,
    openFolderPortalPath,
    showFolderPortalMenu,
    renameFolderPortalItem,
    setFolderPortalClipboard,
    pasteFolderPortalItems,
} from "../../bridge/folderPortal";
import type { FolderPortalItem, FolderPortalSnapshot } from "../../contracts/folderPortal";
import { publishMessage } from "../notifications/messageNotification";
import "./FolderPortal.css";

/** 当前门户内容公开给其公共标题和菜单的操作。 */
export interface FolderPortalActions {
    refresh: () => Promise<void>; // 刷新当前实例目录。
    reset: () => void; // 当前实例回到绑定目录。
    toggleLayout: () => void; // 切换当前实例的显示方式。
    clipboard: (cut: boolean) => Promise<void>; // 复制或剪切当前选择。
    paste: () => Promise<void>; // 粘贴到当前目录。
}

/** 文件夹门户内容视图，负责导航、排序、选择和空闲状态呈现。 */
export const FolderPortal = ({
    rootPath,
    monitorId,
    onCurrentPathChange,
    initialPath = rootPath,
    ref,
}: {
    rootPath: string;
    monitorId: string; // 原生菜单的坐标所属宿主。
    onCurrentPathChange?: (path: string) => void; // 同步原生投放目标的当前目录。
    initialPath?: string;
    ref?: Ref<FolderPortalActions>; // 只向所属外壳暴露操作。
}) => {
    const [snapshot, setSnapshot] = useState<FolderPortalSnapshot>(); // 当前目录快照。
    const [currentPath, setCurrentPath] = useState(initialPath); // 当前会话目录，不持久化子目录。
    const [viewMode, setViewMode] = useState<"list" | "tile">("list"); // 当前门户呈现方式。
    const [selected, setSelected] = useState<string[]>([]); // 当前目录选择集合。
    const [loading, setLoading] = useState(true); // 当前目录读取状态。
    const [rename, setRename] = useState<{ name: string; value: string; pending: boolean }>(); // 当前实例唯一命名输入。
    const [crumbMenuOpen, setCrumbMenuOpen] = useState(false); // 折叠中间层级菜单状态。
    const mergeSnapshot = (next: FolderPortalSnapshot, quiet: boolean) =>
        setSnapshot((previous) => {
            if (!quiet || !previous?.accessible || !next.accessible) return next;
            const items =
                next.items.length === 0 || previous.items.length === 0
                    ? next.items
                    : next.items.map((item) => {
                          const old = previous.items.find((candidate) => candidate.name === item.name);
                          return old && old.isDirectory === item.isDirectory && old.size === item.size && old.modifiedUnix === item.modifiedUnix ? old : item;
                      });
            return { ...next, items };
        });
    useEffect(() => setRename(undefined), [currentPath]); // 导航取消未提交命名。
    useEffect(() => onCurrentPathChange?.(currentPath), [currentPath, onCurrentPathChange]);

    const refresh = useCallback(
        async (path = currentPath, quiet = false) => {
            if (!quiet) setLoading(true);
            try {
                const next = await getFolderPortalSnapshot(rootPath, path);
                mergeSnapshot(next, quiet);
                if (next.accessible) setSelected((current) => current.filter((name) => next.items.some((item) => item.name === name))); // 保留取消或部分失败后仍存在的选择。
                if (!next.accessible)
                    publishMessage({
                        type: "error",
                        content: next.error ?? "无法访问门户目录",
                        source: "folder-portal",
                        dedupeKey: `${rootPath}:${next.error}`,
                    });
            } catch (error) {
                publishMessage({
                    type: "error",
                    content: error instanceof Error ? error.message : "读取门户目录失败",
                    source: "folder-portal",
                    dedupeKey: `${rootPath}:read`,
                });
            } finally {
                if (!quiet) setLoading(false);
            }
        },
        [currentPath, rootPath],
    );
    useEffect(() => {
        void refresh();
    }, [refresh]);
    useEffect(() => {
        const timer = window.setInterval(() => void refresh(currentPath, true), 2000); // ponytail: 静默增量轮询；目录规模或刷新成本显著增加时升级为原生 watcher。
        return () => window.clearInterval(timer);
    }, [currentPath, refresh]);
    useImperativeHandle(
        ref,
        () => ({
            refresh: () => refresh(),
            reset: () => {
                setSelected([]);
                setCurrentPath(rootPath);
            },
            toggleLayout: () => setViewMode((mode) => (mode === "list" ? "tile" : "list")),
            clipboard: async (cut) => {
                if (!selected.length) return;
                const result = await setFolderPortalClipboard(rootPath, currentPath, selected, cut);
                publishMessage({ type: "info", content: result.message, source: "folder-portal" });
            },
            paste: async () => {
                const result = await pasteFolderPortalItems(rootPath, currentPath);
                publishMessage({ type: "info", content: result.message, source: "folder-portal" });
                await refresh();
            },
        }),
        [currentPath, refresh, rootPath, selected],
    );

    const parentPath = useMemo(() => {
        if (currentPath === rootPath) return undefined;
        const parent = currentPath.replace(/[\\/][^\\/]+[\\/]?$/, "");
        return parent.length >= rootPath.length && parent.startsWith(rootPath) ? parent : rootPath;
    }, [currentPath, rootPath]);
    const select = (item: FolderPortalItem, event: MouseEvent) =>
        setSelected((current) => {
            const names = snapshot?.items.map((entry) => entry.name) ?? [];
            if (event.shiftKey && current.length) {
                const anchor = names.indexOf(current[current.length - 1]);
                const target = names.indexOf(item.name);
                if (anchor >= 0 && target >= 0) return names.slice(Math.min(anchor, target), Math.max(anchor, target) + 1);
            }
            if (event.ctrlKey || event.metaKey) return current.includes(item.name) ? current.filter((name) => name !== item.name) : [...current, item.name];
            return [item.name];
        });
    const open = (item: FolderPortalItem) => {
        if (!item.isDirectory) {
            void openFolderPortalPath(rootPath, `${currentPath.replace(/[\\/]$/, "")}/${item.name}`).catch((error) =>
                publishMessage({ type: "error", content: error instanceof Error ? error.message : "打开项目失败", source: "folder-portal" }),
            );
            return;
        }
        setSelected([]);
        setCurrentPath(`${currentPath.replace(/[\\/]$/, "")}/${item.name}`);
    };
    const crumb = currentPath.replace(/\\/g, "/").split("/").filter(Boolean);
    const displayCrumb = crumb.length > 3 ? [crumb[0], "…", ...crumb.slice(-2)] : crumb; // 空间不足时只折叠中间层级。
    const goRoot = () => {
        setSelected([]);
        setCrumbMenuOpen(false);
        setCurrentPath(rootPath);
    }; // 根节点和根目录按钮共用处理。
    const goParent = () => {
        if (parentPath) {
            setSelected([]);
            setCrumbMenuOpen(false);
            setCurrentPath(parentPath);
        }
    }; // 上一层节点和上一级按钮共用处理。
    return (
        <section className={`folder-portal folder-portal--${viewMode}`} aria-label="文件夹门户" onKeyDown={(event) => event.stopPropagation()}>
            <nav className="folder-portal__breadcrumb" aria-label="路径导航">
                <button
                    type="button"
                    disabled={!parentPath}
                    aria-label="上一级"
                    title="上一级"
                    onClick={() => {
                        goParent();
                    }}
                >
                    <svg viewBox="0 0 24 24" aria-hidden="true">
                        <path d="m9 14-5-5 5-5" />
                        <path d="M4 9h10a6 6 0 0 1 6 6v1" />
                    </svg>
                </button>
                <button
                    type="button"
                    aria-label="返回根目录"
                    title="返回根目录"
                    onClick={() => {
                        goRoot();
                    }}
                >
                    <svg viewBox="0 0 24 24" aria-hidden="true">
                        <path d="m3 9 9-7 9 7" />
                        <path d="M5 8v12h14V8" />
                        <path d="M9 20v-6h6v6" />
                    </svg>
                </button>
                <ol className="folder-portal__breadcrumb-list">
                    {displayCrumb.map((part, index) => (
                        <li className="folder-portal__breadcrumb-item" key={`${part}-${index}`}>
                            {part === "…" ? (
                                <span className="folder-portal__breadcrumb-collapse">
                                    <button className="folder-portal__breadcrumb-ellipsis" 
                                        type="button"
                                        aria-label="显示中间层级"
                                        aria-expanded={crumbMenuOpen}
                                        onClick={() => setCrumbMenuOpen((open) => !open)}
                                    >
                                        …
                                    </button>
                                    {crumbMenuOpen && (
                                        <menu className="folder-portal__breadcrumb-menu" role="menu">
                                            {crumb.slice(1, -2).map((middle, middleIndex) => (
                                                <li key={`${middle}-${middleIndex}`} role="none">
                                                    <button
                                                        type="button"
                                                        role="menuitem"
                                                        onClick={() => {
                                                            setSelected([]);
                                                            setCrumbMenuOpen(false);
                                                            setCurrentPath(crumb.slice(0, middleIndex + 2).join("/"));
                                                        }}
                                                    >
                                                        {middle}
                                                    </button>
                                                </li>
                                            ))}
                                        </menu>
                                    )}
                                </span>
                            ) : index === displayCrumb.length - 1 ? (
                                <span className="folder-portal__breadcrumb-page" aria-current="page">{part}</span>
                            ) : (
                                <button
                                    type="button"
                                    onClick={() => {
                                        if (index === 0) goRoot();
                                        else if (index === displayCrumb.length - 2) goParent();
                                        else {
                                            setSelected([]);
                                            setCurrentPath(crumb.slice(0, index + 1).join("/"));
                                        }
                                    }}
                                >
                                    {part}
                                </button>
                            )}
                            {index < displayCrumb.length - 1 && <span className="folder-portal__breadcrumb-separator" aria-hidden="true">›</span>}
                        </li>
                    ))}
                </ol>
            </nav>
            <div
                className="folder-portal__content"
                onKeyDown={(event) => {
                    if (event.target instanceof HTMLInputElement) return;
                    if (event.ctrlKey && event.key.toLowerCase() === "v") {
                        event.preventDefault();
                        void pasteFolderPortalItems(rootPath, currentPath)
                            .then((result) => {
                                publishMessage({ type: "info", content: result.message, source: "folder-portal" });
                                return refresh();
                            })
                            .catch((error) =>
                                publishMessage({ type: "error", content: error instanceof Error ? error.message : "粘贴失败", source: "folder-portal" }),
                            );
                        return;
                    }
                    if (event.ctrlKey && (event.key.toLowerCase() === "c" || event.key.toLowerCase() === "x") && selected.length) {
                        event.preventDefault();
                        void setFolderPortalClipboard(rootPath, currentPath, selected, event.key.toLowerCase() === "x")
                            .then((result) => publishMessage({ type: "info", content: result.message, source: "folder-portal" }))
                            .catch((error) =>
                                publishMessage({ type: "error", content: error instanceof Error ? error.message : "剪贴板操作失败", source: "folder-portal" }),
                            );
                        return;
                    }
                    if (event.key === "F2" && selected.length === 1 && !loading) {
                        event.preventDefault();
                        setRename({ name: selected[0], value: selected[0], pending: false });
                    }
                    if (event.key === "F5") {
                        event.preventDefault();
                        void refresh();
                    }
                    if (event.key === "Delete" && selected.length) {
                        event.preventDefault();
                        void deleteFolderPortalItems(rootPath, currentPath, selected)
                            .then((result) => {
                                publishMessage({
                                    type: result.operationError || result.items.some((item) => !item.succeeded) ? "error" : "info",
                                    content: `${currentPath}：${result.message}`,
                                    source: "folder-portal",
                                });
                                void refresh();
                            })
                            .catch((error) =>
                                publishMessage({ type: "error", content: error instanceof Error ? error.message : "删除项目失败", source: "folder-portal" }),
                            );
                    }
                }}
                tabIndex={0}
            >
                {loading && <p className="folder-portal__state">正在读取目录…</p>}
                {!loading && snapshot && !snapshot.accessible && (
                    <div className="folder-portal__state folder-portal__state--error">
                        <p>{snapshot.error ?? "无法访问绑定目录"}</p>
                        <button type="button" onClick={() => void refresh()}>
                            重试
                        </button>
                    </div>
                )}
                {!loading && snapshot?.accessible && snapshot.items.length === 0 && <p className="folder-portal__state">此文件夹为空</p>}
                {!loading && snapshot?.accessible && (
                    <ul className="folder-portal__items">
                        {snapshot.items.map((item) => (
                            <li
                                key={item.name}
                                className={selected.includes(item.name) ? "is-selected" : undefined}
                                onClick={(event) => select(item, event)}
                                onPointerDown={(event) => {
                                    if (!(event.target instanceof HTMLInputElement))
                                        event.currentTarget.closest<HTMLElement>(".folder-portal__content")?.focus({ preventScroll: true });
                                }}
                                onDoubleClick={() => open(item)}
                                onContextMenu={(event) => {
                                    event.preventDefault();
                                    event.stopPropagation();
                                    const names = selected.includes(item.name) ? selected : [item.name]; // 右键保留已有多选或选中新项目。
                                    setSelected(names);
                                    void showFolderPortalMenu(rootPath, currentPath, names, monitorId, event.clientX, event.clientY)
                                        .then(() => refresh())
                                        .catch((error) =>
                                            publishMessage({
                                                type: "error",
                                                content: error instanceof Error ? error.message : "打开原生菜单失败",
                                                source: "folder-portal",
                                            }),
                                        );
                                }}
                            >
                                <span
                                    className={`folder-portal__icon ${item.isDirectory ? "folder-portal__icon--folder" : "folder-portal__icon--file"}`}
                                    aria-hidden="true"
                                />
                                {rename?.name === item.name ? (
                                    <input
                                        aria-label="项目名称"
                                        className="folder-portal__rename"
                                        autoFocus
                                        value={rename.value}
                                        disabled={rename.pending}
                                        onFocus={(event) =>
                                            event.currentTarget.setSelectionRange(
                                                0,
                                                item.isDirectory || item.name.lastIndexOf(".") <= 0 ? item.name.length : item.name.lastIndexOf("."),
                                            )
                                        }
                                        onChange={(event) => setRename({ ...rename, value: event.target.value })}
                                        onClick={(event) => event.stopPropagation()}
                                        onDoubleClick={(event) => event.stopPropagation()}
                                        onKeyDown={(event) => {
                                            event.stopPropagation();
                                            if (event.key === "Escape") {
                                                event.preventDefault();
                                                setRename(undefined);
                                            }
                                            if (event.key !== "Enter" || event.nativeEvent.isComposing || rename.pending) return;
                                            event.preventDefault();
                                            const input = rename; // 异步结果关联本次提交。
                                            setRename({ ...input, pending: true });
                                            void renameFolderPortalItem(rootPath, currentPath, input.name, input.value)
                                                .then((result) => {
                                                    const failed = result.aborted || result.operationError || result.items.some((entry) => !entry.succeeded); // 取消和部分失败均保留输入。
                                                    publishMessage({ type: failed ? "error" : "info", content: result.message, source: "folder-portal" });
                                                    setRename((active) =>
                                                        active?.name !== input.name ? active : failed ? { ...input, pending: false } : undefined,
                                                    );
                                                    return refresh();
                                                })
                                                .catch((error) => {
                                                    setRename((active) => (active?.name === input.name ? { ...input, pending: false } : active));
                                                    publishMessage({
                                                        type: "error",
                                                        content: error instanceof Error ? error.message : typeof error === "string" ? error : "重命名失败",
                                                        source: "folder-portal",
                                                    });
                                                });
                                        }}
                                    />
                                ) : (
                                    <span>{item.name}</span>
                                )}
                            </li>
                        ))}
                    </ul>
                )}
            </div>
        </section>
    );
};
