/** 根据内容宽度和字号选择布局，16px 滞回避免临界缩放抖动。 */
export const isCompactMusicLayout = (width: number, fontSize: number, current?: boolean): boolean => {
    const lower = Math.ceil(((144 * fontSize) / 13 + 12) / 0.7) + 5; // 左侧封面最多占 30%，右侧保证基本可读宽度。
    if (current === undefined) return width < lower + 8;
    return current ? width < lower + 16 : width <= lower;
};

/** 正文区最小高度：信息、按钮、内边距与频谱条带外部尺寸；宽布局频谱上下各 6px，紧凑绝对定位只留 6px。 */
export const musicMediaMinimum = (
    informationHeight: number,
    buttonsHeight: number,
    paddingTop: number,
    paddingBottom: number,
    spectrumEnabled: boolean,
    compact: boolean,
): number => {
    const band = spectrumEnabled ? (compact ? 24 : 30) : 0;
    const floor = spectrumEnabled ? 112 : 94;
    return Math.ceil(Math.max(floor, informationHeight + buttonsHeight + paddingTop + paddingBottom + band));
};
