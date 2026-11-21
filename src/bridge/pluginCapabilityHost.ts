import { clearPluginCredential, exchangePluginCredential, openPluginUri, publishIslandMessage, setPluginCredential } from "./components.ts";
import { readLoopbackChannel, registerLoopbackChannel, requestLoopback } from "./loopback.ts";
import { createPluginCapabilityContext, type PluginCapabilityCommands, type PluginCapabilityContext } from "./pluginCapabilities.ts";
import type { ComponentCapability } from "../contracts/component.ts";

/** 生产命令：实例身份由宿主传入，插件不能自己指定组件标识。 */
const commands: PluginCapabilityCommands = {
    setCredential: setPluginCredential,
    clearCredential: clearPluginCredential,
    exchangeCredential: exchangePluginCredential,
    publishIslandMessage,
    openUri: openPluginUri,
    registerLoopback: (registration) => registerLoopbackChannel(registration),
    readLoopback: readLoopbackChannel,
    requestLoopback,
};

/** 用生产命令构造按实例授权裁剪的插件能力上下文。 */
export const pluginCapabilityContext = (instanceId: string, capabilities: readonly ComponentCapability[]): PluginCapabilityContext =>
    createPluginCapabilityContext(instanceId, capabilities, commands);
