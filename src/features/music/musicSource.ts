import { useSyncExternalStore } from "react";
import { listenMusicFacts } from "../../bridge/music";
import { createMusicSource } from "./musicState";

/** 同一 WebView 内格子与岛呈现消费者共用唯一底层订阅组和缓存。 */
const source = createMusicSource(listenMusicFacts, () => performance.now());
export const subscribeMusic = source.subscribe;
export const getMusicState = source.getSnapshot;
export const useMusicState = () => useSyncExternalStore(subscribeMusic, getMusicState);
export { positionAt, lyricIndex } from "./musicState";
