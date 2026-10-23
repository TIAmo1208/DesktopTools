import type { UnlistenFn } from "@tauri-apps/api/event";
import { invokeRuntime, listenRuntime } from "./runtime";
import { parseAudioOutputDevices, parseAudioSpectrumStatus, parseSpectrumBins, type AudioOutputDevice, type AudioSpectrumStatus } from "../contracts/audio";

/** 读取后端可选的输出设备。 */
export const listAudioOutputDevices = async (): Promise<AudioOutputDevice[]> =>
    parseAudioOutputDevices(await invokeRuntime<unknown>("list_audio_output_devices"));

/** 启动系统音频回环采集，空设备标识表示跟随系统默认。 */
export const startAudioSpectrum = async (device: string, bars: number, fps: number): Promise<AudioSpectrumStatus> =>
    parseAudioSpectrumStatus(await invokeRuntime<unknown>("start_audio_spectrum", { device, bars, fps }));

/** 停止采集，重复调用无副作用。 */
export const stopAudioSpectrum = () => invokeRuntime<void>("stop_audio_spectrum");

/** 订阅后端按帧率发布的频谱数据。 */
export const listenSpectrum = (onBins: (bins: number[]) => void, onError: (error: unknown) => void): Promise<UnlistenFn> =>
    listenRuntime<unknown>("audio-spectrum-changed", (payload) => {
        try {
            onBins(parseSpectrumBins(payload));
        } catch (error) {
            onError(error);
        }
    });

/** 订阅实际采集设备变化，用于设置页回显回退状态。 */
export const listenSpectrumDevice = (onStatus: (status: AudioSpectrumStatus) => void, onError: (error: unknown) => void): Promise<UnlistenFn> =>
    listenRuntime<unknown>("audio-spectrum-device-changed", (payload) => {
        try {
            onStatus(parseAudioSpectrumStatus(payload));
        } catch (error) {
            onError(error);
        }
    });

/** 订阅系统输出设备列表变化，仅在列表真正改变时投递。 */
export const listenAudioDeviceChanges = (onDevices: (devices: AudioOutputDevice[]) => void, onError: (error: unknown) => void): Promise<UnlistenFn> =>
    listenRuntime<unknown>("audio-devices-changed", (payload) => {
        try {
            onDevices(parseAudioOutputDevices(payload));
        } catch (error) {
            onError(error);
        }
    });

/** 开关后端设备通知监听，面板关闭后释放监听线程。 */
export const setAudioDeviceWatch = (active: boolean) => invokeRuntime<void>("set_audio_device_watch", { active });
