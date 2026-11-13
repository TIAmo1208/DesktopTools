import { currentHostMonitorIndex, listenPopupLayerResult, openPopupLayer, updatePopupLayerPayload } from "../../bridge/popupLayer";
import { menuItemAtPath, menuPayload } from "../tiles/TileMenuList";
import type { DesktopTileMenuItem } from "../tiles/DesktopTileMenu";

/** 宿主菜单请求的自增序号；与时间戳拼成进程内唯一身份。 */
let sequence = 0;

/** 生成一次请求身份；弹层结果必须回传同一值才算本次请求。 */
const nextRequestId = (): number => {
    sequence = (sequence + 1) % 1000;
    return Date.now() * 1000 + sequence;
};

/** 打开宿主菜单的补充行为。 */
export interface TileMenuOptions {
    focusFirst?: boolean; // 键盘打开菜单时把焦点放到第一项；指针打开不显示选中框。
    onSettled?: (dismissed: boolean) => void; // 菜单结束（取消或已选命令）后恢复宿主状态。
    refresh?: () => DesktopTileMenuItem[]; // 复选菜单保持打开时读取最新命令状态。
}

/**
 * 在置顶弹层里打开桌面格子菜单。
 *
 * 命令闭包留在宿主：弹层只回传 `indexPath`，由这里找回原项执行，业务逻辑不迁进弹层。
 */
export const openTileMenu = (items: DesktopTileMenuItem[], anchor: { left: number; top: number }, options: TileMenuOptions = {}): void => {
    const requestId = nextRequestId();
    const monitor = currentHostMonitorIndex(); // 与逐屏宿主创建顺序一致的显示器索引。
    let active = true; // 退订后丢弃迟到结果。
    let stop: (() => void) | undefined;
    const release = () => {
        active = false;
        stop?.();
        stop = undefined;
    };
    /** 复选菜单保持打开：把最新勾选状态推给同一弹层。 */
    const refresh = () => {
        const latest = options.refresh?.();
        if (!latest) return;
        void updatePopupLayerPayload(requestId, { items: menuPayload(latest), focusFirst: options.focusFirst === true }).catch((error: unknown) =>
            console.warn("弹层菜单：刷新命令状态失败", error),
        );
    };
    void listenPopupLayerResult((result) => {
        if (!active || result.requestId !== requestId) return;
        // 保持打开的菜单只回传选择，订阅与原生请求都要继续保留。
        const stayOpen = result.keepOpen === true && result.kind === "menu" && !result.dismissed && result.indexPath?.length;
        if (!stayOpen) release();
        if (result.kind !== "menu" || result.dismissed || !result.indexPath?.length) {
            options.onSettled?.(true);
            return;
        }
        const item = menuItemAtPath(items, result.indexPath); // 只在打开的这份命令表里查，避免执行新状态下的同名项。
        if (!item || item.disabled) {
            if (!stayOpen) options.onSettled?.(true);
            return;
        }
        try {
            item.onSelect?.();
        } finally {
            // 复选菜单保持打开：等宿主渲染出新状态后把最新命令表推回同一弹层。
            if (stayOpen) requestAnimationFrame(refresh);
            else options.onSettled?.(false);
        }
    })
        .then((unlisten) => {
            if (active) stop = unlisten;
            else unlisten();
            return openPopupLayer({
                requestId,
                kind: "menu",
                monitor,
                anchorX: anchor.left,
                anchorY: anchor.top,
                payload: { items: menuPayload(items), focusFirst: options.focusFirst === true },
            });
        })
        .catch((error: unknown) => {
            release();
            options.onSettled?.(true);
            console.warn("弹层菜单：打开失败", error);
        });
};
