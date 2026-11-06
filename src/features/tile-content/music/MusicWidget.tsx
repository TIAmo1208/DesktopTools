import { useEffect, useRef } from "react";
import { executeMusicAction } from "../../../bridge/music";
import { useMusicState } from "../../music/musicSource";
import { publishMessage } from "../../notifications/messageNotification";
import { MusicControl, type MusicControlMinimumSize } from "./MusicControl";

/** 桌面格子仅消费共享事实，保留既有视觉与动作。 */
export const MusicWidget = ({ onMinimumSizeChange }: { onMinimumSizeChange: (minimum: MusicControlMinimumSize) => void }) => {
    const { snapshot, error } = useMusicState(); // 与岛卡使用同一状态源。
    const failed = useRef(false); // 每个连接故障周期只通知一次。
    const handleFailure = (error: unknown) => {
        const message = error instanceof Error ? error.message : typeof error === "string" ? error : "无法连接媒体控制";
        if (!failed.current) publishMessage({ type: "warning", content: message, source: "desktop-music-control" });
        failed.current = true;
    };
    useEffect(() => {
        if (error) handleFailure(error);
        else failed.current = false;
    }, [error]);
    const execute = (action: "previous" | "play" | "pause" | "next") => executeMusicAction(action, snapshot?.sessionId ?? "");
    return (
        <MusicControl
            onMinimumSizeChange={onMinimumSizeChange}
            snapshot={snapshot}
            actions={{
                previous: () => execute("previous"),
                playPause: () => execute(snapshot?.playback === "playing" ? "pause" : "play"),
                next: () => execute("next"),
            }}
            onFailure={handleFailure}
        />
    );
};
