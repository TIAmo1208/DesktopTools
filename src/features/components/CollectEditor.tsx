import { useEffect, useRef, useState } from "react";
import Vditor from "vditor";
import "vditor/dist/index.css";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { getCollectItems, importCollectRenderedResource, renameCollectItem, saveCollectItem, writeCollectResource } from "../../bridge/collect";
import { usePreferences } from "../settings/PreferencesProvider";
import { settingsErrorMessage } from "../../bridge/settings";
import { publishMessage } from "../notifications/messageNotification";
import { pastedLink } from "./collectEditorLinks";
import "./CollectEditor.css";

/** 按需窗口中的 Vditor Markdown 编辑器。 */
export const CollectEditor = () => {
    const name = new URLSearchParams(window.location.search).get("name") ?? ""; // 当前文件身份。
    const currentName = useRef(name); // 编辑过程中可变的当前文件身份。
    const host = useRef<HTMLDivElement>(null); // 编辑器挂载节点。
    const editor = useRef<Vditor | undefined>(undefined); // 已初始化编辑器。
    const dialog = useRef<HTMLDialogElement>(null); // 原生模态对话框管理焦点和背景交互。
    const dirty = useRef(false); // 输入时同步更新，关闭请求不读取过期 React 状态。
    const pending = useRef(false); // 保存与关闭共用互斥状态。
    const allowClose = useRef(false); // 保存成功或明确放弃后允许原生关闭继续。
    const [saved, setSaved] = useState(true); // 保存按钮状态。
    const [ready, setReady] = useState(false); // 完成编辑器异步初始化后才允许保存。
    const [busy, setBusy] = useState(false); // 异步操作期间禁用对话框按钮。
    const [failure, setFailure] = useState<string>(); // 保留可见的失败原因。
    const [itemName, setItemName] = useState(name.replace(/\.md$/i, "")); // 标题栏中的可编辑文件名。
    const [imageMenu, setImageMenu] = useState<{ x: number; y: number; reference: string }>(); // 当前图片菜单位置和引用。
    const { preferences } = usePreferences(); // 当前退出策略。
    const autoSave = useRef(false); // 关闭订阅保持稳定并读取最新设置。
    autoSave.current = preferences?.collectAutoSaveOnExit === true;

    /** 失败不丢弃内容，并沿用应用消息通知。 */
    const handleFailure = (error: unknown) => {
        const message = settingsErrorMessage(error); // 兼容后端字符串和结构化错误。
        setFailure(message);
        publishMessage({ type: "error", content: message, source: "collect-editor" });
    };

    /** 串行保存；保存成功才销毁窗口，失败恢复编辑能力。 */
    const handleSave = async (close = false) => {
        const current = editor.current; // 保存期间持有同一编辑器。
        if (!current || pending.current) return;
        pending.current = true;
        setBusy(true);
        current.disabled();
        try {
            await saveCollectItem(currentName.current, current.getValue());
            dirty.current = false;
            setSaved(true);
            setFailure(undefined);
            if (close) {
                allowClose.current = true;
                await getCurrentWindow().destroy();
            }
        } catch (error) {
            allowClose.current = false;
            handleFailure(error);
        } finally {
            pending.current = false;
            setBusy(false);
            if (editor.current === current) current.enable();
        }
    };
    /** 重命名 Markdown 文件及其旁侧资源目录。 */
    const handleRename = async () => {
        const next = itemName.trim();
        const current = editor.current; // 重命名期间固定编辑器实例并锁定输入。
        if (!ready || !current || !next || next === currentName.current.replace(/\.md$/i, "") || pending.current) return;
        pending.current = true;
        setBusy(true);
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
            setFailure(undefined);
        } catch (error) {
            handleFailure(error);
        } finally {
            pending.current = false;
            setBusy(false);
            if (editor.current === current) current.enable();
        }
    };

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
                if (!item) throw new Error("收集项不可用，文件可能已被删除或重命名");
                editor.current = new Vditor(host.current, {
                    value: item.content,
                    mode: "wysiwyg",
                    lang: "zh_CN",
                    cdn: "/vditor",
                    _lutePath: "/vditor/dist/js/lute/lute.min.js",
                    cache: { enable: false },
                    toolbar: ["headings", "bold", "italic", "strike", "link", "list", "check", "quote", "line", "table", "undo", "redo"],
                    input: () => {
                        dirty.current = true;
                        setSaved(false);
                    },
                    after: () => {
                        if (active) setReady(true);
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
                if (active) handleFailure(error);
            });
        return () => {
            active = false;
            editor.current?.destroy();
            editor.current = undefined;
        };
    }, [name]);
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
    return (
        <main className="collect-editor" aria-busy={busy}>
            <header className="collect-editor__header">
                <div className="collect-editor__name">
                    <input aria-label="收集项名称" value={itemName} onChange={(event) => setItemName(event.currentTarget.value)} disabled={busy} />
                    <button
                        type="button"
                        onClick={() => void handleRename()}
                        disabled={!ready || busy || !itemName.trim() || itemName.trim() === currentName.current.replace(/\.md$/i, "")}
                    >
                        重命名
                    </button>
                </div>
                <button type="button" onClick={() => void handleSave()} disabled={saved || busy || !ready}>
                    保存
                </button>
            </header>
            {failure && (
                <p role="alert" className="collect-editor__error">
                    {failure}
                </p>
            )}
            <div
                ref={host}
                className="collect-editor__content"
                aria-label="Markdown 内容"
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
                <p id="collect-exit-title">收集项尚未保存，退出前如何处理？</p>
                {failure && (
                    <p role="alert" className="collect-editor__error">
                        {failure}
                    </p>
                )}
                <div className="collect-editor__actions">
                    <button type="button" disabled={busy} onClick={() => void handleSave(true)}>
                        保存
                    </button>
                    <button type="button" disabled={busy} onClick={() => void handleDiscard()}>
                        不保存
                    </button>
                    <button type="button" autoFocus disabled={busy} onClick={() => dialog.current?.close()}>
                        取消
                    </button>
                </div>
            </dialog>
        </main>
    );
};
