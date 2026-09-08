import "./SettingsSwitch.css";

/** 设置页共用的受控开关，保持原生键盘操作与独立点击区域。 */
export const SettingsSwitch = ({
    id,
    label,
    checked,
    disabled,
    onChange,
}: {
    id?: string; // 可选的外部标签关联标识。
    label: string; // 辅助技术可读的设置名称。
    checked: boolean; // 已保存的开关状态。
    disabled: boolean; // 加载或保存期间禁止操作。
    onChange: (checked: boolean) => void; // 提交新的开关状态。
}) => (
    <label className="settings-switch">
        <span>{checked ? "开" : "关"}</span>
        <input
            id={id}
            type="checkbox"
            role="switch"
            aria-label={label}
            checked={checked}
            disabled={disabled}
            onChange={(event) => onChange(event.currentTarget.checked)}
        />
        <span className="settings-switch__track" aria-hidden="true" />
    </label>
);
