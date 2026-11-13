import { useEffect, useLayoutEffect, useRef, type KeyboardEvent } from "react";

import type { PopupLayerSpace } from "../../contracts/popupLayer";
import type { MenuFrame } from "./PopupMenu";

/** 确认框载荷；正文按段落给出，避免把富文本带进弹层协议。 */
export interface ConfirmPayload {
    title: string; // 顶部问句。
    body: string[]; // 分段正文。
    confirmLabel: string; // 主命令文案。
    cancelLabel?: string; // 取消文案，缺省为「取消」。
    danger?: boolean; // 破坏性命令使用错误色并默认聚焦取消。
}

/** 判断载荷是否是可用的确认框内容。 */
export const isConfirmPayload = (value: unknown): value is ConfirmPayload => {
    const item = value as Partial<ConfirmPayload> | null;
    return (
        typeof item?.title === "string" &&
        Array.isArray(item.body) &&
        item.body.every((line) => typeof line === "string") &&
        typeof item.confirmLabel === "string"
    );
};

/** 置顶弹层里的确认框：只有是/否两种结论，业务逻辑留在宿主。 */
export const PopupConfirm = ({
    payload,
    available,
    onLayout,
    onConfirm,
    onDismiss,
}: {
    payload: ConfirmPayload; // 问句、正文与按钮文案。
    available: PopupLayerSpace; // 锚点四周可用空间，用于把窗口摆到工作区中心。
    onLayout: (frame: MenuFrame) => void;
    onConfirm: () => void; // 用户确认。
    onDismiss: () => void; // 用户取消或按 Esc。
}) => {
    const dialog = useRef<HTMLDivElement>(null); // 弹窗本体，同时用于测量与聚焦。
    const cancel = useRef<HTMLButtonElement>(null); // 破坏性命令的默认焦点。
    const confirm = useRef<HTMLButtonElement>(null); // 普通命令的默认焦点。
    const reported = useRef(""); // 已上报的摆放，避免重复定位。

    useLayoutEffect(() => {
        const bounds = dialog.current?.getBoundingClientRect();
        if (!bounds) return;
        const key = `${bounds.width.toFixed(2)}x${bounds.height.toFixed(2)}`;
        if (reported.current === key) return;
        reported.current = key;
        // 对话框不跟光标：把窗口摆到工作区中心（相对锚点给出偏移），四周留出阴影边距。
        const width = bounds.width + 16;
        const height = bounds.height + 16;
        onLayout({
            width,
            height,
            anchorOffsetX: (available.right - available.left) / 2 - width / 2,
            anchorOffsetY: (available.bottom - available.top) / 2 - height / 2,
        });
    }, [available, onLayout]);

    useEffect(() => {
        (payload.danger ? cancel : confirm).current?.focus();
    }, [payload.danger]);

    /** Esc 取消、Enter 确认，其余按键不拦截。 */
    const handleKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
        if (event.key === "Escape") {
            event.preventDefault();
            onDismiss();
        } else if (event.key === "Enter" && event.target === dialog.current) {
            event.preventDefault();
            onConfirm();
        }
    };

    return (
        <div
            ref={dialog}
            className="popup-confirm"
            role="dialog"
            aria-modal="true"
            aria-label={payload.title}
            tabIndex={-1}
            onPointerDown={(event) => event.stopPropagation()}
            onKeyDown={handleKeyDown}
        >
            <h2 className="popup-confirm__title">{payload.title}</h2>
            {payload.body.map((line, index) => (
                <p className="popup-confirm__body" key={index}>
                    {line}
                </p>
            ))}
            <div className="popup-confirm__actions">
                <button ref={cancel} type="button" onClick={onDismiss}>
                    {payload.cancelLabel ?? "取消"}
                </button>
                <button ref={confirm} type="button" className={payload.danger ? "popup-confirm__danger" : undefined} onClick={onConfirm}>
                    {payload.confirmLabel}
                </button>
            </div>
        </div>
    );
};
