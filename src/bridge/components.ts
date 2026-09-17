import type { UnlistenFn } from "@tauri-apps/api/event";

import { parseApplicationCommandError } from "../contracts/application";
import {
    parseComponentCapabilityResult,
    parseComponentHostEvent,
    parseComponentRuntimeSnapshot,
    type ComponentCapability,
    type ComponentCapabilityResult,
    type ComponentHostEvent,
    type ComponentRuntimeSnapshot,
} from "../contracts/component";
import { invokeRuntime, listenRuntime } from "./runtime";

// Rust 发布的组件运行时快照事件名称。
export const COMPONENT_RUNTIME_SNAPSHOT_CHANGED_EVENT = "component-runtime-snapshot-changed";

// Rust 发布的组件宿主事件名称。
export const COMPONENT_HOST_EVENT = "component-host-event";

/** 同步当前工作区组件实例并获取运行时注册快照。 */
export const getComponentRuntimeSnapshot = async (): Promise<ComponentRuntimeSnapshot> => {
    const response = await invokeRuntime<unknown>("get_component_runtime_snapshot").catch((error: unknown) => {
        throw parseApplicationCommandError(error);
    });
    return parseComponentRuntimeSnapshot(response);
};

/** 校验实例清单后执行一个已声明并授权的宿主能力。 */
export const invokeComponentCapability = async (instanceId: string, capability: ComponentCapability): Promise<ComponentCapabilityResult> => {
    const response = await invokeRuntime<unknown>("invoke_component_capability", { instanceId, capability }).catch((error: unknown) => {
        throw parseApplicationCommandError(error);
    });
    return parseComponentCapabilityResult(response);
};

/** 订阅 Component Runtime 实例注册快照。 */
export const listenComponentRuntimeSnapshots = (
    onSnapshot: (snapshot: ComponentRuntimeSnapshot) => void,
    onError: (error: unknown) => void,
): Promise<UnlistenFn> =>
    listenRuntime<unknown>(COMPONENT_RUNTIME_SNAPSHOT_CHANGED_EVENT, (payload) => {
        try {
            onSnapshot(parseComponentRuntimeSnapshot(payload));
        } catch (error: unknown) {
            onError(error);
        }
    });

/** 订阅已授权组件可以消费的宿主事件。 */
export const listenComponentHostEvents = async (
    instanceId: string,
    monitorId: string,
    onEvent: (event: ComponentHostEvent) => void,
    onError: (error: unknown) => void,
): Promise<UnlistenFn> => {
    const stop = await listenRuntime<unknown>(COMPONENT_HOST_EVENT, (payload) => {
        try {
            onEvent(parseComponentHostEvent(payload));
        } catch (error: unknown) {
            onError(error);
        }
    });
    try {
        const subscription = await invokeRuntime<number>("subscribe_component_time", { instanceId, monitorId }); // 接收入口已建立，再唤醒共享计时源。
        return () => {
            stop();
            void invokeRuntime("unsubscribe_component_time", { subscription }).catch(onError);
        };
    } catch (error) {
        stop();
        throw error;
    }
};
