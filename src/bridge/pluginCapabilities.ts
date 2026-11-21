import type { ComponentCapability, PluginCredentialExchange, PluginCredentialExchangeResult } from "../contracts/component.ts";
import type { LoopbackChannelHandle, LoopbackChannelRegistration, LoopbackRequestEntry, LoopbackResponse } from "../contracts/loopback.ts";
import type { IslandPluginMessageKind } from "../contracts/island.ts";

/** 插件投递的一条岛内消息；key 在插件内去重。 */
export interface PluginIslandMessage
{
    key: string; // 插件内稳定去重键。
    kind: IslandPluginMessageKind; // 条目语气。
    text: string; // 单行正文。
    uri?: string; // 可选导航去向；宿主按来源实例打开。
}

/** 通用能力命令的依赖；测试可替换为假桥。 */
export interface PluginCapabilityCommands
{
    setCredential: (instanceId: string, slot: string, value: string) => Promise<void>;
    clearCredential: (instanceId: string, slot: string) => Promise<void>;
    exchangeCredential: (instanceId: string, slot: string, request: PluginCredentialExchange) => Promise<PluginCredentialExchangeResult>;
    publishIslandMessage: (instanceId: string, key: string, kind: IslandPluginMessageKind, text: string, uri?: string) => Promise<void>;
    openUri: (instanceId: string, uri: string) => Promise<void>;
    registerLoopback: (registration: LoopbackChannelRegistration) => Promise<LoopbackChannelHandle>;
    readLoopback: (instanceId: string, channelId: string, afterSequence?: number) => Promise<LoopbackRequestEntry[]>;
    requestLoopback: (instanceId: string, channelId: string, method: string, path: string, body?: string) => Promise<LoopbackResponse>;
}

/** 只在实例声明并获授权时提供给插件的上下文；未授权项为 undefined。 */
export interface PluginCapabilityContext
{
    setCredential?: (slot: string, value: string) => Promise<void>;
    clearCredential?: (slot: string) => Promise<void>;
    exchangeCredential?: (slot: string, request: PluginCredentialExchange) => Promise<PluginCredentialExchangeResult>;
    publishIslandMessage?: (message: PluginIslandMessage) => Promise<void>;
    openUri?: (uri: string) => Promise<void>;
    registerLoopbackChannel?: (registration: LoopbackChannelRegistration) => Promise<LoopbackChannelHandle>;
    readLoopbackChannel?: (channelId: string, afterSequence?: number) => Promise<LoopbackRequestEntry[]>;
    requestLoopback?: (channelId: string, method: string, path: string, body?: string) => Promise<LoopbackResponse>;
}

/**
 * 按实例已授权能力裁剪上下文：未声明的能力对应函数为 undefined；
 * 命令统一携带实例身份，设置条目没有实例时整组能力都不提供。
 */
export const createPluginCapabilityContext = (
    instanceId: string,
    capabilities: readonly ComponentCapability[],
    commands: PluginCapabilityCommands,
): PluginCapabilityContext => {
    if (!instanceId) return {};
    const granted = new Set(capabilities); // 同一清单可能声明多项能力。
    const context: PluginCapabilityContext = {};
    if (granted.has("plugin-credential")) {
        context.setCredential = (slot, value) => commands.setCredential(instanceId, slot, value);
        context.clearCredential = (slot) => commands.clearCredential(instanceId, slot);
    }
    if (granted.has("plugin-credential-exchange")) {
        context.exchangeCredential = (slot, request) => commands.exchangeCredential(instanceId, slot, request);
    }
    if (granted.has("publish-island-message")) {
        context.publishIslandMessage = (message) => commands.publishIslandMessage(instanceId, message.key, message.kind, message.text, message.uri);
    }
    if (granted.has("open-uri")) context.openUri = (uri) => commands.openUri(instanceId, uri);
    if (granted.has("loopback-channel")) {
        // 实例身份由宿主覆盖，插件不能登记到别的实例。
        context.registerLoopbackChannel = (registration) => commands.registerLoopback({ ...registration, instanceId });
        context.readLoopbackChannel = (channelId, afterSequence) => commands.readLoopback(instanceId, channelId, afterSequence);
        context.requestLoopback = (channelId, method, path, body) => commands.requestLoopback(instanceId, channelId, method, path, body);
    }
    return context;
};
