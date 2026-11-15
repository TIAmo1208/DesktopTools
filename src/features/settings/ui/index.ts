// 设置基元的公开入口：其他设置子域只经本文件引用 ui，ui 自身不依赖任何子域。
// 公开清单：
// - SettingsSwitch：受控开关。
// - SettingsInfo：设置行说明。
// - SettingsAbout：可折叠说明正文。
// - MusicSettingsRows：音乐相关设置行。
export { MusicLyricsRow, MusicSpectrumBarsRow, MusicSpectrumFpsRow, MusicSpectrumRow } from "./MusicSettingsRows";
export { SettingsAbout } from "./SettingsAbout";
export { SettingsInfo } from "./SettingsInfo";
export { SettingsSwitch } from "./SettingsSwitch";
