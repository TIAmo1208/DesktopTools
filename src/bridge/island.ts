import type { UnlistenFn } from "@tauri-apps/api/event";
import { parseIslandSnapshot, parseIslandStateRequest, parseIslandDragResult, type IslandSnapshot, type IslandStateRequest } from "../contracts/island";
import { parsePreferences } from "../contracts/settings";
import { COMPONENT_RUNTIME_SNAPSHOT_CHANGED_EVENT } from "./components";
import { getPreferences, savePreferences } from "./settings";
import { invokeRuntime, listenRuntime } from "./runtime";

/** 岛实例不从工作区取得，统一读取独立投影。 */
export const getIslandSnapshot = async (): Promise<IslandSnapshot> => parseIslandSnapshot(await invokeRuntime<unknown>("get_island_snapshot"));

/** 提交一次实测三态联合盒，原生失败不静默当作成功。 */
export const setIslandState = (request: IslandStateRequest) => invokeRuntime<void>("set_island_state", { request: parseIslandStateRequest(request) });

/** 常态或 hover 进入原生移动循环，返回吸附完成时的权威指针位置。 */
export const beginIslandDrag = async () => parseIslandDragResult(await invokeRuntime<unknown>("begin_island_drag"));

/** 两个排序入口共用偏好路径；提交前读取权威偏好，避免使用页面旧副本。 */
export const saveIslandCardOrder = async (islandCardOrder: string[]) => {
    const preferences = await getPreferences(); // 保留岛位置及其他已保存字段。
    return savePreferences(parsePreferences({ ...preferences, islandCardOrder }));
};

/** 复用清单与偏好事件作为失效通知，不发布岛专用事件，也不轮询。 */
export const listenIslandSnapshots = async (onSnapshot: (snapshot: IslandSnapshot) => void, onError: (error: unknown) => void): Promise<UnlistenFn> => {
    let active = true; // 退订后拒绝迟到读取。
    let generation = 0; // 新通知优先于旧命令结果。
    const refresh = () => {
        const ticket = ++generation;
        void getIslandSnapshot()
            .then((snapshot) => {
                if (active && ticket === generation) onSnapshot(snapshot);
            })
            .catch((error: unknown) => {
                if (active && ticket === generation) onError(error);
            });
    };
    const stops: UnlistenFn[] = []; // 部分订阅失败也清理已有资源。
    try {
        for (const event of [COMPONENT_RUNTIME_SNAPSHOT_CHANGED_EVENT, "preferences-changed"]) stops.push(await listenRuntime(event, refresh));
        refresh(); // 先订阅再读，覆盖初始化竞态。
    } catch (error) {
        active = false;
        for (const stop of stops) stop();
        throw error;
    }
    return () => {
        active = false;
        generation++;
        for (const stop of stops) stop();
    };
};
