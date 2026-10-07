import { invokeRuntime, listenRuntime } from "./runtime";
import type { UnlistenFn } from "@tauri-apps/api/event";

export interface CollectItem {
    name: string;
    content: string;
    createdUnixMs: number;
    pinned: boolean;
    available: boolean;
}
export const getCollectItems = () => invokeRuntime<CollectItem[]>("get_collect_items");
/** 读取本地图片供编辑器显示，原始 Markdown 引用保持不变。 */
export const readCollectImage = (name: string, reference: string) => invokeRuntime<string>("read_collect_image", { name, reference });
/** 交给系统默认程序打开，禁止在桌面宿主中导航。 */
export const openCollectLink = (url: string) => invokeRuntime<void>("open_collect_link", { url });
export const createCollectItem = (content = "") => invokeRuntime<CollectItem>("create_collect_item", { content });
export const saveCollectItem = (name: string, content: string) => invokeRuntime<void>("save_collect_item", { name, content });
export const importCollectResource = (name: string, source: string, mode: "copy" | "move" | "link") =>
    invokeRuntime<string>("import_collect_resource", { name, source, mode });
export const importCollectRenderedResource = (name: string, reference: string, mode: "copy" | "move") =>
    invokeRuntime<string>("import_collect_rendered_resource", { name, reference, mode });
export const writeCollectResource = (name: string, fileName: string, bytes: number[]) =>
    invokeRuntime<string>("write_collect_resource", { name, fileName, bytes });
export const openCollectEditor = (name: string) => invokeRuntime<void>("open_collect_editor", { name });
export const renameCollectItem = (oldName: string, newName: string, draft: string) => invokeRuntime<string>("rename_collect_item", { oldName, newName, draft });
export const setCollectItemPinned = (name: string, pinned: boolean) => invokeRuntime<void>("set_collect_item_pinned", { name, pinned });
export const deleteCollectItem = (name: string) => invokeRuntime<void>("delete_collect_item", { name });
export const restoreCollectItem = () => invokeRuntime<CollectItem | null>("restore_collect_item");
export const listenCollectChanges = (onChange: (deletedName?: string) => void, onError: (error: unknown) => void): Promise<UnlistenFn> =>
    listenRuntime<{ deletedName?: string } | null>("collect-items-changed", (payload) => onChange(payload?.deletedName))
        .then((stop) =>
            listenRuntime<unknown>("folder-portal-changed", (payload) => {
                if (typeof payload === "object" && payload !== null && "monitor" in payload && payload.monitor === "collect") onChange();
            })
                .then((stopPortal) => () => {
                    stop();
                    stopPortal();
                })
                .catch((error) => {
                    stop();
                    onError(error);
                    return () => undefined;
                }),
        )
        .catch((error) => {
            onError(error);
            return () => undefined;
        });
