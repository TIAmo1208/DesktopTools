import { pluginAssetPath } from "../contracts/plugin.ts";

/**
 * 插件根相对路径变成同源资产地址。
 * 空路径、绝对路径、`.`/`..` 片段与空片段都拒绝，脚本与图标共用同一条边界。
 */
export const pluginAssetUrl = (path: string): string | undefined => {
    return pluginAssetPath(path) ? `/plugins/${path}` : undefined;
};
