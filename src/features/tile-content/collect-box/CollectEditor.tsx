import { useEffect, useRef, useState, type MouseEvent } from "react";
import { createPortal } from "react-dom";
import Vditor from "vditor";
import "vditor/dist/index.css";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
    getCollectItems,
    importCollectRenderedResource,
    openCollectLink,
    readCollectImage,
    renameCollectItem,
    saveCollectItem,
    writeCollectResource,
} from "../../../bridge/collect";
import { usePreferences } from "../../settings/PreferencesProvider";
import { settingsErrorMessage } from "../../../bridge/settings";
import { publishMessage } from "../../notifications/messageNotification";
import { pastedLink } from "./collectEditorLinks";
import { configureCollectImageMarkdown, observeCollectImages } from "./collectEditorImages";
import pencilIcon from "../../../../resources/editor-pencil.svg?raw";
import checkIcon from "../../../../resources/editor-check.svg?raw";
import closeIcon from "../../../../resources/editor-x.svg?raw";
import errorIcon from "../../../../resources/editor-circle-alert.svg?raw";
import headingIcon from "../../../../resources/editor-heading.svg?raw";
import boldIcon from "../../../../resources/editor-bold.svg?raw";
import italicIcon from "../../../../resources/editor-italic.svg?raw";
import strikeIcon from "../../../../resources/editor-strikethrough.svg?raw";
import linkIcon from "../../../../resources/editor-link.svg?raw";
import listIcon from "../../../../resources/editor-list.svg?raw";
import todoIcon from "../../../../resources/editor-list-todo.svg?raw";
import quoteIcon from "../../../../resources/editor-quote.svg?raw";
import lineIcon from "../../../../resources/editor-minus.svg?raw";
import tableIcon from "../../../../resources/editor-table.svg?raw";
import undoIcon from "../../../../resources/editor-undo-2.svg?raw";
import redoIcon from "../../../../resources/editor-redo-2.svg?raw";
import "./CollectEditor.css";

/** 按需窗口中的 Vditor Markdown 编辑器。 */
export const CollectEditor = () => {
    const name = new URLSearchParams(window.location.search).get("name") ?? ""; // 当前文件身份。
    const currentName = useRef(name); // 编辑过程中可变的当前文件身份。
    const host = useRef<HTMLDivElement>(null); // 编辑器挂载节点。
    const editor = useRef<Vditor | undefined>(undefined); // 已初始化编辑器。
    const dialog = useRef<HTMLDialogElement>(null); // 原生模态对话框管理焦点和背景交互。
    const renameInput = useRef<HTMLInputElement>(null); // 按需命名时聚焦并选中主名。
    const renameTrigger = useRef<HTMLButtonElement>(null); // 退出命名后恢复入口焦点。
    const dirty = useRef(false); // 输入时同步更新，关闭请求不读取过期 React 状态。
    const pending = useRef(false); // 保存与关闭共用互斥状态。
    const allowClose = useRef(false); // 保存成功或明确放弃后允许原生关闭继续。
    const readyRef = useRef(false); // 关闭回调读取最新的编辑器就绪状态，避免捕获初始渲染值。
    const [saved, setSaved] = useState(true); // 保存按钮状态。
    const [ready, setReady] = useState(false); // 完成编辑器异步初始化后才允许保存。
    const [busy, setBusy] = useState(false); // 异步操作期间禁用对话框按钮。
    const [failure, setFailure] = useState<{ message: string; operation: "load" | "save" | "rename" | "other" }>(); // 分开呈现读取、保存与命名失败。
    const [operation, setOperation] = useState<"save" | "rename">(); // 区分保存中与改名中的状态文案。
    const [closeFailure, setCloseFailure] = useState(false); // 仅在关闭确认仍打开时呈现关闭保存失败。
    const [renaming, setRenaming] = useState(false); // 文件名默认静态呈现，仅按需显示输入。
    const [fileName, setFileName] = useState(name); // 已确认的文件名，与输入草稿分离。
    const [errorSlot, setErrorSlot] = useState<HTMLElement>(); // Vditor 工具栏下的 React 错误插槽。
    const [itemName, setItemName] = useState(name.replace(/\.md$/i, "")); // 标题栏中的可编辑文件名。
    const [imageMenu, setImageMenu] = useState<{ x: number; y: number; reference: string }>(); // 当前图片菜单位置和引用。
    const { preferences } = usePreferences(); // 当前退出策略。
    const autoSave = useRef(false); // 关闭订阅保持稳定并读取最新设置。
    autoSave.current = preferences?.collectAutoSaveOnExit === true;

    /** 失败不丢弃内容，并沿用应用消息通知。 */
    const handleFailure = (error: unknown, operation: "load" | "save" | "rename" | "other" = "other") => {
        const message = settingsErrorMessage(error); // 兼容后端字符串和结构化错误。
        setFailure({ message, operation });
        publishMessage({ type: "error", content: message, source: "collect-editor" });
    };

    /** 编辑器正文和链接弹出栏共用系统打开，不导航当前窗口。 */
    const handleLinkClick = (event: MouseEvent) => {
        if (event.button !== 0 && event.button !== 1) return;
        const link = (event.target as Element).closest<HTMLAnchorElement>("a[href]"); // 包含中键打开和键盘激活。
        if (!link) return;
        event.preventDefault();
        event.stopPropagation();
        void openCollectLink(link.getAttribute("href") ?? "").catch(handleFailure);
    };

    /** 串行保存；保存成功才销毁窗口，失败恢复编辑能力。 */
    const handleSave = async (close = false) => {
        const current = editor.current; // 保存期间持有同一编辑器。
        if (!readyRef.current || !current || pending.current) return;
        pending.current = true;
        setBusy(true);
        setOperation("save");
        current.disabled();
        try {
            await saveCollectItem(currentName.current, current.getValue());
            dirty.current = false;
            setSaved(true);
            setFailure(undefined);
            setCloseFailure(false);
            if (close) {
                allowClose.current = true;
                await getCurrentWindow().destroy();
            }
        } catch (error) {
            allowClose.current = false;
            setCloseFailure(close);
            handleFailure(error, "save");
        } finally {
            pending.current = false;
            setBusy(false);
            setOperation(undefined);
            if (editor.current === current) current.enable();
        }
    };
    /** 重命名 Markdown 文件及其旁侧资源目录。 */
    const handleRename = async () => {
        const next = itemName.trim(); // 去除主名前后的空白。
        const current = editor.current; // 重命名期间固定编辑器实例并锁定输入。
        if (!readyRef.current || !current || !next || next === currentName.current.replace(/\.md$/i, "") || pending.current) return;
        pending.current = true;
        setBusy(true);
        setOperation("rename");
        current.disabled();
        const wasDirty = dirty.current; // 改名不等于保存或丢弃现有草稿。
        try {
            const nextName = next.endsWith(".md") ? next : `${next}.md`;
            const renamedDraft = await renameCollectItem(currentName.current, nextName, current.getValue()); // 与磁盘共享同一资源引用转换。
            currentName.current = nextName;
            current.setValue(renamedDraft, true); // 旧撤销历史含旧目录，不能在改名后重新写回。
            dirty.current = wasDirty;
            setSaved(!wasDirty);
            setItemName(nextName.replace(/\.md$/i, ""));
            setFileName(nextName);
            setRenaming(false);
            setFailure(undefined);
            requestAnimationFrame(() => renameTrigger.current?.focus());
        } catch (error) {
            handleFailure(error, "rename");
        } finally {
            pending.current = false;
            setBusy(false);
            setOperation(undefined);
            if (editor.current === current) current.enable();
        }
    };

    /** 取消命名只还原文件名输入，不影响正文草稿。 */
    const handleCancelRename = () => {
        if (pending.current) return;
        setItemName(currentName.current.replace(/\.md$/i, ""));
        setRenaming(false);
        setFailure((current) => (current?.operation === "rename" ? undefined : current));
        requestAnimationFrame(() => renameTrigger.current?.focus());
    };
    /** 取消关闭确认后恢复编辑页面的错误呈现。 */
    const handleCancelClose = () => {
        setCloseFailure(false);
        dialog.current?.close();
    };
    useEffect(() => {
        if (!renaming) return;
        renameInput.current?.focus();
        renameInput.current?.select();
    }, [renaming]);

    /** 显式放弃或已无改动时关闭，不触发再次保存。 */
    const handleDiscard = async () => {
        if (pending.current) return;
        pending.current = true;
        setBusy(true);
        try {
            allowClose.current = true;
            await getCurrentWindow().destroy();
        } catch (error) {
            allowClose.current = false;
            handleFailure(error);
        } finally {
            pending.current = false;
            setBusy(false);
        }
    };
    /** 插入链接后把选择范围放在可编辑的链接名称上。 */
    const focusLinkName = (url: string) => {
        requestAnimationFrame(() => {
            const links = [...(host.current?.querySelectorAll("a") ?? [])].filter((link) => link.getAttribute("href") === url);
            const link = links.at(-1);
            if (!link) return;
            const range = document.createRange();
            range.selectNodeContents(link);
            const selection = window.getSelection();
            selection?.removeAllRanges();
            selection?.addRange(range);
        });
    };
    /** 将已渲染图片收入资源目录，并同步替换 Markdown 引用。 */
    const handleImageImport = async (mode: "copy" | "move") => {
        const target = imageMenu;
        if (!target || pending.current) return;
        setImageMenu(undefined);
        try {
            const reference = target.reference.startsWith(window.location.origin)
                ? decodeURIComponent(new URL(target.reference).pathname.replace(/^\//, ""))
                : target.reference;
            const path = await importCollectRenderedResource(currentName.current, reference, mode);
            const current = editor.current;
            if (!current) return;
            current.setValue(current.getValue().split(target.reference).join(path));
            dirty.current = true;
            setSaved(false);
        } catch (error) {
            handleFailure(error);
        }
    };
    useEffect(() => {
        let active = true; // StrictMode 清理后拒绝迟到读取。
        void getCollectItems()
            .then((items) => {
                if (!active || !host.current) return;
                const item = items.find((item) => item.name === currentName.current); // 读取失败不能当作空文件覆盖保存。
                if (!item?.available) throw new Error("收集项不可用，文件可能已被删除或重命名");
                editor.current = new Vditor(host.current, {
                    value: "", // 先安装图片转义修正，再解析原始正文。
                    mode: "wysiwyg",
                    lang: "zh_CN",
                    cdn: "/vditor",
                    _lutePath: "/vditor/dist/js/lute/lute.min.js",
                    cache: { enable: false },
                    toolbar: [
                        { name: "headings", icon: headingIcon },
                        { name: "bold", icon: boldIcon },
                        { name: "italic", icon: italicIcon },
                        { name: "strike", icon: strikeIcon },
                        "|",
                        { name: "link", icon: linkIcon },
                        { name: "list", icon: listIcon },
                        { name: "check", icon: todoIcon },
                        { name: "quote", icon: quoteIcon },
                        { name: "line", icon: lineIcon },
                        { name: "table", icon: tableIcon },
                        "|",
                        { name: "undo", icon: undoIcon },
                        { name: "redo", icon: redoIcon },
                    ],
                    input: () => {
                        dirty.current = true;
                        setSaved(false);
                    },
                    after: () => {
                        if (!active || !host.current) return;
                        const current = editor.current; // Lute 初始化完成后才可安装实例级渲染器。
                        if (!current) return;
                        configureCollectImageMarkdown(current.vditor.lute);
                        current.setValue(item.content, true);
                        dirty.current = false;
                        setSaved(true);
                        const slot = document.createElement("div"); // 由本组件拥有的错误插槽，不参与正文序列化。
                        slot.className = "collect-editor__error-slot";
                        host.current.insertBefore(slot, host.current.querySelector(".vditor-content"));
                        setErrorSlot(slot);
                        readyRef.current = true;
                        setReady(true);
                    },
                });
                const handlePaste = (event: ClipboardEvent) => {
                    const image = event.clipboardData?.files[0];
                    if (image?.type.startsWith("image/")) {
                        event.preventDefault();
                        void image
                            .arrayBuffer()
                            .then((buffer) => writeCollectResource(currentName.current, image.name || "pasted-image.png", [...new Uint8Array(buffer)]))
                            .then((path) => editor.current?.insertMD(`![${image.name || "图片"}](${path})`))
                            .catch(handleFailure);
                        return;
                    }
                    const clipboard = event.clipboardData;
                    const link = pastedLink(clipboard?.getData("text/plain") ?? "") ?? pastedLink(clipboard?.getData("text/html") ?? "");
                    if (link) {
                        event.preventDefault();
                        editor.current?.insertMD(`[${link.label}](${link.url})`);
                        focusLinkName(link.url);
                    }
                };
                host.current.addEventListener("paste", handlePaste);
                const mountedHost = host.current;
                const originalDestroy = editor.current.destroy.bind(editor.current);
                editor.current.destroy = () => {
                    mountedHost.removeEventListener("paste", handlePaste);
                    originalDestroy();
                };
            })
            .catch((error) => {
                if (active) handleFailure(error, "load");
            });
        return () => {
            active = false;
            readyRef.current = false;
            editor.current?.destroy();
            editor.current = undefined;
        };
    }, [name]);
    useEffect(() => {
        if (!ready || !host.current) return;
        return observeCollectImages(host.current, fileName, readCollectImage, handleFailure);
    }, [ready, fileName]);
    useEffect(() => {
        if (!ready || !editor.current) return;
        const current = editor.current; // 仅同步本窗口当前编辑器的内容主题。
        const handleTheme = () => {
            const dark = document.documentElement.dataset.theme === "dark"; // 消费 PreferencesProvider 已解析的主题。
            current.setTheme(dark ? "dark" : "classic", dark ? "dark" : "light");
        }; // 主题变化被动响应，不新增系统主题监听或轮询。
        handleTheme();
        const observer = new MutationObserver(handleTheme); // 只观察主题属性，正文修改不会触发。
        observer.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });
        return () => observer.disconnect();
    }, [ready]);
    useEffect(() => {
        let active = true; // 退订完成前的回调不销毁窗口。
        const close = getCurrentWindow().onCloseRequested(async (event) => {
            if (allowClose.current) return;
            event.preventDefault();
            if (!active) return;
            if (pending.current) return;
            if (!dirty.current) {
                allowClose.current = true;
                await handleDiscard();
                return;
            }
            if (autoSave.current) {
                await handleSave(true);
                return;
            }
            if (!dialog.current?.open) dialog.current?.showModal();
        }); // 只订阅一次，输入变化不会产生无保护的关闭空档。
        void close.catch((error) => {
            if (active) handleFailure(error);
        });
        return () => {
            active = false;
            void close.then((stop) => stop()).catch(() => undefined);
        };
    }, [name]);
    const status = !ready
        ? failure?.operation === "load"
            ? "读取失败"
            : "读取中…"
        : operation === "save"
          ? "保存中…"
          : operation === "rename"
            ? "重命名中…"
            : failure?.operation === "save"
              ? "保存失败"
              : saved
                ? "已保存"
                : "未保存"; // 状态不依赖文件名输入或普通错误。
    const errorMessage = failure && (
        <p role="alert" className="collect-editor__error">
            <span className="collect-editor__icon" aria-hidden="true" dangerouslySetInnerHTML={{ __html: errorIcon }} />
            <span>{failure.message}</span>
        </p>
    ); // 同一错误视图复用于工具栏插槽与关闭确认。
    return (
        <main className="collect-editor" aria-busy={busy}>
            <header className="collect-editor__header">
                <div className="collect-editor__name">
                    {renaming ? (
                        <form
                            className="collect-editor__rename"
                            onSubmit={(event) => {
                                event.preventDefault();
                                void handleRename();
                            }}
                        >
                            <input
                                ref={renameInput}
                                aria-label="收集项名称"
                                aria-invalid={failure?.operation === "rename" || undefined}
                                value={itemName}
                                onChange={(event) => setItemName(event.currentTarget.value)}
                                disabled={busy}
                                onKeyDown={(event) => {
                                    if (event.key === "Escape" && !event.nativeEvent.isComposing) {
                                        event.preventDefault();
                                        handleCancelRename();
                                    }
                                }}
                            />
                            <span className="collect-editor__extension">.md</span>
                            <button
                                className="collect-editor__icon-button"
                                type="submit"
                                title="确认重命名"
                                aria-label="确认重命名"
                                disabled={!ready || busy || !itemName.trim() || itemName.trim() === fileName.replace(/\.md$/i, "")}
                            >
                                <span className="collect-editor__icon" aria-hidden="true" dangerouslySetInnerHTML={{ __html: checkIcon }} />
                            </button>
                            <button
                                className="collect-editor__icon-button"
                                type="button"
                                title="取消重命名"
                                aria-label="取消重命名"
                                disabled={busy}
                                onClick={handleCancelRename}
                            >
                                <span className="collect-editor__icon" aria-hidden="true" dangerouslySetInnerHTML={{ __html: closeIcon }} />
                            </button>
                        </form>
                    ) : (
                        <>
                            <strong className="collect-editor__filename" title={fileName}>
                                {fileName}
                            </strong>
                            <button
                                ref={renameTrigger}
                                className="collect-editor__icon-button"
                                type="button"
                                title="重命名"
                                aria-label="重命名"
                                disabled={!ready || busy}
                                onClick={() => setRenaming(true)}
                            >
                                <span className="collect-editor__icon" aria-hidden="true" dangerouslySetInnerHTML={{ __html: pencilIcon }} />
                            </button>
                        </>
                    )}
                </div>
                <div className="collect-editor__save">
                    <span role="status" className="collect-editor__status" data-error={failure?.operation === "save" || undefined}>
                        {status}
                    </span>
                    <button type="button" onClick={() => void handleSave()} disabled={saved || busy || !ready}>
                        保存
                    </button>
                </div>
            </header>
            {failure?.operation === "rename" && errorMessage}
            {failure?.operation !== "rename" && !closeFailure && (errorSlot ? createPortal(errorMessage, errorSlot) : errorMessage)}
            <div
                ref={host}
                className="collect-editor__content"
                aria-label="Markdown 内容"
                onClickCapture={handleLinkClick}
                onAuxClickCapture={handleLinkClick}
                onContextMenu={(event) => {
                    const target = event.target;
                    if (!(target instanceof HTMLImageElement)) return;
                    event.preventDefault();
                    setImageMenu({ x: event.clientX, y: event.clientY, reference: target.getAttribute("src") ?? target.currentSrc });
                }}
            />
            {imageMenu && (
                <div
                    className="collect-editor__image-menu"
                    style={{ left: imageMenu.x, top: imageMenu.y }}
                    role="menu"
                    onMouseLeave={() => setImageMenu(undefined)}
                >
                    <button type="button" role="menuitem" onClick={() => void handleImageImport("copy")}>
                        复制到资源目录
                    </button>
                    <button type="button" role="menuitem" onClick={() => void handleImageImport("move")}>
                        移动到资源目录
                    </button>
                </div>
            )}
            <dialog
                ref={dialog}
                className="collect-editor__dialog"
                aria-labelledby="collect-exit-title"
                onCancel={(event) => {
                    if (pending.current) event.preventDefault();
                }}
            >
                <h2 id="collect-exit-title">保存对“{fileName}”的更改？</h2>
                <p className="collect-editor__dialog-description">选择“不保存”将丢弃本次正文更改。</p>
                {closeFailure && errorMessage}
                <div className="collect-editor__actions">
                    <button type="button" disabled={busy} onClick={() => void handleDiscard()}>
                        不保存
                    </button>
                    <button type="button" autoFocus disabled={busy} onClick={handleCancelClose}>
                        取消
                    </button>
                    <button type="button" disabled={busy} onClick={() => void handleSave(true)}>
                        {operation === "save" ? "保存中…" : "保存"}
                    </button>
                </div>
            </dialog>
        </main>
    );
};
