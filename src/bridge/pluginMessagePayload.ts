/** 插件消息命令载荷：uri 只在声明时出现，旧载荷保持 5 字段。 */
export const publishIslandMessagePayload = (
    instanceId: string,
    key: string,
    kind: string,
    text: string,
    uri?: string,
): { instanceId: string; key: string; kind: string; text: string; uri?: string } => ({
    instanceId,
    key,
    kind,
    text,
    ...(uri === undefined ? {} : { uri }),
});
