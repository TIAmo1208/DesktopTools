/** 声明式导航：插件只声明去向，宿主负责校验与打开，页面只给就地反馈。 */

import { normalizeIslandNavigationUri } from "../../contracts/island.ts";

/** 归一化导航地址：与宿主同一白名单（dsh:/https:、≤512、无控制字符），非法即未声明。 */
export const islandNavigationUri = (value: unknown): string | undefined => normalizeIslandNavigationUri(value);

/** 锚点与消息条目共用的实例身份：插件在浮岛上的合成实例。 */
export const islandNavigationInstanceId = (componentId: string): string => `island.${componentId}`;

/** 可点击项的可访问名称：带上来源与去向，读屏能听到会打开什么。 */
export const islandNavigationLabel = (label: string, uri: string): string => `${label}：打开 ${uri}`;

/** 失败文案：宿主返回结构化错误时原样取 message，其余按字符串处理。 */
export const islandNavigationFailure = (error: unknown): string => {
    if (typeof error === "string" && error.trim()) return `无法打开链接：${error.trim()}`;
    if (error instanceof Error && error.message.trim()) return `无法打开链接：${error.message.trim()}`;
    return "无法打开链接，请在插件设置中检查配对。";
};

/** 打开一条声明式导航；返回成功或页面可直接显示的原因，不吞掉宿主拒绝。 */
export const openIslandNavigation = async (
    open: (instanceId: string, uri: string) => Promise<void>,
    componentId: string,
    uri: string,
): Promise<{ ok: true } | { ok: false; message: string }> => {
    const destination = islandNavigationUri(uri);
    if (!destination) return { ok: false, message: "无法打开链接：地址为空。" };
    try {
        await open(islandNavigationInstanceId(componentId), destination);
        return { ok: true };
    } catch (error: unknown) {
        return { ok: false, message: islandNavigationFailure(error) };
    }
};
