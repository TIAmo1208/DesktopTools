/** 后端枚举出的音频输出设备。 */
export interface AudioOutputDevice {
    id: string; // cpal 稳定设备标识。
    name: string; // 面向用户的设备名称。
    isDefault: boolean; // 是否为当前系统默认输出设备。
}

/** 一次采集连接的实际设备状态。 */
export interface AudioSpectrumStatus {
    running: boolean; // 采集线程是否在运行。
    deviceId: string; // 实际采集的设备标识。
    deviceName: string; // 实际采集的设备名称。
    fallback: boolean; // 保存的设备不可用时是否回退到系统默认。
}

/** 校验设备列表载荷。 */
export const parseAudioOutputDevices = (value: unknown): AudioOutputDevice[] => {
    if (!Array.isArray(value)) throw new Error("音频设备列表无效");
    return value.map((item) => {
        if (typeof item !== "object" || item === null) throw new Error("音频设备条目无效");
        const device = item as Record<string, unknown>; // 待校验的设备字段。
        if (typeof device.id !== "string" || typeof device.name !== "string" || typeof device.isDefault !== "boolean") throw new Error("音频设备条目无效");
        return { id: device.id, name: device.name, isDefault: device.isDefault };
    });
};

/** 校验采集状态载荷。 */
export const parseAudioSpectrumStatus = (value: unknown): AudioSpectrumStatus => {
    if (typeof value !== "object" || value === null) throw new Error("频谱状态无效");
    const status = value as Record<string, unknown>; // 待校验的状态字段。
    if (
        typeof status.running !== "boolean" ||
        typeof status.deviceId !== "string" ||
        typeof status.deviceName !== "string" ||
        typeof status.fallback !== "boolean"
    )
        throw new Error("频谱状态无效");
    return { running: status.running, deviceId: status.deviceId, deviceName: status.deviceName, fallback: status.fallback };
};

/** 校验单帧频谱载荷，长度由后端设置决定。 */
export const parseSpectrumBins = (value: unknown): number[] => {
    if (typeof value !== "object" || value === null || !("bins" in value)) throw new Error("频谱数据无效");
    const bins = (value as Record<string, unknown>).bins;
    if (!Array.isArray(bins) || bins.length === 0 || bins.length > 64) throw new Error("频谱数据无效");
    if (!bins.every((item) => typeof item === "number" && Number.isInteger(item) && item >= 0 && item <= 255)) throw new Error("频谱数据无效");
    return bins as number[];
};
