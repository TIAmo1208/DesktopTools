import type { ComponentManifest } from "../../contracts/component.ts";

/** 下拉始终保留默认项，未读取到主题或全部停用时也可恢复内置主题。 */
export const appearanceThemeOptions = (manifests: readonly ComponentManifest[] | undefined, disabledPlugins: readonly string[]) => {
    const available = (manifests ?? [])
        .filter((manifest) => manifest.role === "theme" && !disabledPlugins.includes(manifest.id))
        .sort((a, b) => a.name.localeCompare(b.name, "zh")); // 复制数组再排序，不改运行时快照。
    return [{ id: "", name: "默认" }, ...available.map(({ id, name }) => ({ id, name }))];
};
