import type { UnlistenFn } from "@tauri-apps/api/event";

import { LOOPBACK_EVENT_PREFIX, parseComponentHostEvent, type ComponentHostEvent } from "../contracts/component.ts";
import {
    parseLoopbackChannelHandle,
    parseLoopbackRequestEntries,
    parseLoopbackResponse,
    type LoopbackChannelHandle,
    type LoopbackChannelRegistration,
    type LoopbackRequestEntry,
    type LoopbackResponse,
} from "../contracts/loopback.ts";
import { invokeRuntime, listenRuntime } from "./runtime.ts";

// 与 components.ts 的 COMPONENT_HOST_EVENT 同名；单独写一份以保持本模块可被纯逻辑测试直接导入。
const COMPONENT_HOST_EVENT = "component-host-event";

/** 注册一条插件 loopback 通道，返回宿主实际绑定的句柄。 */
export const registerLoopbackChannel = async (registration: LoopbackChannelRegistration): Promise<LoopbackChannelHandle> => {
    const response = await invokeRuntime<unknown>("register_loopback_channel", { ...registration });
    return parseLoopbackChannelHandle(response);
};

/** 读取并消费通道缓冲里尚未读走的原始请求。 */
export const readLoopbackChannel = async (instanceId: string, channelId: string, afterSequence?: number): Promise<LoopbackRequestEntry[]> => {
    const response = await invokeRuntime<unknown>("read_loopback_channel", { instanceId, channelId, ...(afterSequence === undefined ? {} : { afterSequence }) });
    return parseLoopbackRequestEntries(response);
};

/** 经 request 通道发一次原始请求；非 2xx 仍按成功返回，由插件判断。 */
export const createRequestLoopback = (invoke: (command: string, payload: Record<string, unknown>) => Promise<unknown>) => async (
    instanceId: string,
    channelId: string,
    method: string,
    path: string,
    body?: string,
): Promise<LoopbackResponse> => {
    const response = await invoke("request_loopback", { instanceId, channelId, method, path, ...(body === undefined ? {} : { body }) });
    return parseLoopbackResponse(response);
};

/** 生产环境的 request 命令。 */
export const requestLoopback = createRequestLoopback((command, payload) => invokeRuntime<unknown>(command, payload));

/** 一条订阅的处理函数；载荷是本次读到的原始请求。 */
export interface LoopbackSubscriptionHandler {
    onEntries: (entries: LoopbackRequestEntry[]) => void;
    onError: (error: unknown) => void;
}

/** 订阅表工厂的依赖，测试可替换为假桥。 */
export interface LoopbackHubDependencies {
    listen: (eventName: string, handler: (event: ComponentHostEvent) => void) => Promise<UnlistenFn>;
    read: (instanceId: string, channelId: string) => Promise<LoopbackRequestEntry[]>;
}

interface LoopbackSubscription {
    instanceId: string; // 读取缓冲时使用的实例。
    channelId: string; // 事件种类里的通道标识。
    handlers: Set<LoopbackSubscriptionHandler>; // 同一实例通道的多个读者。
    paused: boolean; // 不可见时暂停交付，缓冲留在宿主。
}

/** 订阅键：同一实例的同一条通道只保留一份底层监听。 */
const subscriptionKey = (instanceId: string, channelId: string) => `${instanceId}\u0000${channelId}`;

/**
 * 创建按实例通道去重的通知订阅。
 * 所有订阅共用一条 component-host-event 监听；最后一位读者退订后才释放。
 */
export const createLoopbackHub = (dependencies: LoopbackHubDependencies) => {
    const subscriptions = new Map<string, LoopbackSubscription>(); // 按实例与通道归并的读者。
    let stopListen: UnlistenFn | undefined; // 唯一的底层事件监听。
    let listening: Promise<UnlistenFn> | undefined; // 建立中的监听，避免并发重复订阅。
    let disposed = false; // 释放后拒绝迟到事件。

    /** 按通道标识找到所有等待这条通知的订阅。 */
    const matching = (kind: string) => {
        const channelId = kind.slice(LOOPBACK_EVENT_PREFIX.length);
        return [...subscriptions.values()].filter((subscription) => subscription.channelId === channelId);
    };

    /** 读一次缓冲并把结果交给当前可见的读者。 */
    const deliver = async (subscription: LoopbackSubscription) => {
        try {
            const entries = await dependencies.read(subscription.instanceId, subscription.channelId);
            if (disposed || subscription.paused || entries.length === 0) return;
            for (const handler of [...subscription.handlers]) handler.onEntries(entries);
        } catch (error: unknown) {
            if (disposed) return;
            for (const handler of [...subscription.handlers]) handler.onError(error);
        }
    };

    /** 处理宿主事件：只认 loopback 数据通知，事件本身不带正文。 */
    const handleEvent = (event: ComponentHostEvent) => {
        if (disposed || !event.kind.startsWith(LOOPBACK_EVENT_PREFIX)) return;
        for (const subscription of matching(event.kind)) {
            if (subscription.paused) continue; // 不可见时不消费缓冲，恢复时一次补读。
            void deliver(subscription);
        }
    };

    /** 首个子订阅建立底层监听；其余实例通道共用。 */
    const ensureListening = () => {
        listening ??= dependencies
            .listen(COMPONENT_HOST_EVENT, handleEvent)
            .then((stop) => {
                if (disposed || subscriptions.size === 0) stop();
                else stopListen = stop;
                return stop;
            })
            .finally(() => {
                listening = undefined;
            });
        return listening;
    };

    /** 订阅一条实例通道；重复调用返回各自的退订函数。 */
    const subscribe = (instanceId: string, channelId: string, handler: LoopbackSubscriptionHandler) => {
        const key = subscriptionKey(instanceId, channelId); // 同实例同通道合并。
        let subscription = subscriptions.get(key);
        if (!subscription) {
            subscription = { instanceId, channelId, handlers: new Set(), paused: false };
            subscriptions.set(key, subscription);
            void ensureListening();
        }
        subscription.handlers.add(handler);
        return () => {
            subscription.handlers.delete(handler);
            if (subscription.handlers.size > 0) return;
            subscriptions.delete(key);
            if (subscriptions.size === 0) {
                stopListen?.();
                stopListen = undefined;
            }
        };
    };

    /** 不可见时暂停交付；恢复时补读一次最新缓冲，不重放积压事件。 */
    const setPaused = (instanceId: string, channelId: string, paused: boolean) => {
        const subscription = subscriptions.get(subscriptionKey(instanceId, channelId));
        if (!subscription || subscription.paused === paused) return;
        subscription.paused = paused;
        if (!paused) void deliver(subscription);
    };

    /** 释放监听与全部读者；用于卸载或测试收尾。 */
    const dispose = () => {
        disposed = true;
        subscriptions.clear();
        stopListen?.();
        stopListen = undefined;
    };

    return { subscribe, setPaused, dispose };
};

/** 正式页面桥上的 loopback 订阅。 */
const pageHub = createLoopbackHub({
    listen: (eventName, handler) =>
        listenRuntime<unknown>(eventName, (payload) => {
            try {
                handler(parseComponentHostEvent(payload));
            } catch (error: unknown) {
                console.warn("loopback 通知无效", error);
            }
        }),
    read: (instanceId, channelId) => readLoopbackChannel(instanceId, channelId),
});

/** 订阅实例通道的数据通知，返回退订函数。 */
export const subscribeLoopbackChannel = (instanceId: string, channelId: string, handler: LoopbackSubscriptionHandler) =>
    pageHub.subscribe(instanceId, channelId, handler);

/** 暂停或恢复视觉刷新；恢复时补读一次缓冲。 */
export const setLoopbackVisualPaused = (instanceId: string, channelId: string, paused: boolean) => pageHub.setPaused(instanceId, channelId, paused);

/** 释放页面级订阅，仅测试与页面卸载使用。 */
export const disposeLoopbackSubscriptions = () => pageHub.dispose();
