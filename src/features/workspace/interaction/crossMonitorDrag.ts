import { invokeRuntime } from "../../../bridge/runtime";
import type { WorkspacePoint, WorkspaceRect } from "../../../contracts/workspace";

/** 跨屏预览的语义项目，禁止传输或执行 HTML。 */
export interface DragVisual {
    itemId?: string; // 使用目标页已有的 Shell 快照绘制。
    title?: string; // 围栏标题。
    bounds: WorkspaceRect; // 源屏 DIP 边界。
}

/** 源宿主维护的短生命周期拖动信息。 */
export interface CrossMonitorDrag {
    session: string; // 独立会话身份。
    source: string; // 源显示器。
    target: string; // 指针所在显示器。
    pointer: WorkspacePoint; // 目标屏 DIP 坐标。
    delta: WorkspacePoint; // 源屏 DIP 位移。
    origin: WorkspacePoint; // 源屏指针起点。
    kind: "items" | "fence"; // 预览语义。
    visuals: DragVisual[]; // 本次预览对象。
    itemIds: string[]; // 原子提交的完整拖动集合。
    committed?: boolean; // 成功提交后保留目标滚动，否则恢复临时状态。
}

/** 目标屏使用实际滚动 DOM 计算出的投放结果。 */
export interface CrossMonitorDrop {
    snapTargets?: WorkspaceRect[]; // 目标宿主实际可见格子的 DIP 边界，不包含拖动预览。
    rejected?: boolean; // 当前最上层格子不接受文件，禁止退化为背景投放。
    fenceId?: string; // 可接收项目的围栏。
    beforeItemId?: string | null; // 目标插入后继。
    scrollTop?: number; // 自动滚动后的最终位置。
}

let activeDrag: CrossMonitorDrag | undefined; // 当前页面拥有的最新拖动，静止时仍需保持目标预览。
let heartbeat: ReturnType<typeof setInterval> | undefined; // 源页面结束或卸载时自动释放。

/** 经原生宿主转发消息，静止拖动也维持目标页的过期保护。 */
export const publishCrossMonitorDrag = (drag: CrossMonitorDrag, phase: "move" | "drop" | "end") => {
    if (phase === "move") {
        activeDrag = drag;
        heartbeat ??= setInterval(() => {
            if (activeDrag) void invokeRuntime("relay_workspace_drag", { payload: { ...activeDrag, phase: "move" } }).catch(() => undefined);
        }, 500);
    }
    if ((phase === "end" || phase === "drop") && activeDrag?.session === drag.session) {
        clearInterval(heartbeat);
        heartbeat = undefined;
        activeDrag = undefined;
    }
    return invokeRuntime<void>("relay_workspace_drag", { payload: { ...drag, phase } });
};

/** 释放时向目标页重新查询一次，避免提交上一帧的命中。 */
export const resolveCrossMonitorDrop = (drag: CrossMonitorDrag): Promise<CrossMonitorDrop> =>
    new Promise((resolve, reject) => {
        const cleanup = () => {
            window.clearTimeout(timer);
            window.removeEventListener("desktop-tools-drag", handleReply);
        }; // 回应或超时均释放监听。
        const handleReply = (event: Event) => {
            const reply = (event as CustomEvent).detail; // 目标页返回的本次结果。
            if (reply?.phase !== "result" || reply.session !== drag.session || reply.target !== drag.target) return;
            cleanup();
            resolve(reply.drop ?? {});
        };
        const timer = window.setTimeout(() => {
            cleanup();
            reject(new Error("目标屏未响应拖动，项目已恢复原位"));
        }, 2000); // Renderer 失效时不猜测落点。
        window.addEventListener("desktop-tools-drag", handleReply);
        void publishCrossMonitorDrag(drag, "drop").catch((error: unknown) => {
            cleanup();
            reject(error);
        });
    });
