import type { ApplicationPreferences } from "../../contracts/settings";
import { SettingsInfo } from "./SettingsInfo";
import { SettingsSwitch } from "./SettingsSwitch";

/** 频谱段数档位：与采集端点的固定档位一致。 */
export const MUSIC_SPECTRUM_BARS = [16, 24, 32, 48, 64] as const;

/** 频谱刷新帧率档位。 */
export const MUSIC_SPECTRUM_FPS = [15, 30, 60] as const;

/**
 * 歌词开关的说明：把两个播放器前提写在这里，桌面子格子与浮岛卡片读到同一份文案。
 * 具体播放器只在说明里出现，不写进卡片自身描述。
 */
export const MUSIC_LYRICS_INFO =
    "歌词来自公开歌词源。播放软件需要在自身设置里开启系统媒体控制（SMTC），否则读不到媒体会话；网易云要拿到准确播放进度，需以 --remote-debugging-port=9222 启动客户端。";

/** 音乐设置行的排版类：桌面功能设置与浮岛卡片二级页各自沿用所在页面的行样式。 */
type MusicRowClassName = "feature-settings__row" | "settings__row";

/** 音乐设置行共用的偏好来源。 */
interface MusicRowProps {
    className: MusicRowClassName; // 所在页面的行样式。
    preferences?: ApplicationPreferences; // 当前偏好；缺失时控件按默认值显示。
    disabled: boolean; // 设置不可编辑时禁用控件。
    onPreferences: (patch: Partial<ApplicationPreferences>) => void; // 提交偏好补丁。
}

/** 选择项的行属性：换页时选择控件沿用该页面的选择样式。 */
interface MusicSelectRowProps extends MusicRowProps {
    selectClassName?: string; // 所在页面的选择控件样式；缺省时由页面自身的 select 规则决定。
}

/** 行标题样式：浮岛页面用共享标题列把控件推到右端，功能格子页面沿用原有纯文本标签。 */
const titleClassName = (className: MusicRowClassName, inline: boolean): string | undefined =>
    className === "settings__row" ? (inline ? "settings__row-title settings__row-title--inline" : "settings__row-title") : undefined;

/** 显示歌词：桌面格子与浮岛卡片共用同一开关。 */
export const MusicLyricsRow = ({ className, preferences, disabled, onPreferences }: MusicRowProps) => (
    <div className={className}>
        <span className={titleClassName(className, true)}>
            <label htmlFor="music-lyrics-enabled">显示歌词</label>
            <SettingsInfo label="显示歌词" description={MUSIC_LYRICS_INFO} />
        </span>
        <SettingsSwitch
            id="music-lyrics-enabled"
            label="显示歌词"
            disabled={disabled}
            checked={preferences?.musicLyricsEnabled ?? true}
            onChange={(checked) => onPreferences({ musicLyricsEnabled: checked })}
        />
    </div>
);

/** 显示实时频谱：关闭后不占用音频采集。 */
export const MusicSpectrumRow = ({ className, preferences, disabled, onPreferences }: MusicRowProps) => (
    <div className={className}>
        <span className={titleClassName(className, false)}>
            <label htmlFor="music-spectrum-enabled">显示实时频谱</label>
        </span>
        <SettingsSwitch
            id="music-spectrum-enabled"
            label="显示实时频谱"
            disabled={disabled}
            checked={preferences?.musicSpectrumEnabled ?? false}
            onChange={(checked) => onPreferences({ musicSpectrumEnabled: checked })}
        />
    </div>
);

/** 频谱段数：只在频谱开启时可调。 */
export const MusicSpectrumBarsRow = ({ className, selectClassName, preferences, disabled, onPreferences }: MusicSelectRowProps) => (
    <label className={className}>
        <span className={titleClassName(className, false)}>频谱段数</span>
        <select
            className={selectClassName}
            disabled={disabled || !preferences?.musicSpectrumEnabled}
            value={String(preferences?.musicSpectrumBars ?? 32)}
            onChange={(event) => onPreferences({ musicSpectrumBars: Number(event.currentTarget.value) })}
        >
            {MUSIC_SPECTRUM_BARS.map((bars) => (
                <option key={bars} value={String(bars)}>
                    {bars} 段
                </option>
            ))}
        </select>
    </label>
);

/** 频谱帧率：只在频谱开启时可调。 */
export const MusicSpectrumFpsRow = ({ className, selectClassName, preferences, disabled, onPreferences }: MusicSelectRowProps) => (
    <label className={className}>
        <span className={titleClassName(className, false)}>频谱帧率</span>
        <select
            className={selectClassName}
            disabled={disabled || !preferences?.musicSpectrumEnabled}
            value={String(preferences?.musicSpectrumFps ?? 30)}
            onChange={(event) => onPreferences({ musicSpectrumFps: Number(event.currentTarget.value) })}
        >
            {MUSIC_SPECTRUM_FPS.map((fps) => (
                <option key={fps} value={String(fps)}>
                    {fps} 帧/秒
                </option>
            ))}
        </select>
    </label>
);
