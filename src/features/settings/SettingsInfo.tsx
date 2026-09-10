import * as Tooltip from "@radix-ui/react-tooltip";
import infoIcon from "../../../resources/info.svg?raw";
import "./SettingsInfo.css";

/** 设置名称旁的可聚焦补充说明，避让与可访问关联由 Radix 提供。 */
export const SettingsInfo = ({ label, description }: { label: string; description: string }) => (
    <Tooltip.Provider>
        <Tooltip.Root>
            <Tooltip.Trigger asChild>
                <button className="settings-info" type="button" aria-label={`关于${label}`}>
                    <span aria-hidden="true" dangerouslySetInnerHTML={{ __html: infoIcon }} />
                </button>
            </Tooltip.Trigger>
            <Tooltip.Portal>
                <Tooltip.Content className="settings-info__tooltip" sideOffset={8} collisionPadding={8} avoidCollisions>
                    {description}
                </Tooltip.Content>
            </Tooltip.Portal>
        </Tooltip.Root>
    </Tooltip.Provider>
);
