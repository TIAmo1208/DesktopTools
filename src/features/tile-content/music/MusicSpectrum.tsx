import { useEffect, useRef } from "react";
import { listenSpectrum, startAudioSpectrum, stopAudioSpectrum } from "../../../bridge/audio";
import { effectiveBarCount, resampleSpectrum } from "./spectrumBars";
import "./MusicSpectrum.css";

/** 订阅后端回环频谱并在条带内绘制柱状图，暂停或静音时保留基线。 */
export const MusicSpectrum = ({ bars, fps, device, playing }: { bars: number; fps: number; device: string; playing: boolean }) => {
    const canvas = useRef<HTMLCanvasElement>(null); // 只承载绘制结果，不参与布局测量。
    const target = useRef<Float32Array>(new Float32Array(bars)); // 后端最新一帧的归一化幅度。
    const smooth = useRef<Float32Array>(new Float32Array(bars)); // 视觉平滑后的幅度。
    useEffect(() => {
        target.current = new Float32Array(bars);
        smooth.current = new Float32Array(bars);
    }, [bars]);
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
                        for (let index = 0; index < values.length; index++) values[index] = (bins[index] ?? 0) / 255;
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
        let frame = 0; // 当前动画帧句柄。
        const draw = () => {
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
            const values = smooth.current;
            const latest = target.current;
            for (let index = 0; index < values.length; index++) values[index] = Math.max(latest[index] ?? 0, values[index] * 0.82); // 上升即时、下降缓慢，避免逐帧抖动。
            const count = effectiveBarCount(values.length, rect.width); // 条带宽度不足时减少实绘段数。
            const drawn = resampleSpectrum(values, count); // 按峰值取每段最高电平。
            const step = Math.min(width / count, 12 * ratio); // 单段横向步进不超过 12px。
            const offset = (width - step * count) / 2; // 宽度富余时整组居中。
            const barWidth = Math.max(1, step * 0.72);
            const peak = drawn.reduce((max, value) => Math.max(max, value), 0); // 平滑后的最高电平，用于判定静音。
            if (!playing || peak < 0.002) {
                // 暂停、静音、无会话或采集失败都画同色 30% 的 1px 基线，条带不隐藏、不冻结末帧。
                context.globalAlpha = 0.3;
                context.fillRect(offset, height - Math.max(1, ratio), step * count, Math.max(1, ratio));
            } else {
                context.globalAlpha = 0.75;
                for (let index = 0; index < count; index++) {
                    const barHeight = drawn[index] * height;
                    if (barHeight < 0.5) continue;
                    context.fillRect(offset + index * step + (step - barWidth) / 2, height - barHeight, barWidth, barHeight); // 底部对齐。
                }
            }
            frame = requestAnimationFrame(draw);
        };
        frame = requestAnimationFrame(draw);
        return () => cancelAnimationFrame(frame);
    }, [playing]);
    return <canvas ref={canvas} className="music-spectrum" aria-hidden="true" />;
};
