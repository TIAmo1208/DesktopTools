import { getPluginSettings, listenPluginSettings, savePluginSettings } from "../../bridge/pluginSettings";
import { loadPluginScript, readPluginApi } from "../../bridge/pluginScripts";

// 当前窗口已激活的主题脚本、它的设置订阅与激活代次；切换主题时用来停旧并丢弃迟到结果。
let live: { id: string; stop: () => void } | undefined;
let subscription: (() => void) | undefined;
let current: { id: string; script: string } | undefined;
let ticket = 0;

/** 停用当前主题脚本，连同它的设置订阅。 */
export const stopThemeScript = () => {
    ticket++;
    subscription?.();
    subscription = undefined;
    const stopped = live;
    live = undefined;
    current = undefined;
    stopped?.stop();
};

/** 主题设置变化后按新值重新执行一次：先跑旧清理，再重新激活。 */
export const restartThemeScript = () => {
    const previous = current;
    stopThemeScript();
    if (previous) void activateThemeScript(previous);
};

/**
 * 激活当前主题的入口脚本：切换时先停旧再启新，加载或执行失败只记日志，不影响界面。
 * 主题导出的 activateTheme 返回清理函数，在停用、切换主题和窗口销毁时执行一次。
 */
export const activateThemeScript = async (active: { id: string; script: string } | undefined) => {
    if (live?.id === active?.id) return; // 同一主题不重复执行。
    stopThemeScript();
    if (!active) return;
    const mine = ticket; // 本次激活的代次，期间换了主题就放弃。
    const failure = await loadPluginScript(active.id, active.script);
    if (mine !== ticket) return;
    if (failure) {
        console.warn(`主题脚本加载失败：${active.id}`, failure);
        return;
    }
    const activate = readPluginApi(active.id)?.activateTheme; // 主题可以不导出这个钩子。
    if (typeof activate !== "function") return;
    // 设置读不到只当作没有设置，不能因此把整个主题脚本卡住不执行。
    const settings = await getPluginSettings(active.id).catch(() => null);
    if (mine !== ticket) return;
    const id = active.id;
    try {
        const stop = activate({
            pluginId: id,
            instanceId: "", // 主题没有格子实例。
            collapsed: false,
            settings: settings ?? null,
            saveSettings: (value) => savePluginSettings(id, value),
        });
        if (mine !== ticket) {
            if (typeof stop === "function") stop(); // 期间换过主题，立刻回收这次激活。
            return;
        }
        live = { id, stop: typeof stop === "function" ? stop : () => undefined };
        current = active;
    } catch (error) {
        console.warn(`主题脚本激活失败：${id}`, error);
        return;
    }
    // 主题设置一改就按新值重放，别的窗口不用重启也能看到开关效果。
    void listenPluginSettings(
        (changedId) => {
            if (changedId === id) restartThemeScript();
        },
        () => undefined,
    )
        .then((unlisten) => {
            if (ticket === mine) subscription = unlisten;
            else unlisten();
        })
        .catch(() => undefined);
};
