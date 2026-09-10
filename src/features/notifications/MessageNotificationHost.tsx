import { useSyncExternalStore } from "react";
import { dismissMessage, getMessages, subscribeMessages } from "./messageNotification";
import "./MessageNotificationHost.css";

/** 在当前窗口原有右下角位置依次呈现程序消息，不转移键盘焦点。 */
export const MessageNotificationHost = () => {
    const messages = useSyncExternalStore(subscribeMessages, getMessages); // 当前窗口未关闭的消息快照。
    const message = messages[0]; // 本次呈现的队首消息。
    return (
        <aside
            onPointerDown={(event) => event.stopPropagation()}
            onKeyDown={(event) => event.stopPropagation()}
            onContextMenu={(event) => {
                event.preventDefault();
                event.stopPropagation();
            }}
        >
            {message && (
                <p
                    className="message-notification"
                    data-type={message?.type}
                    role={message?.type === "error" || message?.type === "warning" ? "alert" : "status"}
                    aria-atomic="true"
                    data-hit-id="workspace-status"
                    data-hit-kind="workspace-status"
                    data-hit-priority="80"
                >
                    {message && (
                        <>
                            <span>{message.content}</span>
                            <button type="button" aria-label="关闭消息" onClick={() => dismissMessage(message)}>
                                关闭
                            </button>
                        </>
                    )}
                </p>
            )}
        </aside>
    );
};
