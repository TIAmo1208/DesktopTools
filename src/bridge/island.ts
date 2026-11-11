import type { UnlistenFn } from "@tauri-apps/api/event";
import { parseIslandMessages, parseIslandSnapshot, parseIslandStateRequest, parseIslandDragResult, type IslandMessage, type IslandSnapshot, type IslandStateRequest } from "../contracts/island";
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

/** 浮岛消息与原生 Toast 共用队列；事件载荷是完整投影，不另开增量协议。 */
export const ISLAND_MESSAGES_CHANGED_EVENT = "island-messages-changed";

/** 首次挂载与广播丢失后的补齐读取。 */
export const getIslandMessages = async (): Promise<IslandMessage[]> => parseIslandMessages(await invokeRuntime<unknown>("get_island_messages"));

/** 关闭一条消息：后端走与原生关闭按钮相同的离场路径，不另建第二份队列。 */
export const dismissIslandMessage = async (id: number) => {
    if (!Number.isSafeInteger(id) || id <= 0) throw new Error("岛消息身份无效");
    await invokeRuntime<void>("dismiss_island_message", { id });
};

/** 先订阅再读一次：事件到达后迟到的首次读取不得覆盖新集合。 */
export const listenIslandMessages = async (onMessages: (messages: IslandMessage[]) => void, onError: (error: unknown) => void): Promise<UnlistenFn> => {
    let active = true; // 退订后拒绝迟到交付。
    let generation = 0; // 每次事件与读取都推进，旧读取直接丢弃。
    const stop = await listenRuntime<unknown>(ISLAND_MESSAGES_CHANGED_EVENT, (payload) => {
        if (!active) return;
        generation++;
        try {
            onMessages(parseIslandMessages(payload));
        } catch (error) {
            onError(error); // 坏载荷只报告，不覆盖页面已有消息。
        }
    });
    const ticket = ++generation;
    void getIslandMessages()
        .then((messages) => {
            if (active && ticket === generation) onMessages(messages);
        })
        .catch((error: unknown) => {
            if (active && ticket === generation) onError(error);
        });
    return () => {
        active = false;
        generation++;
        stop();
    };
};
