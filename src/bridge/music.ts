import type { UnlistenFn } from "@tauri-apps/api/event";
import { invokeRuntime, listenRuntime } from "./runtime";
import { parseMusicSnapshot, type MusicMediaSnapshot } from "../contracts/music";

/** 读取 Rust GSMTC 当前会话快照。 */
export const getCurrentMusicSession = async (): Promise<MusicMediaSnapshot | undefined> => {
    return parseMusicSnapshot(await invokeRuntime<unknown>("get_current_music_session"));
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
