/** 程序内统一消息通知的类型。 */
export type MessageNotificationType = "info" | "success" | "warning" | "error";

/** 外部模块提交给通知管道的消息。 */
export interface MessageNotification {
    type: MessageNotificationType; // 消息严重程度。
    content: string; // 纯文本消息正文。
    source: string; // 调用模块标识。
    dedupeKey?: string; // 同一来源待显示消息的去重标识。
}

let messages: readonly MessageNotification[] = []; // 当前窗口按提交顺序保留的未关闭消息。
const listeners = new Set<() => void>(); // 通知表面的更新订阅者。

/** 读取稳定的消息快照，挂载前提交的消息同样保留。 */
export const getMessages = () => messages;

/** 订阅通知变化，卸载时只移除当前订阅。 */
export const subscribeMessages = (onChange: () => void) => {
    listeners.add(onChange);
    return () => {
        listeners.delete(onChange);
    };
};

/** 关闭指定消息，避免迟到的关闭操作误删下一条。 */
export const dismissMessage = (message: MessageNotification) => {
    messages = messages.filter((item) => item !== message);
    listeners.forEach((onChange) => onChange());
};

/** 向程序通知管道提交一条新消息。 */
export const publishMessage = (message: MessageNotification) => {
    if (!message.content.trim() || !message.source.trim()) return;
    if (!["info", "success", "warning", "error"].includes(message.type)) return;
    if (message.dedupeKey && messages.some((item) => item.source === message.source && item.dedupeKey === message.dedupeKey)) return;
    messages = [...messages, { ...message }];
    listeners.forEach((onChange) => onChange());
};
