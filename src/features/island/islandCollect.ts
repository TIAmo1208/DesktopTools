import type { CollectItem } from "../../bridge/collect.ts";
import { collectExcerpt } from "../../shared/collectExcerpt.ts";
import { orderCollectViewItems } from "../../shared/collectView.ts";

/** 浮岛收集箱只显示磁盘事实；排序与桌面格子共用置顶与时间规则。 */
export const islandCollectItems = (items: readonly CollectItem[], limit: number) => orderCollectViewItems(items, "mixed", "time").slice(0, limit);

/** 条目标题与桌面收集箱一致：正文首个标题优先，其次去扩展名的文件名。 */
export const islandCollectTitle = (item: CollectItem) => item.content.match(/^#{1,6}\s+(.+)$/m)?.[1]?.trim() || item.name.replace(/\.md$/i, "");

/** 摘要只取正文纯文本；图片与文件映射使用显示名，不伪造正文。 */
export const islandCollectSummary = (item: CollectItem) => (item.kind === "text" ? collectExcerpt(item.content).slice(0, 120) : item.displayName);

/** 浮岛页面启动时预读的收集项：展开首帧就有真实条目，不再先按加载态测一次高度。 */
let islandCollectSnapshot: readonly CollectItem[] | undefined;

/** 读取预读快照；尚未读到或读取失败时返回 undefined，由卡片自己再取一次。 */
export const readIslandCollectCache = () => islandCollectSnapshot;

/** 写入最近一次成功读取的收集项；条目变化仍由收集事件触发重读。 */
export const writeIslandCollectCache = (items: readonly CollectItem[]) => { islandCollectSnapshot = items; };

/** 浮岛三态各自展示的条目数：胶囊一条、hover 两条、展开有上限。 */
export const islandCollectLimit = (mode: "collapsed" | "hover" | "expanded") => (mode === "collapsed" ? 1 : mode === "hover" ? 2 : 50);
