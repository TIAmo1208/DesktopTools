/** 频谱条带按可用宽度裁剪段数并按峰值重采样，绘制与测试共用。 */

/** 按条带宽度限制实绘段数，每段至少保留 3px。 */
export const effectiveBarCount = (configuredBars: number, width: number): number => Math.max(1, Math.min(configuredBars, Math.floor(width / 3)));

/** 将设置段数的幅度按峰值重采样为实绘段数；全 0 输入保持全 0。 */
export const resampleSpectrum = (values: ArrayLike<number>, count: number): Float32Array => {
    const total = Math.max(0, Math.floor(count)); // 输出段数，避免非法长度分配。
    const result = new Float32Array(total);
    if (!values.length) return result;
    for (let index = 0; index < total; index++) {
        const start = Math.floor((index * values.length) / total);
        const end = Math.max(start + 1, Math.floor(((index + 1) * values.length) / total));
        let peak = 0; // 该段覆盖的源区间最高电平。
        for (let source = start; source < end && source < values.length; source++) peak = Math.max(peak, values[source]);
        result[index] = peak;
    }
    return result;
};
