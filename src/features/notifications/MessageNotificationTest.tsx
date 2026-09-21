import { publishMessage, type MessageNotification } from "./messageNotification";
import "./MessageNotificationTest.css";

/** Debug 格子的消息验收入口，直接使用正式消息管道。 */
export const MessageNotificationTest = () => {
    const messages: { label: string; message: MessageNotification }[] = [
        { label: "信息 1s", message: { type: "info", content: "信息测试：显示 1 秒。", source: "toast-test", durationMs: 1000 } },
        { label: "成功 5s", message: { type: "success", content: "成功测试：显示 5 秒。", source: "toast-test", durationMs: 5000 } },
        { label: "警告 2s", message: { type: "warning", content: "警告测试：显示 2 秒。", source: "toast-test", durationMs: 2000 } },
        { label: "错误 8s", message: { type: "error", content: "错误测试：显示 8 秒。", source: "toast-test", durationMs: 8000 } },
        { label: "Loading 12s", message: { type: "info", content: "加载测试：显示 12 秒。", source: "toast-test", loading: true, durationMs: 12000 } },
    ]; // 不设置去重键，允许连续点击验证堆叠。
    return (
        <aside
            className="message-notification-test"
            aria-label="消息测试"
            onPointerDown={(event) => event.stopPropagation()}
            onKeyDown={(event) => event.stopPropagation()}
            onContextMenu={(event) => {
                event.preventDefault();
                event.stopPropagation();
            }}
        >
            {messages.map(({ label, message }) => (
                <button key={label} type="button" onClick={() => publishMessage(message)}>
                    {label}
                </button>
            ))}
            <button type="button" onClick={() => messages.forEach(({ message }) => publishMessage(message))}>
                连续发送 5 条
            </button>
        </aside>
    );
};
