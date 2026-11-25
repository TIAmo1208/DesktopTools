/**
 * 插件根相对路径变成同源资产地址。
 * 空路径、绝对路径、`.`/`..` 片段与空片段都拒绝，脚本与图标共用同一条边界。
 */
export const pluginAssetUrl = (path: string): string | undefined => {
    const parts = path.split("/"); // 只接受相对片段。
    if (!path || path.startsWith("/") || parts.some((part) => !part || part === "." || part === "..")) return undefined;
    return `/plugins/${path}`;
};
