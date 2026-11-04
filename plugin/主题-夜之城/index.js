/**
 * 夜之城主题的脚本半边
 * 1. theme.css 与 theme.json 负责皮肤；这里负责特效开关、设置条目和可创建的时钟格子
 * 2. 开关写进文档根的 data-nightcity-* 属性，样式表只按属性开关，不改主题数据
 * 3. 时钟只读本机时间，十六进制读出由秒数派生，卸载时清掉定时器与监听
 */

const PLUGIN_ID = "plugin.theme-nightcity"; // 清单标识，同时作为样式表键。
const styleUrl = new URL("./style.css", import.meta.url).href; // 同级样式表资产。
const HEX_MASK = 0xffffff; // 数据流读出保留 24 位。

/** 可独立关闭的特效；键同时决定 data-nightcity-<键> 属性名。 */
const OPTIONS = [
    { key: "scan", label: "CRT 扫描线", hint: "深色叠扫描线与城市网格，浅色只留稀疏网格" },
    { key: "bevel", label: "45° 切角", hint: "按钮削角；开关轨道与滑块不参与" },
    { key: "glow", label: "悬停冷光", hint: "按钮悬停时叠一层青色光晕" },
    { key: "mono", label: "等宽数字", hint: "时间与数字改用等宽字形，读数不抖动" },
];

// 纯逻辑测试在 Node 里直接导入本模块，那里没有 document。
if (typeof document !== "undefined" && !document.querySelector(`link[data-plugin-style="${PLUGIN_ID}"]`)) {
    const link = document.createElement("link");
    link.rel = "stylesheet";
    link.href = styleUrl;
    link.setAttribute("data-plugin-style", PLUGIN_ID);
    document.head.appendChild(link);
}

/** 两位补零，时分秒与十六进制共用。 */
const pad = (value) => String(value).padStart(2, "0");

/** 设置值归一成四个布尔；非对象、缺键都按默认开启。 */
export function normalizeOptions(settings) {
    const saved = settings && typeof settings === "object" && !Array.isArray(settings) ? settings : {};
    return Object.fromEntries(OPTIONS.map((option) => [option.key, saved[option.key] !== false]));
}

/**
 * 把开关写到文档根：样式表按 data-nightcity-<键>="on" / "off" 生效。
 * 默认态写 "on"，删掉属性也等价于默认，避免主题切换后残留关闭状态。
 */
export function applyThemeOptions(root, settings) {
    const options = normalizeOptions(settings);
    for (const option of OPTIONS) {
        const name = option.key.charAt(0).toUpperCase() + option.key.slice(1);
        root.dataset[`nightcity${name}`] = options[option.key] ? "on" : "off";
    }
    return options;
}

/** 撤掉开关属性，主题停用或切换时回到宿主默认。 */
export function clearThemeOptions(root) {
    for (const option of OPTIONS) {
        delete root.dataset[`nightcity${option.key.charAt(0).toUpperCase()}${option.key.slice(1)}`];
    }
}

/**
 * 把毫秒时间换算成夜之城的 HUD 读出。
 * 十六进制值只由秒数派生，所以同一秒内重复调用结果一致。
 */
export function nightCityReadout(unixMillis, showSeconds = false) {
    const date = new Date(unixMillis);
    const clock = `${pad(date.getHours())}:${pad(date.getMinutes())}${showSeconds ? `:${pad(date.getSeconds())}` : ""}`;
    const seed = Math.floor(unixMillis / 1000) % HEX_MASK;
    return { clock, hex: `0x${seed.toString(16).toUpperCase().padStart(6, "0")}` };
}

/** 四个开关的设置条目：保存后立即作用到当前窗口。 */
const themeOptionsEntry = {
    id: "nightcity-effects",
    name: "夜之城特效",
    render(container, context) {
        const root = document.createElement("div");
        root.className = "nightcity-options";
        const state = normalizeOptions(context.settings);

        for (const option of OPTIONS) {
            const row = document.createElement("label");
            row.className = "nightcity-options__row";
            const text = document.createElement("span");
            text.className = "nightcity-options__text";
            const label = document.createElement("span");
            label.className = "nightcity-options__label";
            label.textContent = option.label;
            const hint = document.createElement("span");
            hint.className = "nightcity-options__hint";
            hint.textContent = option.hint;
            text.append(label, hint);

            const input = document.createElement("input");
            input.type = "checkbox";
            input.className = "nightcity-options__input";
            input.checked = state[option.key];
            input.setAttribute("aria-label", option.label);
            input.addEventListener("change", () => {
                state[option.key] = input.checked;
                applyThemeOptions(document.documentElement, state); // 当前窗口立刻生效。
                void context.saveSettings({ ...state }).catch(() => undefined); // 其他窗口靠设置广播重新激活。
            });

            row.append(text, input);
            root.append(row);
        }

        container.append(root);
        return () => root.remove();
    },
};

export default {
    activateTheme(context) {
        const root = document.documentElement;
        applyThemeOptions(root, context.settings);
        return () => clearThemeOptions(root);
    },

    settingsEntries: [themeOptionsEntry],

    /** 生成一个时钟格子，折叠时只留时间读出。 */
    renderTile(container, context) {
        const root = document.createElement("div");
        root.className = "nightcity-clock";
        root.toggleAttribute("data-collapsed", Boolean(context.collapsed));

        const hex = document.createElement("span");
        hex.className = "nightcity-clock__hex";
        hex.setAttribute("aria-hidden", "true"); // 数据流只是氛围，不进辅助技术读出。

        const clock = document.createElement("time");
        clock.className = "nightcity-clock__time";

        const zone = document.createElement("span");
        zone.className = "nightcity-clock__zone";
        zone.textContent = "NC-TIME";

        root.append(hex, clock, zone);
        container.append(root);

        const paint = () => {
            const now = Date.now();
            const readout = nightCityReadout(now);
            hex.textContent = readout.hex;
            clock.textContent = readout.clock;
            clock.dateTime = new Date(now).toISOString();
        };
        paint();
        // 性能页的「暂停刷新」是省电开关：暂停期间不重画，恢复时立刻补一帧，不等下一个整秒。
        const tick = () => {
            if (window.__DESKTOP_TOOLS_NATIVE__?.isRefreshPaused?.()) return;
            paint();
        };
        const timer = setInterval(tick, 1000); // 与钟面走秒对齐，不额外插帧。
        window.addEventListener("desktop-refresh-changed", tick);
        return () => {
            clearInterval(timer);
            window.removeEventListener("desktop-refresh-changed", tick);
        };
    },
};
