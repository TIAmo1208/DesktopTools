import { createRoot, type Root } from "react-dom/client";

import type { OpenPopupLayerRequest } from "../../bridge/popupLayer";
import type { PopupLayerResult, PopupLayerSpace } from "../../contracts/popupLayer";
import type { TileMenuPayloadItem } from "../tiles/TileMenuList";
import { isConfirmPayload, PopupConfirm } from "./PopupConfirm";
import { PopupMenu } from "./PopupMenu";

/** 断言页视口足够大，菜单与确认框都按未收敛的默认可用空间摆放。 */
const 断言页空间: PopupLayerSpace = { left: 0, top: 0, right: 960, bottom: 720 };

/** 弹层夹具句柄：容器里的 DOM 与真实弹层一致，供断言直接查询。 */
export interface 弹层夹具 {
    容器: HTMLDivElement; // 浮层在本页的挂载点。
    处理请求: (request: OpenPopupLayerRequest) => void; // 假桥收到 open_popup_layer 后调用。
    更新载荷: (requestId: number, payload: Record<string, unknown>) => void; // 复选菜单保持打开时的换载荷。
    销毁: () => void; // 卸载浮层并移除容器。
}

/**
 * 断言页的置顶弹层替身：把宿主请求渲染成真实的 PopupMenu / PopupConfirm，
 * 选择或确认按 `popup-layer-result` 协议回传给页面的假桥，替代第二个弹层窗口。
 *
 * 只做宿主与弹层之间的协议接线；菜单项闭包、业务命令仍由被断言的格子执行。
 */
export const 创建弹层夹具 = ({ 回传 }: { 回传: (result: PopupLayerResult) => void }): 弹层夹具 => {
    const 容器 = document.createElement("div");
    容器.dataset.popupFixture = "true";
    document.body.append(容器);
    let root: Root | undefined;
    let 当前: OpenPopupLayerRequest | undefined; // 最近的弹层请求，更新载荷时沿用它。

    /** 结束本次请求：按协议回传结果，并像原生窗口一样收掉浮层内容。 */
    const 结束 = (结果: Omit<PopupLayerResult, "requestId" | "kind">) => {
        if (!当前) return;
        回传({ requestId: 当前.requestId, kind: 当前.kind, ...结果 });
        if (!结果.keepOpen) {
            root?.render(null);
            当前 = undefined;
        }
    };

    /** 按请求种类装配浮层；重新渲染沿用同一 requestId 作为 key，键盘焦点不丢。 */
    const 渲染 = (请求: OpenPopupLayerRequest) => {
        当前 = 请求;
        root ??= createRoot(容器);
        if (请求.kind === "menu") {
            const 载荷 = 请求.payload as { items?: TileMenuPayloadItem[]; focusFirst?: boolean };
            root.render(
                <PopupMenu
                    key={请求.requestId}
                    items={载荷.items ?? []}
                    available={断言页空间}
                    focusFirst={载荷.focusFirst === true}
                    onLayout={() => {}}
                    onSelect={(路径, 保持打开) => 结束({ indexPath: 路径, dismissed: false, keepOpen: 保持打开 })}
                    onDismiss={() => 结束({ dismissed: true })}
                />,
            );
            return;
        }
        if (!isConfirmPayload(请求.payload)) return;
        root.render(
            <PopupConfirm
                key={请求.requestId}
                payload={请求.payload}
                available={断言页空间}
                onLayout={() => {}}
                onConfirm={() => 结束({ confirmed: true, dismissed: false })}
                onDismiss={() => 结束({ confirmed: false, dismissed: true })}
            />,
        );
    };

    return {
        容器,
        处理请求: 渲染,
        更新载荷: (requestId, payload) => {
            if (!当前 || 当前.requestId !== requestId) return;
            渲染({ ...当前, payload });
        },
        销毁: () => {
            root?.unmount();
            root = undefined;
            当前 = undefined;
            容器.remove();
        },
    };
};
