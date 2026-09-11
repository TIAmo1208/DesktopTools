import { useEffect, useRef, useState } from "react";
import { executeMusicAction, getCurrentMusicSession, listenMusicSession } from "../../bridge/music";
import { publishMessage } from "../notifications/messageNotification";
import { MusicControl, type MusicControlMinimumSize } from "./MusicControl";

/** 读取当前 GSMTC 会话并绑定三项媒体动作。 */
export const MusicWidget = ({ onMinimumSizeChange }: { onMinimumSizeChange: (minimum: MusicControlMinimumSize) => void }) => {
    const [snapshot, setSnapshot] = useState<import("../../contracts/music").MusicMediaSnapshot>(); // 当前媒体快照。
    const generation = useRef(0); // 丢弃早于事件或卸载的读取结果。
    const mounted = useRef(false); // 禁止卸载后的异步结果更新界面。
    const failed = useRef(false); // 当前连接故障周期是否已经通知。
    const refreshRetry = useRef<ReturnType<typeof setTimeout> | undefined>(undefined); // 故障后持续同步，即使恢复值未触发新事件。
    const handleFailure = (error: unknown) => {
        if (!mounted.current) return;
        generation.current++;
        setSnapshot(undefined);
        const message =
            error instanceof Error
                ? error.message
                : typeof error === "object" && error && "message" in error
                  ? String(error.message)
                  : typeof error === "string"
                    ? error
                    : "无法连接媒体控制"; // 兼容两个 transport 的错误载荷。
        if (!failed.current) publishMessage({ type: "warning", content: message, source: "desktop-music-control" });
        failed.current = true;
        clearTimeout(refreshRetry.current);
        refreshRetry.current = setTimeout(() => void refresh(), 1000);
    };
    const refresh = async () => {
        if (!mounted.current) return;
        clearTimeout(refreshRetry.current);
        const request = ++generation.current; // 此次读取对应的本地代次。
        try {
            const current = await getCurrentMusicSession(); // 读取后只接受仍有效的结果。
            if (!mounted.current || request !== generation.current) return;
            failed.current = false;
            setSnapshot(current);
        } catch (error) {
            if (request === generation.current) handleFailure(error);
        }
    };
    useEffect(() => {
        mounted.current = true;
        let active = true; // 隔离同一组件重新挂载前的订阅。
        let unlisten: (() => void) | undefined; // 已成功建立的订阅释放入口。
        let retry: ReturnType<typeof setTimeout> | undefined; // 订阅建立失败后的有界重试间隔。
        const connect = async () => {
            try {
                const stop = await listenMusicSession(
                    (current) => {
                        if (!active) return;
                        generation.current++;
                        clearTimeout(refreshRetry.current);
                        failed.current = false;
                        setSnapshot(current);
                    },
                    (error) => {
                        if (active) handleFailure(error);
                    },
                ); // 先订阅再读取，避免建立期间漏更新。
                if (!active) {
                    stop();
                    return;
                }
                unlisten = stop;
                void refresh();
            } catch (error) {
                if (!active) return;
                handleFailure(error);
                retry = setTimeout(() => void connect(), 1000);
            }
        };
        void connect();
        return () => {
            active = false;
            mounted.current = false;
            generation.current++;
            clearTimeout(retry);
            clearTimeout(refreshRetry.current);
            unlisten?.();
        };
    }, []);
    const execute = async (action: "previous" | "play" | "pause" | "next") => {
        await executeMusicAction(action, snapshot?.sessionId ?? "");
        await refresh();
    };
    return (
        <MusicControl
            onMinimumSizeChange={onMinimumSizeChange}
            snapshot={snapshot}
            actions={{
                previous: () => execute("previous"),
                playPause: () => execute(snapshot?.playback === "playing" ? "pause" : "play"),
                next: () => execute("next"),
            }}
            onFailure={(message) => {
                handleFailure(message);
                void refresh();
            }}
        />
    );
};
