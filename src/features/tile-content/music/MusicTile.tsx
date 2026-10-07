import { useState } from "react";
import { ComponentTile, type ComponentTileProps } from "../../components/ComponentTile";
import { MUSIC_CONTROL_MINIMUM_SIZE } from "./MusicControl";
import { MusicWidget } from "./MusicWidget";

/** 音乐拥有内容尺寸反馈，外框仍使用公共几何路径。 */
export const MusicTile = (props: ComponentTileProps) => {
    const [contentMinimum, setContentMinimum] = useState(MUSIC_CONTROL_MINIMUM_SIZE); // 当前内容真实最小需求。
    const minimum = { width: contentMinimum.width + 24, height: contentMinimum.height + 24 + ((props.appearance?.titleMode ?? "fixed") === "fixed" ? 28 : 0) }; // 沿用内容留白和外框占用。
    const bounds = {
        ...props.instance.bounds,
        width: Math.max(props.instance.bounds.width, minimum.width),
        height: Math.max(props.instance.bounds.height, minimum.height),
    }; // 按有效下限呈现，几何提交保存修正值。
    return (
        <ComponentTile {...props} instance={{ ...props.instance, bounds }} title="音乐控制" minimum={minimum} scrollable={false}>
            {props.registration?.entry === "official:music-control" ? (
                <MusicWidget onMinimumSizeChange={setContentMinimum} />
            ) : (
                <p className="official-component__error">组件前端入口不存在</p>
            )}
        </ComponentTile>
    );
};
