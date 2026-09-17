import { parseApplicationCommandError } from "../contracts/application";
import { parseFolderPortalSnapshot, type FolderPortalSnapshot } from "../contracts/folderPortal";
import { invokeRuntime, listenRuntime } from "./runtime";
import { parseShellDragPreparation, parseShellOperationResult, type ShellDragPreparation, type ShellOperationResult } from "../contracts/desktopItem";

/** 获取指定文件夹门户的目录快照。 */
export const getFolderPortalSnapshot = async (rootPath: string, currentPath?: string): Promise<FolderPortalSnapshot> => {
    if (!rootPath.trim()) throw new Error("门户绑定目录不能为空");
    const response = await invokeRuntime<unknown>("get_folder_portal_snapshot", { rootPath, currentPath }).catch((error: unknown) => {
        throw parseApplicationCommandError(error);
    });
    return parseFolderPortalSnapshot(response);
};

/** 管理一个目录的订阅、串行读取和故障恢复；释放后拒绝全部迟到结果。 */
export const watchFolderPortal = (
    rootPath: string,
    currentPath: string,
    monitorId: string,
    onSnapshot: (snapshot: FolderPortalSnapshot) => void,
    onError: (error: unknown) => void,
    previousRead: Promise<void> = Promise.resolve(), // 导航沿用上一个目录的在途读取，避免同一门户并发枚举。
) => {
    let active = true; // 当前目录生命周期。
    let paused = window.__DESKTOP_TOOLS_NATIVE__?.isRefreshPaused?.() ?? false; // 暂停期间只保存重读意图。
    let generation = 0; // 暂停及销毁使在途读取失效。
    let dirty = false; // 突发通知合并为一次后续读取。
    let reading: Promise<void> | undefined; // 同一目录只有一个在途枚举。
    let connecting: Promise<void> | undefined; // 手动恢复和重试共用一次握手。
    let subscription: number | undefined; // 当前有效后端监听。
    let unlisten: (() => void) | undefined; // 前端接收入口。
    let retry: ReturnType<typeof setTimeout> | undefined; // 仅失败状态下使用的重连计时器。
    let failures = 0; // 当前连续失败的退避级别。
    const earlyErrors = new Map<number, string>(); // 握手返回身份前收到的终止通知。
    const release = (id: number) => {
        void invokeRuntime("unwatch_folder_portal", { subscription: id }).catch((error) => active && onError(error));
    }; // 身份独立，旧退订不能取消新监听。
    const read = (): Promise<void> => {
        dirty = true;
        if (!active || paused) return Promise.resolve();
        if (reading) return reading;
        reading = (async () => {
            await previousRead;
            while (active && !paused && dirty) {
                dirty = false;
                const request = generation; // 当前读取允许写回的代次。
                try {
                    const snapshot = await getFolderPortalSnapshot(rootPath, currentPath); // 路径固定于本次订阅。
                    if (active && !paused && request === generation && !dirty) onSnapshot(snapshot);
                } catch (error) {
                    if (active && !paused && request === generation) onError(error);
                }
            }
        })().finally(() => {
            reading = undefined;
        });
        return reading;
    }; // 在途变化只追加一次重读，不并发枚举。
    const scheduleRetry = (error: unknown) => {
        if (!active) return;
        onError(error);
        clearTimeout(retry);
        if (!paused)
            retry = setTimeout(() => {
                retry = undefined;
                void connect();
            }, [1000, 2000, 5000, 10000, 30000][Math.min(failures++, 4)]);
    }; // 无有效事件源时退避，正常订阅不设置计时器。
    const connect = (): Promise<void> => {
        if (!active || paused) return Promise.resolve();
        if (connecting) return connecting;
        connecting = (async () => {
            try {
                if (!unlisten) {
                    const stop = await listenRuntime<{ subscription: number; error?: string }>("folder-portal-changed", (event) => {
                        if (!active) return;
                        if (subscription === undefined) {
                            if (connecting && event.error) earlyErrors.set(event.subscription, event.error);
                            return; // 握手后的初始读取覆盖期间的普通变化。
                        }
                        if (event.subscription !== subscription) return;
                        if (event.error) {
                            release(subscription);
                            subscription = undefined;
                            scheduleRetry(new Error(event.error));
                        }
                        void read();
                    }); // 前端先就绪，后启动后端监听。
                    if (!active) {
                        stop();
                        return;
                    }
                    unlisten = stop;
                }
                if (subscription === undefined) {
                    earlyErrors.clear();
                    const id = await invokeRuntime<number>("watch_folder_portal", { rootPath, currentPath, monitorId }); // 返回时首次 I/O 已就绪。
                    if (!active) {
                        release(id);
                        return;
                    }
                    const error = earlyErrors.get(id); // 监听可能已在命令响应前失效。
                    earlyErrors.clear();
                    if (error) {
                        release(id);
                        throw new Error(error);
                    }
                    subscription = id;
                }
                failures = 0;
                clearTimeout(retry);
            } catch (error) {
                scheduleRetry(error);
            }
            void read(); // 枚举不阻塞监听重连；不可访问快照沿用现有恢复入口。
        })().finally(() => {
            connecting = undefined;
        });
        return connecting;
    }; // 初始化和重建使用相同路径。
    const refresh = async () => {
        clearTimeout(retry);
        if (subscription === undefined) await connect();
        else await read();
    }; // 手动刷新同时修复已失效监听。
    const handleRefreshState = () => {
        paused = window.__DESKTOP_TOOLS_NATIVE__?.isRefreshPaused?.() ?? false;
        generation++;
        clearTimeout(retry);
        if (!paused) void refresh();
    }; // 恢复只读最终事实。
    window.addEventListener("desktop-refresh-changed", handleRefreshState);
    void connect();
    return {
        refresh,
        drained: () => Promise.all([previousRead, reading]).then(() => undefined), // 导航后的读取等待旧枚举结束。
        stop: () => {
            active = false;
            generation++;
            clearTimeout(retry);
            window.removeEventListener("desktop-refresh-changed", handleRefreshState);
            unlisten?.();
            if (subscription !== undefined) release(subscription);
        },
    };
};

/** 交由宿主打开当前选择。 */
export const openFolderPortalPath = async (rootPath: string, path: string): Promise<void> => {
    await invokeRuntime<unknown>("open_folder_portal_path", { rootPath, path });
};

/** 读取 Shell 每项回收结果，不将取消或部分失败当作全部成功。 */
export const deleteFolderPortalItems = async (rootPath: string, currentPath: string, names: string[]): Promise<ShellOperationResult> => {
    return parseShellOperationResult(await invokeRuntime<unknown>("delete_folder_portal_items", { rootPath, currentPath, names }));
};

/** 将当前目录选择写入系统 Shell 剪贴板。 */
export const setFolderPortalClipboard = async (rootPath: string, currentPath: string, names: string[], cut: boolean): Promise<ShellOperationResult> =>
    parseShellOperationResult(await invokeRuntime("set_folder_portal_clipboard", { rootPath, currentPath, names, cut }));

/** 在所属显示器显示当前目录项目的原生菜单。 */
export const showFolderPortalMenu = async (
    rootPath: string,
    currentPath: string,
    names: string[],
    monitorId: string,
    clientX: number,
    clientY: number,
): Promise<void> => {
    await invokeRuntime("show_folder_portal_menu", { request: { rootPath, currentPath, names, monitorId, clientX, clientY } });
};

/** 提交门户单项原位重命名并保留 Shell 实际结果。 */
export const renameFolderPortalItem = async (rootPath: string, currentPath: string, name: string, newName: string): Promise<ShellOperationResult> => {
    return parseShellOperationResult(await invokeRuntime("rename_folder_portal_item", { request: { rootPath, currentPath, name, newName } }));
};

/** 将系统剪贴板粘贴到门户当前目录。 */
export const pasteFolderPortalItems = async (rootPath: string, currentPath: string): Promise<ShellOperationResult> =>
    parseShellOperationResult(await invokeRuntime("paste_folder_portal_items", { rootPath, currentPath }));

/** 准备当前目录中完整选择集合的原生 Shell 拖放凭据。 */
export const prepareFolderPortalDrag = async (rootPath: string, currentPath: string, names: string[]): Promise<ShellDragPreparation> => {
    const result = parseShellDragPreparation(await invokeRuntime("prepare_folder_portal_drag", { rootPath, currentPath, names }));
    if (!result) throw new Error("无法准备门户文件拖放");
    return result;
};
