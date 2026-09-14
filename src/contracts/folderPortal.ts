/** 文件夹门户中一个真实目录项目。 */
export interface FolderPortalItem {
    name: string;
    isDirectory: boolean;
    size: number | null;
    modifiedUnix: number | null;
}

/** 文件夹门户目录快照。 */
export interface FolderPortalSnapshot {
    rootPath: string;
    currentPath: string;
    accessible: boolean;
    items: FolderPortalItem[];
    error: string | null;
}

/** 校验目录快照的边界数据。 */
export const parseFolderPortalSnapshot = (value: unknown): FolderPortalSnapshot => {
    if (typeof value !== "object" || value === null) throw new Error("文件夹门户快照无效");
    const candidate = value as Record<string, unknown>;
    if (
        typeof candidate.rootPath !== "string" ||
        typeof candidate.currentPath !== "string" ||
        typeof candidate.accessible !== "boolean" ||
        !Array.isArray(candidate.items)
    )
        throw new Error("文件夹门户快照无效");
    const items = candidate.items.map((item) => {
        if (typeof item !== "object" || item === null) throw new Error("文件夹门户项目无效");
        const entry = item as Record<string, unknown>;
        if (typeof entry.name !== "string" || typeof entry.isDirectory !== "boolean") throw new Error("文件夹门户项目无效");
        if (entry.size !== null && typeof entry.size !== "number") throw new Error("文件夹门户项目大小无效");
        if (entry.modifiedUnix !== null && typeof entry.modifiedUnix !== "number") throw new Error("文件夹门户项目时间无效");
        return { name: entry.name, isDirectory: entry.isDirectory, size: entry.size as number | null, modifiedUnix: entry.modifiedUnix as number | null };
    });
    if (candidate.error !== null && typeof candidate.error !== "string") throw new Error("文件夹门户错误无效");
    return {
        rootPath: candidate.rootPath,
        currentPath: candidate.currentPath,
        accessible: candidate.accessible,
        items,
        error: candidate.error as string | null,
    };
};
