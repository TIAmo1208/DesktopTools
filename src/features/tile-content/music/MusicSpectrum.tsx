import { useEffect, useRef } from "react";
import { listenSpectrum, startAudioSpectrum, stopAudioSpectrum } from "../../../bridge/audio";
import "./MusicSpectrum.css";

/** 订阅后端回环频谱并在封面区域内绘制柱状图。 */
export const MusicSpectrum = ({ bars, fps, device }: { bars: number; fps: number; device: string }) => {
    const canvas = useRef<HTMLCanvasElement>(null); // 只承载绘制结果，不参与布局测量。
    const target = useRef<Float32Array>(new Float32Array(bars)); // 后端最新一帧的归一化幅度。
    const smooth = useRef<Float32Array>(new Float32Array(bars)); // 视觉平滑后的幅度。
    useEffect(() => {
        target.current = new Float32Array(bars);
        smooth.current = new Float32Array(bars);
    }, [bars]);
    useEffect(() => {
        let active = true; // 卸载后拒绝迟到的订阅与启动结果。
        let unlisten: (() => void) | undefined; // 已建立的频谱订阅。
        const connect = async () => {
            try {
                const stop = await listenSpectrum(
                    (bins) => {
                        if (!active) return;
                        const values = target.current;
                        for (let index = 0; index < values.length; index++) values[index] = (bins[index] ?? 0) / 255;
                    },
                    () => {},
                );
                if (!active) {
                    stop();
                    return;
                }
                unlisten = stop;
                await startAudioSpectrum(device, bars, fps);
            } catch {
                // 采集失败时保持静默，控件其余部分仍可用。
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
            context.fillStyle = getComputedStyle(element).color; // 沿用控件当前文字颜色。
            context.globalAlpha = 0.75;
            const values = smooth.current;
            const latest = target.current;
            const slot = width / values.length; // 每段的完整横向步进。
            const barWidth = Math.max(1, slot * 0.72);
            for (let index = 0; index < values.length; index++) {
                values[index] = Math.max(latest[index], values[index] * 0.82); // 上升即时、下降缓慢，避免逐帧抖动。
                const barHeight = values[index] * height;
                if (barHeight < 0.5) continue;
                context.fillRect(index * slot + (slot - barWidth) / 2, height - barHeight, barWidth, barHeight);
            }
            frame = requestAnimationFrame(draw);
        };
        frame = requestAnimationFrame(draw);
        return () => cancelAnimationFrame(frame);
    }, []);
    return <canvas ref={canvas} className="music-spectrum" aria-hidden="true" />;
};
