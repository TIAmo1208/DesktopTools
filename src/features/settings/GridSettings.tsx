import { useEffect, useState } from "react";
import { DEFAULT_DESKTOP_GRID, DEFAULT_FENCE_GRID, type ApplicationPreferences, type GridPreferences } from "../../contracts/settings";
import "./GridSettings.css";

/** 桌面与围栏各自拥有独立编辑值及恢复默认入口。 */
export const GridSettings = ({
    kind,
    preferences,
    disabled,
    onPreferences,
}: {
    kind: "desktopGrid" | "fenceGrid"; // 当前编辑的网格作用域。
    preferences?: ApplicationPreferences; // 后端权威偏好。
    disabled: boolean; // 保存期间暂停编辑。
    onPreferences: (patch: Partial<ApplicationPreferences>) => void; // 提交完整的单组网格。
}) => {
    const defaults = kind === "desktopGrid" ? DEFAULT_DESKTOP_GRID : DEFAULT_FENCE_GRID; // 独立恢复默认值。
    const saved = preferences?.[kind] ?? defaults; // 当前已保存值。
    const [draft, setDraft] = useState(saved); // 一组可整体提交的编辑值。
    const [error, setError] = useState(""); // 输入边界错误。
    useEffect(() => {
        setDraft(saved);
        setError("");
    }, [saved]);

    return (
        <form
            className="grid-settings"
            onSubmit={(event) => {
                event.preventDefault();
                if (!event.currentTarget.reportValidity()) return;
                setError("");
                onPreferences({ [kind]: draft });
            }}
        >
            <fieldset disabled={disabled || !preferences} aria-label={kind === "desktopGrid" ? "桌面网格" : "围栏内网格"}>
                <div className="grid-settings__fields">
                    {(
                        [
                            ["width", "格子宽度", 64, 160],
                            ["height", "格子高度", 90, 180],
                            ["columnGap", "横向间距", 0, 48],
                            ["rowGap", "纵向间距", 0, 48],
                        ] as const
                    ).map(([key, label, min, max]) => (
                        <label key={key}>
                            {label}
                            <span>
                                <input
                                    type="number"
                                    required
                                    min={min}
                                    max={max}
                                    step={1}
                                    value={Number.isNaN(draft[key]) ? "" : draft[key]}
                                    onChange={(event) => setDraft({ ...draft, [key]: event.currentTarget.valueAsNumber } as GridPreferences)}
                                    onInvalid={() => setError("请填写范围内的整数。")}
                                />
                                DIP
                            </span>
                        </label>
                    ))}
                </div>
                <div className="grid-settings__actions">
                    <button
                        type="button"
                        onClick={() => {
                            setDraft(defaults);
                            setError("");
                            onPreferences({ [kind]: { ...defaults } });
                        }}
                    >
                        恢复默认
                    </button>
                    <button type="submit">应用</button>
                </div>
                {error && <p role="alert">{error}</p>}
            </fieldset>
        </form>
    );
};
