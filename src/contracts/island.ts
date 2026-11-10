import { parseComponentRuntimeSnapshot, type ComponentInstanceSnapshot, type ComponentRuntimeSnapshot } from "./component.ts";

/** 岛快照独立于工作区：清单复用组件契约，实例是无布局的运行时合成值。 */
export interface IslandSnapshot {
    runtime: ComponentRuntimeSnapshot;
    instances: ComponentInstanceSnapshot[];
}

/** 原生拖动完成后的事实：指针是否仍在岛内，以及本次落点的沿边偏移。 */
export interface IslandDragResult {
    pointerInside: boolean;
    offset: number | null; // 页面在权威偏好事件到达前按该偏移重测，避免回跳。
}

/** 拖动返回值严格校验，缺失或未知字段不能被当作成功命中。 */
export const parseIslandDragResult = (value: unknown): IslandDragResult => {
    const item = record(value);
    if (
        Object.keys(item).length !== 2 ||
        !Object.hasOwn(item, "pointerInside") ||
        typeof item.pointerInside !== "boolean" ||
        !Object.hasOwn(item, "offset") ||
        (item.offset !== null && (typeof item.offset !== "number" || !Number.isFinite(item.offset) || Math.abs(item.offset) > 1_000_000))
    )
        throw new Error("岛拖动结果无效");
    return { pointerInside: item.pointerInside, offset: item.offset as number | null };
};

/** 一次原生提交的屏幕 DIP 矩形。 */
export interface IslandRect {
    left: number;
    top: number;
    width: number;
    height: number;
}

/** 三态几何由页面实测，一次提交全部窗口、固定页面视口与局部卡片区域。 */
export interface IslandStateRequest {
    state: "collapsed" | "hover" | "expanded";
    windowBox: IslandRect;
    viewport: IslandRect; // 固定页面原点所在的屏幕视口带；父窗口只负责裁剪，内容不随窗口移动。
    cards: { rect: IslandRect; radius: number }[];
}

/** 边界对象校验，不接受数组或空值。 */
const record = (value: unknown): Record<string, unknown> => {
    if (typeof value !== "object" || value === null || Array.isArray(value)) throw new Error("岛数据无效");
    return value as Record<string, unknown>;
};

/** 原生坐标使用有界有限数，尺寸必须为正。 */
const parseRect = (value: unknown): IslandRect => {
    const item = record(value); // 待校验矩形。
    for (const key of ["left", "top", "width", "height"]) {
        const field = item[key]; // 与 Rust valid_rect 保持一致。
        if (typeof field !== "number" || !Number.isFinite(field) || Math.abs(field) > 1_000_000) throw new Error("岛区域无效");
    }
    if ((item.width as number) <= 0 || (item.height as number) <= 0) throw new Error("岛区域尺寸无效");
    return { left: item.left as number, top: item.top as number, width: item.width as number, height: item.height as number };
};

/** 与原生 IslandStateRequest 相同的几何门禁，错误在进入 IPC 前明确拒绝。 */
export const parseIslandStateRequest = (value: unknown): IslandStateRequest => {
    const item = record(value); // 三态几何请求。
    if (Object.keys(item).some((key) => !["state", "windowBox", "viewport", "cards"].includes(key))) throw new Error("岛状态字段无效");
    if (item.state !== "collapsed" && item.state !== "hover" && item.state !== "expanded") throw new Error("未知岛状态");
    const windowBox = parseRect(item.windowBox); // 屏幕绝对坐标。
    const viewport = parseRect(item.viewport); // 固定页面原点所在的视口带。
    if (viewport.width > 16_384 || viewport.height > 16_384) throw new Error("岛固定视口过大");
    const tolerance = 0.5; // 原生与页面各自量化，允许半个 DIP 的边界误差。
    if (
        windowBox.left < viewport.left - tolerance ||
        windowBox.top < viewport.top - tolerance ||
        windowBox.left + windowBox.width > viewport.left + viewport.width + tolerance ||
        windowBox.top + windowBox.height > viewport.top + viewport.height + tolerance
    )
        throw new Error("岛窗口盒超出固定视口");
    if (!Array.isArray(item.cards) || item.cards.length === 0 || item.cards.length > 128 || (item.state === "collapsed" && item.cards.length !== 1))
        throw new Error("岛卡片数量无效");
    const cards = item.cards.map((value) => {
        const card = record(value); // 窗口局部卡片区域。
        if (Object.keys(card).some((key) => key !== "rect" && key !== "radius")) throw new Error("岛卡片字段无效");
        const rect = parseRect(card.rect);
        const radius = card.radius;
        if (
            rect.left < 0 ||
            rect.top < 0 ||
            rect.left + rect.width > windowBox.width ||
            rect.top + rect.height > windowBox.height ||
            typeof radius !== "number" ||
            !Number.isFinite(radius) ||
            radius < 0 ||
            radius > Math.min(rect.width, rect.height) / 2
        )
            throw new Error("岛卡片区域或圆角无效");
        return { rect, radius };
    });
    return { state: item.state, windowBox, viewport, cards };
};

/** 合成实例没有持久尺寸，不能套用桌面格子的最小宽高门禁。 */
export const parseIslandSnapshot = (value: unknown): IslandSnapshot => {
    const item = record(value); // 岛命令只返回岛实例与组件契约投影。
    const runtime = parseComponentRuntimeSnapshot(item.runtime);
    if (!Array.isArray(item.instances) || item.instances.length > 32) throw new Error("岛实例数量无效");
    const seen = new Set<string>(); // 实例身份必须唯一。
    const instances = item.instances.map((value): ComponentInstanceSnapshot => {
        const instance = record(value);
        const bounds = record(instance.bounds);
        const componentId = instance.componentId; // 前缀与后端偏好规范化规则一致。
        if (
            typeof componentId !== "string" ||
            !["official.", "plugin."].some((prefix) => componentId.startsWith(prefix) && componentId.length > prefix.length) ||
            instance.instanceId !== `island.${instance.componentId}` ||
            seen.has(instance.instanceId as string) ||
            instance.monitorId !== "" ||
            instance.projectionMonitorId !== null ||
            instance.projectionBounds !== null ||
            instance.stateVersion !== 1 ||
            ["left", "top", "width", "height"].some((key) => bounds[key] !== 0)
        )
            throw new Error("岛合成实例无效");
        if (
            !runtime.manifests.some((manifest) => manifest.id === instance.componentId && manifest.island) ||
            !runtime.instances.some((registration) => registration.instanceId === instance.instanceId && registration.componentId === instance.componentId)
        )
            throw new Error("岛实例清单或注册信息缺失");
        seen.add(instance.instanceId as string);
        return {
            instanceId: instance.instanceId as string,
            componentId,
            monitorId: "",
            projectionMonitorId: null,
            projectionBounds: null,
            bounds: { left: 0, top: 0, width: 0, height: 0 },
            stateVersion: 1,
            state: instance.state,
        };
    });
    if (runtime.instances.length !== instances.length) throw new Error("岛注册集合无效");
    return { runtime, instances };
};
