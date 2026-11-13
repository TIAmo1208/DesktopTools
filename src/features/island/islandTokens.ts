/**
 * 浮岛根从桌面围栏复制的完整令牌组。
 *
 * 浮岛自身只画表面、描边与正文，但浮岛上的插件卡按桌面契约可以使用整组 `--fence-*` 语义色，
 * 所以不裁剪成 surface / border / text 三项，避免插件卡在浮岛上掉色。
 * 顺序与 `docs/design/主题变量契约.md` 的「桌面网格（围栏）」表一致；
 * `islandTokens.test.ts` 核对它覆盖 `.fence-workspace` 声明的全部围栏令牌。
 */
export const ISLAND_FENCE_TOKENS = [
    "--fence-border",
    "--fence-surface",
    "--fence-text",
    "--fence-active",
    "--fence-warning",
    "--fence-error",
    "--fence-muted",
    "--fence-focus-border",
    "--fence-item-text-shadow",
] as const;
