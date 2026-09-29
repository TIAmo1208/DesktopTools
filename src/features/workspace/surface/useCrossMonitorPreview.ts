import { useEffect, useRef } from "react";
import type { DesktopItemSnapshot } from "../../../contracts/desktopItem";
import type { WorkspaceSnapshot } from "../../../contracts/workspace";
import { invokeRuntime } from "../../../bridge/runtime";
import type { CrossMonitorPreviewEvent } from "../interaction/crossMonitorDrag";
import { createWorkspaceDropTarget } from "./workspaceDropTarget";
import { createCrossMonitorDragPreview } from "./crossMonitorDragPreview";

/** 目标宿主拥有预览节点与真实 DOM 命中，源宿主仍独占事务提交。 */
export const useCrossMonitorPreview = (monitorId: string | undefined, items: DesktopItemSnapshot | undefined, workspace: WorkspaceSnapshot | undefined) => {
    const state = useRef({ items, workspace }); // 消息处理始终读取最新快照。
    state.current = { items, workspace };
    useEffect(() => {
        if (!monitorId) return;
        const preview = createCrossMonitorDragPreview(); // 当前会话复用的目标预览。
        let timeout = 0; // 源 Renderer 失效后清理预览。
        let session: string | undefined; // 当前接收会话。
        let sessionSource: string | undefined; // 当前会话来源。
        const endedSessions = new Map<string, string>(); // 每个来源保留最近结束会话，拒绝迟到恢复。
        let renderFrame = 0; // 等待悬停展开提交后的单一渲染帧。
        let pendingSession: string | undefined; // 首次呈现前收到结束时也能取消预览。
        let geometryRevision = 0; // 当前 Renderer 发布的几何帧版本。
        let newestOle: { session: string; sequence: number } | undefined; // 旧自动滚动帧不能覆盖新的原生指针。
        let pendingOperation: string | undefined; // 已释放后不再重放旧指针进行自动滚动。
        let replaying = false; // 自动滚动重算不延长源宿主心跳过期时间。
        const target = createWorkspaceDropTarget(monitorId, (drag) => {
            replaying = true;
            handleMessage(new CustomEvent("desktop-tools-drag", { detail: drag }), false);
            replaying = false;
        }); // 几何适配器请求重放本帧，不直接接触消息通道。
        /** 结束、切屏和卸载共用会话清理。 */
        const clear = (restoreScroll = false, finish = false) => {
            cancelAnimationFrame(renderFrame);
            renderFrame = 0;
            pendingSession = undefined;
            preview.clear();
            target.clear(restoreScroll, finish);
        };
        const handleMessage = (event: Event, deferDrop = true) => {
            const drag = (event as CustomEvent).detail as CrossMonitorPreviewEvent; // 本应用原生转发数据。
            if (!drag || typeof drag.session !== "string" || typeof drag.source !== "string" || drag.phase === "result") return;
            if (drag.phase === "end") endedSessions.set(drag.source, drag.session);
            else if (endedSessions.get(drag.source) === drag.session) return;
            if (drag.source === "ole" && drag.phase === "pending") {
                pendingOperation = drag.session;
                if (session === drag.session || pendingSession === drag.session) clear(false);
                return;
            }
            if (drag.phase === "move" && pendingOperation === drag.session) return;
            if (drag.source === "ole" && drag.phase === "move" && Number.isSafeInteger(drag.sequence)) {
                if (newestOle?.session === drag.session && drag.sequence! < newestOle.sequence) return;
                newestOle = { session: drag.session, sequence: drag.sequence! };
            }
            if (
                drag.target === monitorId &&
                drag.source !== monitorId &&
                (drag.phase === "drop" || (drag.source === "ole" && drag.phase === "move")) &&
                deferDrop
            ) {
                cancelAnimationFrame(renderFrame);
                pendingSession = drag.session;
                renderFrame = requestAnimationFrame(() => handleMessage(event, false));
                return;
            } // 给悬停展开一次 DOM 提交机会。
            if (drag.source === monitorId) return;
            if (drag.phase === "end" || drag.target !== monitorId) {
                if (pendingOperation === drag.session) pendingOperation = undefined;
                if (session === drag.session || pendingSession === drag.session) {
                    window.clearTimeout(timeout);
                    clear(drag.target !== monitorId || !drag.committed, drag.phase === "end");
                }
                return;
            }
            if (!["move", "drop"].includes(drag.phase) || !Array.isArray(drag.visuals) || drag.visuals.length > 512 || !Array.isArray(drag.itemIds)) return;
            if (![drag.pointer?.left, drag.pointer?.top, drag.origin?.left, drag.origin?.top].every(Number.isFinite)) return;
            if (session && session !== drag.session) {
                if (sessionSource) endedSessions.set(sessionSource, session);
                clear(true, true);
            }
            session = drag.session;
            sessionSource = drag.source;
            if (!replaying) {
                window.clearTimeout(timeout);
                timeout = window.setTimeout(() => {
                    endedSessions.set(drag.source, drag.session);
                    clear(true, true);
                }, 2500);
            }
            const geometry = target.update(drag, state.current.workspace); // 只消费目标适配器的投放和几何结果。
            preview.update(drag, state.current.items, state.current.workspace, geometry.drop.snapTargets);
            const drop = geometry.drop; // 源宿主独占布局事务提交。
            if (drag.source === "ole" && drag.phase === "move" && Number.isSafeInteger(drag.sequence) && drag.revision === state.current.workspace?.revision) {
                void invokeRuntime("relay_workspace_drag", {
                    payload: {
                        phase: "ole-preview",
                        geometryRevision: ++geometryRevision,
                        session: drag.session,
                        sequence: drag.sequence,
                        revision: drag.revision,
                        monitorId,
                        pointer: drag.pointer,
                        regions: geometry.regions,
                        fenceScrolls: geometry.fenceScrolls,
                    },
                }).catch(() => undefined);
            } // 只异步确认已渲染几何，原生 Drop 不等待该响应。
            if (drag.phase === "drop")
                void invokeRuntime("relay_workspace_drag", { payload: { phase: "result", session: drag.session, target: monitorId, drop } });
        };
        window.addEventListener("desktop-tools-drag", handleMessage);
        return () => {
            window.removeEventListener("desktop-tools-drag", handleMessage);
            window.clearTimeout(timeout);
            clear(true, true);
        };
    }, [monitorId]);
};
