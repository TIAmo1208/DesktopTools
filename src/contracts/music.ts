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

/** 时间轴原始 100ns ticks 保留字符串；LastUpdatedTime 为 1601 纪元。 */
export interface MusicTimeline {
    sessionId: string;
    title: string;
    artist: string;
    positionTicks: string;
    endTimeTicks: string;
    lastUpdatedTimeTicks: string;
    playbackStatus: number;
    positionMs: number;
    durationMs: number;
    advancing: boolean;
}
/** 已按起始毫秒排序、合并同时间戳的逐句歌词；翻译行本轮可选。 */
export interface LyricLine {
    startMs: number;
    text: string;
    translation?: string;
}
export interface MusicDetails {
    timeline?: MusicTimeline;
    lyrics: LyricLine[];
}

/** 第二事实边界拒绝错误类型和非有限位置，不推断缺失字段。 */
export const parseMusicDetails = (value: unknown): MusicDetails => {
    if (!value || typeof value !== "object" || !("timeline" in value) || !("lyrics" in value) || !Array.isArray(value.lyrics))
        throw new Error("音乐时间轴无效");
    let timeline: MusicTimeline | undefined;
    if (value.timeline !== null) {
        if (!value.timeline || typeof value.timeline !== "object") throw new Error("音乐时间轴无效");
        const raw = value.timeline as Record<string, unknown>;
        for (const key of ["sessionId", "title", "artist"]) if (typeof raw[key] !== "string") throw new Error("时间轴身份无效");
        if (!raw.sessionId) throw new Error("时间轴身份无效");
        for (const key of ["positionTicks", "endTimeTicks", "lastUpdatedTimeTicks"])
            if (typeof raw[key] !== "string" || !/^-?\d+$/.test(raw[key])) throw new Error("时间轴原始值无效");
        for (const key of ["positionMs", "durationMs", "playbackStatus"])
            if (typeof raw[key] !== "number" || !Number.isFinite(raw[key])) throw new Error("时间轴数值无效");
        if (
            !Number.isInteger(raw.playbackStatus) ||
            Number(raw.positionMs) < 0 ||
            Number(raw.durationMs) < Number(raw.positionMs) ||
            typeof raw.advancing !== "boolean"
        )
            throw new Error("时间轴投影无效");
        timeline = raw as unknown as MusicTimeline;
    }
    let previous = -1;
    const lyrics = value.lyrics.map((line: unknown): LyricLine => {
        if (
            !line ||
            typeof line !== "object" ||
            !("startMs" in line) ||
            !("text" in line) ||
            typeof line.startMs !== "number" ||
            !Number.isSafeInteger(line.startMs) ||
            line.startMs < 0 ||
            line.startMs <= previous ||
            typeof line.text !== "string"
        )
            throw new Error("歌词行无效");
        previous = line.startMs;
        const translation = "translation" in line ? line.translation : undefined; // 翻译字段只在提供时渲染。
        if (translation !== undefined && typeof translation !== "string") throw new Error("歌词翻译无效");
        return { startMs: line.startMs, text: line.text, ...(translation === undefined ? {} : { translation }) };
    });
    return { timeline, lyrics };
};

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
