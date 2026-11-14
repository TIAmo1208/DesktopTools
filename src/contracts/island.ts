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

/** 浮岛消息：与原生 Toast 同一队列的只读投影，按到达顺序下发。 */
export interface IslandMessage {
    id: number; // 队列内稳定身份，关闭同一条消息时回传。
    type: "info" | "success" | "warning" | "error";
    content: string; // 完整正文，三态下的截断由页面决定。
    source: string; // 来源标识。
    loading: boolean;
    receivedAtMs: number; // 到达时刻的墙钟毫秒。
}

const ISLAND_MESSAGE_TYPES: readonly IslandMessage["type"][] = ["info", "success", "warning", "error"];

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

/** 消息集合严格校验：未知字段、重复身份、空正文与非安全整数都拒绝，坏数据不进入渲染。 */
export const parseIslandMessages = (value: unknown): IslandMessage[] => {
    if (!Array.isArray(value) || value.length > 32) throw new Error("岛消息集合无效");
    const seen = new Set<number>(); // 同一集合内身份必须唯一。
    return value.map((entry): IslandMessage => {
        const item = record(entry); // 单条消息投影。
        const keys = ["id", "type", "content", "source", "loading", "receivedAtMs"];
        if (Object.keys(item).length !== keys.length || keys.some((key) => !Object.hasOwn(item, key))) throw new Error("岛消息字段无效");
        const id = item.id;
        if (typeof id !== "number" || !Number.isSafeInteger(id) || id <= 0 || seen.has(id)) throw new Error("岛消息身份无效");
        if (!ISLAND_MESSAGE_TYPES.includes(item.type as IslandMessage["type"])) throw new Error("岛消息类型无效");
        if (typeof item.content !== "string" || !item.content.trim() || typeof item.source !== "string" || !item.source.trim()) throw new Error("岛消息正文或来源无效");
        if (typeof item.loading !== "boolean") throw new Error("岛消息加载状态无效");
        if (typeof item.receivedAtMs !== "number" || !Number.isSafeInteger(item.receivedAtMs) || item.receivedAtMs < 0) throw new Error("岛消息时间无效");
        seen.add(id);
        return { id, type: item.type as IslandMessage["type"], content: item.content, source: item.source, loading: item.loading, receivedAtMs: item.receivedAtMs };
    });
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
