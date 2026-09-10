/** GSMTC 提供的媒体事实，缺失封面允许为空。 */
export interface MusicMediaSnapshot {
    sessionId: string; // 后端保留的会话实例代次。
    title: string; // 媒体标题。
    artist: string; // 艺人文本。
    source: string; // 系统来源身份。
    artworkUrl?: string; // 经过类型检查的数据 URL。
    playback: "playing" | "paused" | "stopped" | "unknown"; // 播放事实。
    canPrevious: boolean; // 上一曲能力。
    canPlayPause: boolean; // 当前播放动作能力。
    canNext: boolean; // 下一曲能力。
}

/** 校验真实 IPC 数据，不能以类型断言代替边界验证。 */
export const parseMusicSnapshot = (value: unknown): MusicMediaSnapshot | undefined => {
    if (!value || typeof value !== "object" || !("media" in value)) throw new Error("媒体快照无效");
    if ("error" in value && typeof value.error === "string") throw new Error(value.error);
    if (value.media === null) return undefined;
    if (!value.media || typeof value.media !== "object") throw new Error("媒体会话无效");
    const media = value.media as Record<string, unknown>; // 待验证的 IPC 媒体字段。
    for (const key of ["title", "artist", "source", "sessionId"]) if (typeof media[key] !== "string") throw new Error("媒体信息无效");
    if (!media.sessionId) throw new Error("媒体会话身份无效");
    for (const key of ["canPrevious", "canPlayPause", "canNext"]) if (typeof media[key] !== "boolean") throw new Error("媒体能力无效");
    if (!["playing", "paused", "stopped", "unknown"].includes(String(media.playback))) throw new Error("媒体播放状态无效");
    if (
        media.artworkUrl != null &&
        (typeof media.artworkUrl !== "string" || !/^data:image\/(jpeg|png|gif|webp|bmp);base64,[A-Za-z0-9+/]*={0,2}$/.test(media.artworkUrl))
    )
        throw new Error("媒体封面无效");
    return {
        sessionId: media.sessionId as string,
        title: media.title as string,
        artist: media.artist as string,
        source: media.source as string,
        artworkUrl: media.artworkUrl == null ? undefined : (media.artworkUrl as string),
        playback: media.playback as MusicMediaSnapshot["playback"],
        canPrevious: media.canPrevious as boolean,
        canPlayPause: media.canPlayPause as boolean,
        canNext: media.canNext as boolean,
    };
};
