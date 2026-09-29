import { invokeRuntime } from "../../../bridge/runtime";
import type { WorkspacePoint, WorkspaceRect } from "../../../contracts/workspace";
import type { TileDragVisual } from "../../tiles/tileDragPreview";

/** 跨屏预览的语义项目，禁止传输或执行 HTML。 */
export interface DragVisual {
    content?: TileDragVisual; // 格子当前已渲染内容的只读快照。
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
    snap?: boolean; // 源格子当前吸附设置。
    minimum?: { width: number; height: number }; // 源内容有效最小尺寸。
}

/** 目标屏使用实际滚动 DOM 计算出的投放结果。 */
export interface CrossMonitorDrop {
    snapTargets?: WorkspaceRect[]; // 目标宿主实际可见格子的 DIP 边界，不包含拖动预览。
    rejected?: boolean; // 当前最上层格子不接受文件，禁止退化为背景投放。
    fenceId?: string; // 可接收项目的围栏。
    portalId?: string; // 可接收真实文件传输的门户。
    beforeItemId?: string | null; // 目标插入后继。
    scrollTop?: number; // 自动滚动后的最终位置。
}

/** 原生与跨屏消息共用的预览输入，序号与修订仅用于原生几何确认。 */
export interface CrossMonitorPreviewEvent extends CrossMonitorDrag {
    phase: string; // 当前会话阶段。
    sequence?: number; // 原生指针序号。
    revision?: number; // 工作区版本。
    modifiers?: number; // 原生修饰键。
    external?: boolean; // 外部来源决定复制占位规则。
    contentTargets?: string[]; // 原生业务白名单允许的组件类型，不包含正文或图像。
    fileContent?: boolean; // 非文件内容不得进入桌面或围栏的布局与传输流程。
}

let activeDrag: CrossMonitorDrag | undefined; // 当前页面拥有的最新拖动，静止时仍需保持目标预览。
let heartbeat: ReturnType<typeof setInterval> | undefined; // 源页面结束或卸载时自动释放。
let moveFrame = 0; // 每帧最多发送一次最新跨屏预览。
let pendingMove: CrossMonitorDrag | undefined; // 合并连续指针事件，避免重复序列化视觉树。
let sentVisual: { session: string; target: string } | undefined; // 同一目标连续移动只发送一次静态帧。

/** 后续几何消息及结束消息不再携带完整视觉树。 */
const geometryOnly = (drag: CrossMonitorDrag): CrossMonitorDrag => ({ ...drag, visuals: drag.visuals.map(({ content: _content, ...visual }) => visual) });

/** 沿现有消息形状在进入目标屏时发送静态帧，持续移动只更新几何。 */
const sendMove = (drag: CrossMonitorDrag) => {
    const repeated = sentVisual?.session === drag.session && sentVisual.target === drag.target; // 切换目标屏必须重新提供完整静态帧。
    const sent = repeated ? sentVisual! : { session: drag.session, target: drag.target }; // 首帧失败时，后续轻量消息不能掩盖重发需要。
    sentVisual = sent;
    return invokeRuntime("relay_workspace_drag", { payload: { ...(repeated ? geometryOnly(drag) : drag), phase: "move" } }).catch(() => {
        if (sentVisual === sent) sentVisual = undefined;
    });
};

/** 在下一帧发送最新跨屏状态，保留释放消息的即时通道。 */
const flushMove = () => {
    moveFrame = 0;
    const drag = pendingMove;
    pendingMove = undefined;
    if (drag) void sendMove(drag);
};

/** 浏览器帧调度不可用时使用一次短延迟，保持消息合并语义。 */
const scheduleMove = () => (typeof window.requestAnimationFrame === "function" ? window.requestAnimationFrame(flushMove) : window.setTimeout(flushMove, 16));

/** 取消尚未发送的合并帧。 */
const cancelMove = (frame: number) => (typeof window.cancelAnimationFrame === "function" ? window.cancelAnimationFrame(frame) : window.clearTimeout(frame));

/** 经原生宿主转发消息，静止拖动也维持目标页的过期保护。 */
export const publishCrossMonitorDrag = (drag: CrossMonitorDrag, phase: "move" | "drop" | "end") => {
    if (phase === "move") {
        activeDrag = drag;
        heartbeat ??= setInterval(() => {
            if (activeDrag) void sendMove(activeDrag);
        }, 500);
        pendingMove = drag;
        if (!moveFrame) moveFrame = scheduleMove();
    }
    if ((phase === "end" || phase === "drop") && activeDrag?.session === drag.session) {
        clearInterval(heartbeat);
        heartbeat = undefined;
        if (moveFrame) cancelMove(moveFrame);
        moveFrame = 0;
        pendingMove = undefined;
        activeDrag = undefined;
        sentVisual = undefined;
    }
    if (phase === "move") return Promise.resolve();
    return invokeRuntime<void>("relay_workspace_drag", { payload: { ...geometryOnly(drag), phase } });
};

/** 释放时向目标页重新查询一次，避免提交上一帧的命中。 */
export const resolveCrossMonitorDrop = (drag: CrossMonitorDrag, signal?: AbortSignal): Promise<CrossMonitorDrop> =>
    new Promise((resolve, reject) => {
        const cleanup = () => {
            window.clearTimeout(timer);
            window.removeEventListener("desktop-tools-drag", handleReply);
            signal?.removeEventListener("abort", handleAbort);
        }; // 回应或超时均释放监听。
        const handleReply = (event: Event) => {
            const reply = (event as CustomEvent).detail; // 目标页返回的本次结果。
            if (reply?.phase !== "result" || reply.session !== drag.session || reply.target !== drag.target) return;
            cleanup();
            resolve(reply.drop ?? {});
        };
        const handleAbort = () => {
            cleanup();
            reject(new DOMException("跨屏释放已取消", "AbortError"));
        }; // 取消立即释放握手监听，不等待超时或迟到回复。
        const timer = window.setTimeout(() => {
            cleanup();
            reject(new Error("目标屏未响应拖动，项目已恢复原位"));
        }, 2000); // Renderer 失效时不猜测落点。
        if (signal?.aborted) {
            handleAbort();
            return;
        }
        signal?.addEventListener("abort", handleAbort, { once: true });
        window.addEventListener("desktop-tools-drag", handleReply);
        void publishCrossMonitorDrag(drag, "drop").catch((error: unknown) => {
            cleanup();
            reject(error);
        });
    });
