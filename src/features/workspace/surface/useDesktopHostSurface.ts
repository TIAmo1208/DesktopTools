import { useCallback, useEffect, useState } from "react";

import {
    beginDesktopHostInputDrag,
    endDesktopHostInputDrag,
    getDesktopHostSnapshot,
    listenDesktopHostSnapshots,
    publishDesktopHostReadiness,
    recordDesktopHostInputProbe,
    updateDesktopHostHitMap,
} from "../../../bridge/desktopHost";
import type { DesktopHostSnapshot, HitRegion } from "../../../contracts/desktopHost";
import type { DesktopItemSnapshot } from "../../../contracts/desktopItem";

/** 将未知错误转换为正式宿主用户可见说明。 */
const hostErrorMessage = (error: unknown): string => (error instanceof Error ? error.message : "桌面宿主请求失败");

/** 等待浏览器完成连续两帧呈现。 */
const waitForTwoFrames = (): Promise<void> =>
    new Promise((resolve) => {
        requestAnimationFrame(() => requestAnimationFrame(() => resolve()));
    });

/** 收集页面当前完整语义命中图。 */
const collectHitRegions = (): HitRegion[] =>
    [...document.querySelectorAll<HTMLElement>("[data-hit-id]")].map((element) => {
        const bounds = element.getBoundingClientRect(); // 当前语义对象 CSS 像素边界。
        return {
            id: element.dataset.hitId ?? "",
            kind: element.dataset.hitKind ?? "",
            left: bounds.left,
            top: bounds.top,
            width: bounds.width,
            height: bounds.height,
            priority: Number(element.dataset.hitPriority ?? "0"),
        };
    });

/** 等待当前系统图像节点与 Shell 快照数量一致并完成解码。 */
const waitForSystemImages = async (expectedTotal: number): Promise<{ total: number; loaded: number }> => {
    // 当前系统图像解码的轮询次数。
    for (let attempt = 0; attempt < 20; attempt += 1) {
        const images = [...document.querySelectorAll<HTMLImageElement>(".desktop-item__image img")]; // 当前真实系统图像节点。
        await Promise.all(images.map((image) => image.decode().catch(() => undefined)));
        const loaded = images.filter((image) => image.naturalWidth > 0).length; // 当前实际解码成功的系统图像数量。
        if (images.length === expectedTotal && loaded === expectedTotal) return { total: images.length, loaded };
        await new Promise((resolve) => window.setTimeout(resolve, 100));
    }

    const images = [...document.querySelectorAll<HTMLImageElement>(".desktop-item__image img")]; // 超时后最终核对的系统图像节点。
    return { total: images.length, loaded: images.filter((image) => image.naturalWidth > 0).length };
};

/** 驱动正式桌面宿主页面门槛、命中图和最小输入探针。 */
export const useDesktopHostSurface = (enabled: boolean, items?: DesktopItemSnapshot, layoutRevision = 0) => {
    const [snapshot, setSnapshot] = useState<DesktopHostSnapshot>(); // 当前正式宿主快照。
    const [failure, setFailure] = useState<string>(); // 当前正式宿主错误说明。

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
        if (!enabled || !items || items.items.length === 0) return;
        let cancelled = false; // 当前页面准备任务取消标记。
        void (async () => {
            await waitForTwoFrames();
            const imageProbe = await waitForSystemImages(items.items.length); // 当前系统图像实际加载结果。
            if (cancelled) return;
            const hitRegions = collectHitRegions(); // 首份完整交互命中图。
            // 已发布全部前端门槛的正式宿主快照。
            const nextSnapshot = await publishDesktopHostReadiness({
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
    }, [enabled, items]);

    useEffect(() => {
        if (!enabled || !items) return;
        let frame = 0; // 合并页面布局变化的动画帧标识。
        // 发布当前页面完整命中图。
        const publishHitMap = () => {
            cancelAnimationFrame(frame);
            frame = requestAnimationFrame(() => {
                void updateDesktopHostHitMap(collectHitRegions()).catch((error: unknown) => setFailure(hostErrorMessage(error)));
            });
        };
        // 监视页面布局变化并发布最新完整命中图。
        const observer = new ResizeObserver(publishHitMap);
        observer.observe(document.documentElement);
        publishHitMap();
        return () => {
            cancelAnimationFrame(frame);
            observer.disconnect();
        };
    }, [enabled, items, layoutRevision]);

    // 将真实桌面项目指针输入回传正式宿主。
    const handleItemInput = useCallback(
        (itemId: string) => {
            if (!enabled) return;
            void recordDesktopHostInputProbe(itemId)
                .then(setSnapshot)
                .catch((error: unknown) => setFailure(hostErrorMessage(error)));
        },
        [enabled],
    );

    // 暂停命中裁剪并进入连续拖动输入状态。
    const beginInputDrag = useCallback(async () => {
        try {
            setSnapshot(await beginDesktopHostInputDrag());
            setFailure(undefined);
        } catch (error: unknown) {
            setFailure(hostErrorMessage(error));
            throw error;
        }
    }, []);

    // 恢复最近完整命中图并结束连续拖动输入状态。
    const endInputDrag = useCallback(async () => {
        try {
            setSnapshot(await endDesktopHostInputDrag());
            setFailure(undefined);
        } catch (error: unknown) {
            setFailure(hostErrorMessage(error));
            throw error;
        }
    }, []);

    return { snapshot, failure, handleItemInput, beginInputDrag, endInputDrag };
};
