use tauri::{AppHandle, command};

/// 列出可选择的音频输出设备。
#[command]
pub fn list_audio_output_devices() -> Vec<crate::platform::windows::audio::OutputDevice> { crate::platform::windows::audio::output_devices() }

/// 启动系统音频频谱采集。
#[command]
pub fn start_audio_spectrum(app: AppHandle, device: String, bars: u8, fps: u8) -> Result<crate::platform::windows::audio::SpectrumStatus, String> { crate::platform::windows::audio::start(app, device, bars, fps) }

/// 停止系统音频频谱采集。
#[command]
pub fn stop_audio_spectrum() { crate::platform::windows::audio::stop() }

/// 读取采集线程是否仍在运行。
#[command]
pub fn get_audio_spectrum_status() -> bool { crate::platform::windows::audio::running() }
