/**
 * 封锁橙主题的脚本半边
 * 1. theme.css 与 theme.json 负责皮肤；这里只负责两档可撤销特效、设置条目与设置值归一
 * 2. 开关写进文档根的 data-blockade-* 属性，切角量写进主题私有变量 --blockade-bevel，样式表只按这些状态生效
 * 3. 行顶橙条是设计语言的焦点语言，常驻生效，不做开关；停用或切换主题时清理函数移除全部属性与变量
 */

const PLUGIN_ID = "plugin.theme-blockade"; // 清单标识，同时作为样式表键。
const styleUrl = new URL("./style.css", import.meta.url).href; // 同级样式表资产。
const DEFAULT_BEVEL_PIXELS = 8; // 切角量独立于宿主圆角偏好。

/** 可独立关闭的特效；键同时决定 data-blockade-<键> 属性名。 */
const OPTIONS = [
    { key: "bevel", label: "45° 切角", hint: "按钮与输入削角；整行命中层、菜单命令与开关不参与" },
    { key: "mono", label: "等宽数字", hint: "数字与读数改用等宽字形，读数不抖动" },
];

// 纯逻辑测试在 Node 里直接导入本模块，那里没有 document。
if (typeof document !== "undefined" && !document.querySelector(`link[data-plugin-style="${PLUGIN_ID}"]`)) {
    const link = document.createElement("link");
    link.rel = "stylesheet";
    link.href = styleUrl;
    link.setAttribute("data-plugin-style", PLUGIN_ID);
    document.head.appendChild(link);
}

/** 设置值归一成布尔特效；非对象、缺键都按默认开启。 */
export function normalizeOptions(settings) {
    const saved = settings && typeof settings === "object" && !Array.isArray(settings) ? settings : {};
    return Object.fromEntries(OPTIONS.map((option) => [option.key, saved[option.key] !== false]));
}

/** 切角像素独立归一：非法值回默认，合法数值限制在 0–12。 */
export function normalizeBevelPixels(value) {
    return typeof value === "number" && Number.isFinite(value) ? Math.min(12, Math.max(0, value)) : DEFAULT_BEVEL_PIXELS;
}

/**
 * 把开关写到文档根：样式表按 data-blockade-<键>="on" / "off" 生效。
 * 默认态写 "on"，删掉属性也等价于默认，避免主题切换后残留关闭状态。
 */
export function applyThemeOptions(root, settings) {
    const options = normalizeOptions(settings);
    for (const option of OPTIONS) {
        const name = option.key.charAt(0).toUpperCase() + option.key.slice(1);
        root.dataset[`blockade${name}`] = options[option.key] ? "on" : "off";
    }
    root.style.setProperty("--blockade-bevel", `${normalizeBevelPixels(settings?.bevelPixels)}px`);
    return options;
}

/** 撤掉开关属性与切角变量，主题停用或切换时回到宿主默认。 */
export function clearThemeOptions(root) {
    for (const option of OPTIONS) {
        delete root.dataset[`blockade${option.key.charAt(0).toUpperCase()}${option.key.slice(1)}`];
    }
    root.style.removeProperty("--blockade-bevel");
}

/** 布尔特效与独立数值项：保存后立即作用到当前窗口。 */
const themeOptionsEntry = {
    id: "blockade-effects",
    name: "封锁橙特效",
    render(container, context) {
        const root = document.createElement("div");
        root.className = "blockade-options";
        const state = normalizeOptions(context.settings);
        let bevelPixels = normalizeBevelPixels(context.settings?.bevelPixels);

        for (const option of OPTIONS) {
            const row = document.createElement("label");
            row.className = "blockade-options__row";
            const text = document.createElement("span");
            text.className = "blockade-options__text";
            const label = document.createElement("span");
            label.className = "blockade-options__label";
            label.textContent = option.label;
            const hint = document.createElement("span");
            hint.className = "blockade-options__hint";
            hint.textContent = option.hint;
            text.append(label, hint);

            const input = document.createElement("input");
            input.type = "checkbox";
            input.className = "blockade-options__input";
            input.checked = state[option.key];
            input.setAttribute("aria-label", option.label);
            input.addEventListener("change", () => {
                state[option.key] = input.checked;
                applyThemeOptions(document.documentElement, { ...state, bevelPixels }); // 当前窗口立刻生效。
                void context.saveSettings({ ...state, bevelPixels }).catch(() => undefined); // 其他窗口靠设置广播重新激活。
            });

            row.append(text, input);
            root.append(row);
        }

        const bevelRow = document.createElement("label");
        bevelRow.className = "blockade-options__row";
        const bevelText = document.createElement("span");
        bevelText.className = "blockade-options__text";
        const bevelLabel = document.createElement("span");
        bevelLabel.className = "blockade-options__label";
        bevelLabel.textContent = "切角像素";
        const bevelHint = document.createElement("span");
        bevelHint.className = "blockade-options__hint";
        bevelHint.textContent = "只调整切角量，与界面圆角无关";
        bevelText.append(bevelLabel, bevelHint);
        const bevelInput = document.createElement("input");
        bevelInput.type = "number";
        bevelInput.className = "blockade-options__number";
        bevelInput.min = "0";
        bevelInput.max = "12";
        bevelInput.step = "1";
        bevelInput.value = String(bevelPixels);
        bevelInput.setAttribute("aria-label", "切角像素");
        bevelInput.addEventListener("change", () => {
            bevelPixels = normalizeBevelPixels(bevelInput.valueAsNumber);
            bevelInput.value = String(bevelPixels);
            applyThemeOptions(document.documentElement, { ...state, bevelPixels });
            void context.saveSettings({ ...state, bevelPixels }).catch(() => undefined);
        });
        bevelRow.append(bevelText, bevelInput);
        root.append(bevelRow);

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
};
