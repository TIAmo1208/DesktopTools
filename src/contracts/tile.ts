/** 所有格子共用的外观，独立于空间布局与内容私有状态。 */
export interface TileAppearance {
    title: string | null; // 未设置时使用内容默认名称。
    titleMode: "fixed" | "hover"; // 标题占位或悬浮覆盖。
    buttonMode: "fixed" | "hover"; // 固定标题下的工具偏好。
    collapsed: boolean; // 保存的折叠状态。
    autoExpand: boolean; // 折叠时临时展开。
}

/** 单字段外观修改，不覆盖其他窗口已保存的偏好。 */
export type TileAppearanceChange = {
    [Key in keyof TileAppearance]: {
        type: Key extends "titleMode" ? "title-mode" : Key extends "buttonMode" ? "button-mode" : Key extends "autoExpand" ? "auto-expand" : Key;
        value: NonNullable<TileAppearance[Key]>;
    };
}[keyof TileAppearance];

/** 从宿主边界校验共享外观表，缺省使用默认策略。 */
export const parseTileAppearances = (value: unknown): Record<string, TileAppearance> => {
    if (value === undefined) return {};
    if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error("格子外观数据无效");
    return Object.fromEntries(
        Object.entries(value).map(([key, entry]) => {
            if (!key.startsWith("fence:") && !key.startsWith("component:")) throw new Error("格子外观身份无效");
            if (!entry || typeof entry !== "object" || Array.isArray(entry)) throw new Error("格子外观数据无效");
            const appearance = entry as Record<string, unknown>; // 完成逐字段校验后才发布给界面。
            if (appearance.title !== null && typeof appearance.title !== "string") throw new Error("格子标题无效");
            if (appearance.titleMode !== "fixed" && appearance.titleMode !== "hover") throw new Error("格子标题策略无效");
            if (appearance.buttonMode !== "fixed" && appearance.buttonMode !== "hover") throw new Error("格子按钮策略无效");
            if (typeof appearance.collapsed !== "boolean" || typeof appearance.autoExpand !== "boolean") throw new Error("格子折叠策略无效");
            return [
                key,
                {
                    title: appearance.title,
                    titleMode: appearance.titleMode,
                    buttonMode: appearance.buttonMode,
                    collapsed: appearance.collapsed,
                    autoExpand: appearance.autoExpand,
                } as TileAppearance,
            ];
        }),
    );
};
