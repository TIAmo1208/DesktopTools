import type { ComponentManifest, ComponentTileSize, ComponentTileType } from "../../contracts/component";
import type { DesktopTileMenuItem } from "../tiles/DesktopTileMenu";
import type { PluginMenuEntry } from "../../bridge/pluginMenu";

/** 按实例保存的类型键找类型声明；空键或未知键回落到首个类型。 */
export const matchTileType = (manifest: ComponentManifest | undefined, tileType: string): ComponentTileType | undefined => {
    if (!manifest) return undefined;
    return manifest.tileTypes.find((item) => item.key === tileType) ?? manifest.tileTypes[0];
};

/** 类型级最小尺寸；未声明时返回 undefined，由公共外框使用公共底线。 */
export const pluginTileMinimum = (manifest: ComponentManifest | undefined, tileType: string): ComponentTileSize | undefined =>
    matchTileType(manifest, tileType)?.minimumSize ?? undefined;

/** 格子标题：声明了类型就用该类型的显示名，隐式单类型沿用插件名。 */
export const pluginTileTitle = (manifest: ComponentManifest | undefined, tileType: string, fallback: string): string => matchTileType(manifest, tileType)?.name ?? fallback;

/** 插件菜单项映射到公共菜单；分组用 separatorBefore 表达，顺序保持注册顺序。 */
export const pluginMenuItems = (entries: readonly PluginMenuEntry[]): DesktopTileMenuItem[] =>
    entries.map((entry) => ({
        label: entry.label,
        separatorBefore: entry.separatorBefore === true,
        danger: entry.danger === true,
        disabled: entry.disabled === true,
        description: entry.description,
        onSelect: entry.onSelect,
    }));
