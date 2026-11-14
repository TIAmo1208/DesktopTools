import type { UnlistenFn } from "@tauri-apps/api/event";
import { invokeRuntime, listenRuntime } from "./runtime";
import { parseMusicDetails, parseMusicSnapshot, type MusicDetails, type MusicMediaSnapshot } from "../contracts/music";

/** 读取 Rust GSMTC 当前会话快照。 */
export const getCurrentMusicSession = async (): Promise<MusicMediaSnapshot | undefined> => {
    return parseMusicSnapshot(await invokeRuntime<unknown>("get_current_music_session"));
};

/** 只读第二事实缓存，网络取词始终由 Rust 负责。 */
export const getMusicDetails = async () => parseMusicDetails(await invokeRuntime<unknown>("get_music_details"));

/** 每个共享源只建立这一组订阅；每类事实各自保护初读竞态。 */
export const listenMusicFacts = async (
    onSnapshot: (value: MusicMediaSnapshot | undefined) => void,
    onDetails: (value: MusicDetails) => void,
    onError: (error: unknown) => void,
): Promise<UnlistenFn> => {
    let active = true;
    const stops: UnlistenFn[] = [];
    const stop = () => {
        active = false;
        for (const dispose of stops) dispose();
    };
    const connect = async <T>(event: string, command: string, parse: (value: unknown) => T, accept: (value: T) => void, invalidate: () => void) => {
        let revision = 0;
        const deliver = (value: unknown) => {
            if (!active) return;
            revision++;
            try {
                accept(parse(value));
            } catch (error) {
                invalidate();
                onError(error);
            }
        };
        const dispose = await listenRuntime<unknown>(event, deliver);
        if (!active) {
            dispose();
            return;
        }
        stops.push(dispose);
        // 原生桥订阅自带初读；Tauri 页面才需要显式初读。
        if (!globalThis.window?.__DESKTOP_TOOLS_NATIVE__) {
            const before = revision;
            const value = await invokeRuntime<unknown>(command);
            if (before === revision) deliver(value);
        }
    };
    try {
        await connect("music-session-changed", "get_current_music_session", parseMusicSnapshot, onSnapshot, () => onSnapshot(undefined));
        await connect("music-timeline-changed", "get_music_details", parseMusicDetails, onDetails, () => onDetails({ timeline: undefined, lyrics: [] }));
        return stop;
    } catch (error) {
        stop();
        throw error;
    }
};

/** 执行上一曲、播放/暂停或下一曲。 */
export const executeMusicAction = (action: "previous" | "play" | "pause" | "next", expectedSession: string) =>
    invokeRuntime<void>("execute_music_action", { action, expectedSession });

/** 订阅后端仅在媒体快照变化时发布的事件。 */
export const listenMusicSession = (onSnapshot: (snapshot: MusicMediaSnapshot | undefined) => void, onError: (error: unknown) => void): Promise<UnlistenFn> =>
    listenRuntime<unknown>("music-session-changed", (payload) => {
        try {
            onSnapshot(parseMusicSnapshot(payload));
        } catch (error) {
            onError(error);
        }
    });
