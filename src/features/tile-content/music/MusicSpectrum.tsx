import { useEffect, useRef } from "react";
import { listenSpectrum, startAudioSpectrum, stopAudioSpectrum } from "../../../bridge/audio";
import { effectiveBarCount, resampleSpectrum } from "./spectrumBars";
import "./MusicSpectrum.css";

/** 播放中帧内容持续不变的判定阈值，超过即视为回环流停摆。 */
const STALL_CONTENT_MS = 2000;
/** 停摆后重建采集的退避下限与上限，避免恒定音频反复重建。 */
const RESTART_DELAY_MS = 2000;
const RESTART_DELAY_MAX_MS = 30000;
/** 低于该电平视为静音：只画基线且不占用帧回调。 */
const SILENT_LEVEL = 0.002;
/** 超过该时间没有新帧视为上游暂停投递：保留最后一帧并停止重绘。 */
const FRAME_GAP_MS = 1000;

/** 订阅后端回环频谱并在条带内绘制柱状图，只在有可见变化时占用帧回调。 */
export const MusicSpectrum = ({ bars, fps, device, playing }: { bars: number; fps: number; device: string; playing: boolean }) => {
    const canvas = useRef<HTMLCanvasElement>(null); // 只承载绘制结果，不参与布局测量。
    const target = useRef<Float32Array>(new Float32Array(bars)); // 后端最新一帧的归一化幅度。
    const smooth = useRef<Float32Array>(new Float32Array(bars)); // 视觉平滑后的幅度。
    const handle = useRef(0); // 当前动画帧句柄，0 表示重绘已暂停。
    const tick = useRef<() => void>(() => {}); // 绘制入口，供订阅唤醒已暂停的循环。
    const restart = useRef<() => void>(() => {}); // 采集重建入口，供绘制循环调用。
    const lastArrival = useRef(0); // 最近一帧到达时间。
    const lastSignature = useRef(-1); // 最近一帧的内容签名，用于识别恒定帧。
    const steadySince = useRef(0); // 帧内容开始恒定的时间。
    const nextRestartAt = useRef(0); // 下一次允许重建采集的时间。
    const restartDelay = useRef(RESTART_DELAY_MS); // 当前重建退避。
    const playingRef = useRef(playing); // 供订阅回调判断是否唤醒绘制。
    useEffect(() => {
        playingRef.current = playing;
    }, [playing]);
    useEffect(() => {
        target.current = new Float32Array(bars);
        smooth.current = new Float32Array(bars);
        lastSignature.current = -1;
    }, [bars]);
    useEffect(() => {
        // 停摆时复用既有启动命令重建采集：start 内部先停止旧线程，无需新增命令。
        restart.current = () => {
            const now = performance.now();
            if (now < nextRestartAt.current) return;
            nextRestartAt.current = now + restartDelay.current;
            restartDelay.current = Math.min(restartDelay.current * 2, RESTART_DELAY_MAX_MS);
            void startAudioSpectrum(device, bars, fps).catch(() => {});
        };
        return () => {
            restart.current = () => {}; // 卸载或参数变化后禁止迟到的重建。
        };
    }, [device, bars, fps]);
    useEffect(() => {
        // 订阅与启停只随段数、帧率和设备重建，布局切换不重建。
        let active = true; // 卸载后拒绝迟到的订阅与启动结果。
        let unlisten: (() => void) | undefined; // 已建立的频谱订阅。
        const clearLevels = () => {
            target.current.fill(0);
            smooth.current.fill(0);
        }; // 采集失败时清空缓冲，绘制当帧即进入基线分支，不残留旧柱形。
        const connect = async () => {
            try {
                const stop = await listenSpectrum(
                    (bins) => {
                        if (!active) return;
                        const values = target.current;
                        let peak = 0; // 本帧最高电平，用于判断是否需要唤醒重绘。
                        for (let index = 0; index < values.length; index++) {
                            values[index] = (bins[index] ?? 0) / 255;
                            if (values[index] > peak) peak = values[index];
                        }
                        lastArrival.current = performance.now();
                        // 静音帧不唤醒重绘；有信号时恢复已暂停的绘制循环。
                        if (!handle.current && playingRef.current && peak >= SILENT_LEVEL) handle.current = requestAnimationFrame(tick.current);
                    },
                    () => clearLevels(),
                );
                if (!active) {
                    stop();
                    return;
                }
                unlisten = stop;
                await startAudioSpectrum(device, bars, fps);
            } catch {
                // 采集失败时清空缓冲并保持静默，控件其余部分仍可用。
                clearLevels();
            }
        };
        void connect();
        return () => {
            active = false;
            unlisten?.();
            void stopAudioSpectrum();
        };
    }, [bars, fps, device]);
    useEffect(() => {
        const element = canvas.current;
        const context = element?.getContext("2d");
        if (!element || !context) return;
        const draw = () => {
            handle.current = 0; // 本帧结束前保持暂停状态，订阅可直接唤醒。
            const now = performance.now();
            const latest = target.current;
            const values = smooth.current;
            // 上游暂停投递时保留最后一帧，不再逐帧重绘，等新帧唤醒。
            if (playing && now - lastArrival.current > FRAME_GAP_MS) return;
            const rect = element.getBoundingClientRect();
            const ratio = window.devicePixelRatio || 1;
            const width = Math.max(1, Math.round(rect.width * ratio)); // 绘制分辨率跟随屏幕缩放。
            const height = Math.max(1, Math.round(rect.height * ratio));
            if (element.width !== width || element.height !== height) {
                element.width = width;
                element.height = height;
            }
            context.clearRect(0, 0, width, height);
            context.fillStyle = getComputedStyle(element).color; // 沿用条带文字颜色，不跟随封面白色文字。
            for (let index = 0; index < values.length; index++) values[index] = Math.max(latest[index] ?? 0, values[index] * 0.82); // 上升即时、下降缓慢，避免逐帧抖动。
            // 播放中帧内容长期不变说明回环流已停摆：退回基线并重建采集，避免画面停在假电平。
            let signature = 0;
            for (let index = 0; index < latest.length; index++) signature += latest[index] ?? 0;
            if (signature !== lastSignature.current) {
                lastSignature.current = signature;
                steadySince.current = now;
                restartDelay.current = RESTART_DELAY_MS;
            } else if (now - steadySince.current > STALL_CONTENT_MS) {
                latest.fill(0);
                values.fill(0);
                steadySince.current = now;
                restart.current();
            }
            const count = effectiveBarCount(values.length, rect.width); // 条带宽度不足时减少实绘段数。
            const drawn = resampleSpectrum(values, count); // 按峰值取每段最高电平。
            const step = Math.min(width / count, 12 * ratio); // 单段横向步进不超过 12px。
            const offset = (width - step * count) / 2; // 宽度富余时整组居中。
            const barWidth = Math.max(1, step * 0.72);
            const peak = drawn.reduce((max, value) => Math.max(max, value), 0); // 平滑后的最高电平，用于判定静音。
            if (!playing || peak < SILENT_LEVEL) {
                // 暂停、静音、无会话或采集失败都画同色 30% 的 1px 基线，条带不隐藏、不冻结末帧。
                context.globalAlpha = 0.3;
                context.fillRect(offset, height - Math.max(1, ratio), step * count, Math.max(1, ratio));
                return; // 无可见变化时不排下一帧，避免空转占用绘制与合成资源。
            }
            context.globalAlpha = 0.75;
            for (let index = 0; index < count; index++) {
                const barHeight = drawn[index] * height;
                if (barHeight < 0.5) continue;
                context.fillRect(offset + index * step + (step - barWidth) / 2, height - barHeight, barWidth, barHeight); // 底部对齐。
            }
            handle.current = requestAnimationFrame(draw);
        };
        tick.current = draw;
        lastArrival.current = performance.now(); // 首帧到达前按静音处理，先画出基线。
        handle.current = requestAnimationFrame(draw);
        return () => {
            cancelAnimationFrame(handle.current);
            handle.current = 0;
        };
    }, [playing]);
    return <canvas ref={canvas} className="music-spectrum" aria-hidden="true" />;
};
