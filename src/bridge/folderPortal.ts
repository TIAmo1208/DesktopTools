import { parseApplicationCommandError } from "../contracts/application";
import { parseFolderPortalSnapshot, type FolderPortalSnapshot } from "../contracts/folderPortal";
import { invokeRuntime } from "./runtime";
import { parseShellOperationResult, type ShellOperationResult } from "../contracts/desktopItem";

/** 获取指定文件夹门户的目录快照。 */
export const getFolderPortalSnapshot = async (rootPath: string, currentPath?: string): Promise<FolderPortalSnapshot> => {
    if (!rootPath.trim()) throw new Error("门户绑定目录不能为空");
    const response = await invokeRuntime<unknown>("get_folder_portal_snapshot", { rootPath, currentPath }).catch((error: unknown) => {
        throw parseApplicationCommandError(error);
    });
    return parseFolderPortalSnapshot(response);
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
