import type { DesktopItemSnapshot } from "../../contracts/desktopItem";

/** 页面呈现使用的桌面项目展示模型。 */
export interface DesktopItemViewModel {
    key: string;
    label: string;
    imageSource: string | null;
    imageRevision?: number; // 事实变化后隔离迟到图像。
    isAvailable: boolean;
}

/** 将 Shell 桌面项目快照投影为页面展示模型。 */
export const projectDesktopItems = (snapshot: DesktopItemSnapshot): DesktopItemViewModel[] =>
    snapshot.items.map((item) => ({
        key: item.stableReference,
        label: item.displayName,
        imageSource: item.systemImage,
        imageRevision: snapshot.revision,
        isAvailable: item.availability === "available",
    }));
