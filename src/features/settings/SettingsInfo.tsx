import * as Tooltip from "@radix-ui/react-tooltip";
import { useRef, useState } from "react";
import infoIcon from "../../../resources/info.svg?raw";
import "./SettingsInfo.css";

/** 设置名称旁的可聚焦补充说明，避让与可访问关联由 Radix 提供。 */
export const SettingsInfo = ({ label, description }: { label: string; description: string }) => {
    const trigger = useRef<HTMLButtonElement>(null); // 说明按钮，用于定位所属弹窗。
    const [container, setContainer] = useState<HTMLElement>(); // 弹窗内的说明必须挂在弹窗内部，否则会被模态层遮住。
    return (
        <Tooltip.Provider>
            <Tooltip.Root
                onOpenChange={(open) => {
                    if (open) setContainer(trigger.current?.closest("dialog") ?? undefined);
                }}
            >
                <Tooltip.Trigger asChild>
                    <button ref={trigger} className="settings-info" type="button" aria-label={`关于${label}`}>
                        <span aria-hidden="true" dangerouslySetInnerHTML={{ __html: infoIcon }} />
                    </button>
                </Tooltip.Trigger>
                <Tooltip.Portal container={container}>
                    <Tooltip.Content className="settings-info__tooltip" sideOffset={8} collisionPadding={8} avoidCollisions>
                        {description}
                    </Tooltip.Content>
                </Tooltip.Portal>
            </Tooltip.Root>
        </Tooltip.Provider>
    );
};
