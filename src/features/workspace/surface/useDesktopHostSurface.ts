import { useCallback, useEffect, useState } from "react";

import {
    beginDesktopHostInputDrag,
    endDesktopHostInputDrag,
    getDesktopHostSnapshot,
    listenDesktopHostSnapshots,
    publishDesktopHostReadiness,
    recordDesktopHostInputProbe,
    recordDesktopHostRendererHeartbeat,
    updateDesktopHostHitMap,
} from "../../../bridge/desktopHost";
import type { DesktopHostSnapshot } from "../../../contracts/desktopHost";
import type { DesktopItemSnapshot } from "../../../contracts/desktopItem";
import { usePreferences } from "../../settings/PreferencesProvider";
import { collectHitRegions } from "./workspaceSurfaceProjection";
import { waitForDesktopItemImages } from "../../desktop-items/desktopItemReadiness";

/** 将未知错误转换为正式宿主用户可见说明。 */
const hostErrorMessage = (error: unknown): string => (error instanceof Error ? error.message : "桌面宿主请求失败");

/** 等待浏览器完成连续两帧呈现。 */
const waitForTwoFrames = (): Promise<void> =>
    new Promise((resolve) => {
        requestAnimationFrame(() => requestAnimationFrame(() => resolve()));
    });

/** 驱动正式桌面宿主页面门槛、命中图和最小输入探针。 */
export const useDesktopHostSurface = (enabled: boolean, monitorIndex: number, items?: DesktopItemSnapshot, layoutRevision = 0) => {
    const { preferences } = usePreferences(); // 外观与格子显隐变更后重发命中图。
    const [snapshot, setSnapshot] = useState<DesktopHostSnapshot>(); // 当前正式宿主快照。
    const [failure, setFailure] = useState<string>(); // 当前正式宿主错误说明。
    const host = snapshot?.hosts[monitorIndex]; // 当前页面枚举索引对应的逐屏宿主上下文。

    useEffect(() => {
        if (!enabled) return;
        let isMounted = true; // 当前正式宿主页面挂载标记。
        // 接收并保存正式宿主快照。
        const handleSnapshot = (nextSnapshot: DesktopHostSnapshot) => {
            if (!isMounted) return;
            setSnapshot(nextSnapshot);
            setFailure(undefined);
        };
        // 保存正式宿主订阅失败说明。
        const handleFailure = (error: unknown) => {
            if (isMounted) setFailure(hostErrorMessage(error));
        };
        const subscription = listenDesktopHostSnapshots(handleSnapshot, handleFailure); // 正式宿主快照订阅。
        void subscription.catch(handleFailure);
        void getDesktopHostSnapshot().then(handleSnapshot).catch(handleFailure);
        return () => {
            isMounted = false;
            void subscription.then((unlisten) => unlisten()).catch(() => undefined);
        };
    }, [enabled]);

    useEffect(() => {
        if (!enabled || !host || !items || items.revision === 0) return;
        let cancelled = false; // 当前页面准备任务取消标记。
        void (async () => {
            await waitForTwoFrames();
            const imageProbe = await waitForDesktopItemImages(); // 桌面项目模块提供的实际系统图像就绪结果。
            if (cancelled) return;
            const hitRegions = collectHitRegions(); // 首份完整交互命中图。
            // 已发布全部前端门槛的正式宿主快照。
            const nextSnapshot = await publishDesktopHostReadiness({
                monitorId: host.monitor.id,
                viewportWidth: window.innerWidth,
                viewportHeight: window.innerHeight,
                devicePixelRatio: window.devicePixelRatio,
                totalSystemImages: imageProbe.total,
                loadedSystemImages: imageProbe.loaded,
                hitRegions,
            });
            if (cancelled) return;
            setSnapshot(nextSnapshot);
        })().catch((error: unknown) => {
            if (!cancelled) setFailure(hostErrorMessage(error));
        });
        return () => {
            cancelled = true;
        };
    }, [enabled, host?.monitor.id, items]);

    useEffect(() => {
        if (!enabled || !host || !items) return;
        let frame = 0; // 合并页面布局变化的动画帧标识。
        // 发布当前页面完整命中图。
        const publishHitMap = () => {
            cancelAnimationFrame(frame);
            frame = requestAnimationFrame(() => {
                void updateDesktopHostHitMap(host.monitor.id, collectHitRegions()).catch((error: unknown) => setFailure(hostErrorMessage(error)));
            });
        };
        // 监视页面布局变化并发布最新完整命中图。
        const observer = new ResizeObserver(publishHitMap);
        // 菜单与功能格子挂载、移除后重发命中图；忽略时钟文本和拖动预览更新。
        const menuObserver = new MutationObserver((records) => {
            if (
                records.some(
                    (record) =>
                        (record.target instanceof Element && record.target.closest(".desktop-tile-menu")) ||
                        (record.type === "attributes" &&
                            record.target instanceof Element &&
                            record.target.matches(".desktop-tile,.desktop-tile-header,.desktop-tile-viewport")) ||
                        [...record.addedNodes, ...record.removedNodes].some(
                            (node) => node instanceof Element && (node.matches("[data-hit-id]") || node.querySelector("[data-hit-id]")),
                        ),
                )
            )
                publishHitMap();
        });
        menuObserver.observe(document.querySelector(".fence-workspace") ?? document.documentElement, {
            childList: true,
            subtree: true,
            attributes: true,
            attributeFilter: ["style", "hidden", "data-keyboard-focus", "data-visible"],
        });
        observer.observe(document.documentElement);
        document.addEventListener("scroll", publishHitMap, true);
        publishHitMap();
        return () => {
            cancelAnimationFrame(frame);
            observer.disconnect();
            menuObserver.disconnect();
            document.removeEventListener("scroll", publishHitMap, true);
        };
    }, [enabled, host?.monitor.id, items, layoutRevision, preferences]);

    useEffect(() => {
        if (!enabled || !host) return;
        const publishHeartbeat = () => {
            void recordDesktopHostRendererHeartbeat(host.monitor.id).catch((error: unknown) => setFailure(hostErrorMessage(error)));
        }; // 当前逐屏 Renderer 心跳上报。
        publishHeartbeat();
        const timer = window.setInterval(publishHeartbeat, 1000); // 宿主 watchdog 使用的一秒心跳。
        return () => window.clearInterval(timer);
    }, [enabled, host?.monitor.id]);

    // 将真实桌面项目指针输入回传正式宿主。
    const handleItemInput = useCallback(
        (itemId: string) => {
            if (!enabled || !host) return;
            void recordDesktopHostInputProbe(host.monitor.id, itemId)
                .then(setSnapshot)
                .catch((error: unknown) => setFailure(hostErrorMessage(error)));
        },
        [enabled, host],
    );

    // 暂停命中裁剪并进入连续拖动输入状态。
    const beginInputDrag = useCallback(async () => {
        try {
            if (!host) throw new Error("当前显示器宿主尚未就绪");
            setSnapshot(await beginDesktopHostInputDrag(host.monitor.id));
            setFailure(undefined);
        } catch (error: unknown) {
            setFailure(hostErrorMessage(error));
            throw error;
        }
    }, [host]);

    // 恢复最近完整命中图并结束连续拖动输入状态。
    const endInputDrag = useCallback(async () => {
        try {
            if (!host) throw new Error("当前显示器宿主尚未就绪");
            setSnapshot(await endDesktopHostInputDrag(host.monitor.id));
            setFailure(undefined);
        } catch (error: unknown) {
            setFailure(hostErrorMessage(error));
            throw error;
        }
    }, [host]);

    return { snapshot, host, failure, handleItemInput, beginInputDrag, endInputDrag };
};
