/** 程序内统一消息通知的类型。 */
import { invokeRuntime } from "../../bridge/runtime.ts";

export type MessageNotificationType = "info" | "success" | "warning" | "error";

/** 外部模块提交给通知管道的消息。 */
export interface MessageNotification {
    type: MessageNotificationType; // 消息严重程度。
    content: string; // 纯文本消息正文。
    source: string; // 调用模块标识。
    dedupeKey?: string; // 同一来源待显示消息的去重标识。
    loading?: boolean; // 是否显示加载状态。
    durationMs?: number; // 显示时长，单位毫秒；省略时使用 4000ms，悬停暂停计时。
}

/** 向程序通知管道提交一条新消息。 */
export const publishMessage = (message: MessageNotification) => {
    if (!message.content.trim() || !message.source.trim()) return;
    if (!["info", "success", "warning", "error"].includes(message.type)) return;
    const durationMs = message.durationMs ?? 4000; // 每次发送给原生的消息都携带明确期限。
    if (!Number.isInteger(durationMs) || durationMs < 1 || durationMs > 0xffffffff) return;
    void Promise.resolve()
        .then(() => invokeRuntime("publish_message_notification", { message: { ...message, durationMs } }))
        .catch(() => undefined);
};
