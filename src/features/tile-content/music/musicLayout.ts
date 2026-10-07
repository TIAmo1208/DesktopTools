/** 根据内容宽度和字号选择布局，16px 滞回避免临界缩放抖动。 */
export const isCompactMusicLayout = (width: number, fontSize: number, current?: boolean): boolean => {
    const lower = Math.ceil(((144 * fontSize) / 13 + 12) / 0.7) + 5; // 左侧封面最多占 30%，右侧保证基本可读宽度。
    if (current === undefined) return width < lower + 8;
    return current ? width < lower + 16 : width <= lower;
};
