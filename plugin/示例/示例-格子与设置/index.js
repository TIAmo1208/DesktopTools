/**
 * 示例-格子与设置
 * 演示：
 * 1. 通过 new URL('./style.css', import.meta.url) 动态加载样式表（满足 style-src 'self'，不使用内联 style）
 * 2. renderTile 展示设置中的文本和字号（字号使用样式类名切换，文本使用 textContent 注入避免 XSS）
 * 3. settingsEntries 修改设置并保存
 * 4. maxTiles=2 多实例与 instanceId 区分
 */

// 动态载入同级 style.css 资产
const styleUrl = new URL("./style.css", import.meta.url).href;
if (!document.querySelector(`link[data-plugin-style="plugin.example.combined"]`)) {
    const link = document.createElement("link");
    link.rel = "stylesheet";
    link.href = styleUrl;
    link.setAttribute("data-plugin-style", "plugin.example.combined");
    document.head.appendChild(link);
}

function resolveSettings(rawSettings) {
    const defaults = {
        text: "欢迎使用复合插件示例",
        fontSize: "14px",
    };
    if (rawSettings && typeof rawSettings === "object") {
        return {
            text: typeof rawSettings.text === "string" ? rawSettings.text : defaults.text,
            fontSize: typeof rawSettings.fontSize === "string" ? rawSettings.fontSize : defaults.fontSize,
        };
    }
    return defaults;
}

function sizeToClass(fontSize) {
    switch (fontSize) {
        case "12px":
            return "tile-content--size-12";
        case "16px":
            return "tile-content--size-16";
        case "18px":
            return "tile-content--size-18";
        case "14px":
        default:
            return "tile-content--size-14";
    }
}

export default {
    renderTile(container, context) {
        const { text, fontSize } = resolveSettings(context.settings);

        container.innerHTML = `
            <div class="plugin-example-combined-tile">
                <div class="tile-title">复合示例 (maxTiles: 2)</div>
                <div class="tile-content ${sizeToClass(fontSize)}" id="combined-tile-text"></div>
                <div class="tile-footer">
                    <span class="tile-footer-item" id="combined-tile-instance"></span>
                    <span class="tile-footer-item" id="combined-tile-collapsed"></span>
                </div>
            </div>
        `;

        const textEl = container.querySelector("#combined-tile-text");
        if (textEl) {
            textEl.textContent = text;
        }

        const instanceEl = container.querySelector("#combined-tile-instance");
        if (instanceEl) {
            instanceEl.textContent = `实例: ${context.instanceId}`;
        }

        const collapsedEl = container.querySelector("#combined-tile-collapsed");
        if (collapsedEl) {
            collapsedEl.textContent = `折叠: ${context.collapsed ? "是" : "否"}`;
        }

        // 演示返回清理函数
        return () => {
            // 在此清理该实例的资源
        };
    },

    settingsEntries: [
        {
            id: "display-options",
            name: "显示内容与文字大小",
            render(container, context) {
                const { text, fontSize } = resolveSettings(context.settings);

                container.innerHTML = `
                    <div class="plugin-example-combined-settings">
                        <div class="field-row">
                            <label class="field-label" for="combined-text-input">展示文本内容：</label>
                            <input type="text" id="combined-text-input" />
                        </div>
                        <div class="field-row">
                            <label class="field-label" for="combined-fontsize-select">文字字号：</label>
                            <select id="combined-fontsize-select">
                                <option value="12px">小 (12px)</option>
                                <option value="14px">默认 (14px)</option>
                                <option value="16px">大 (16px)</option>
                                <option value="18px">超大 (18px)</option>
                            </select>
                        </div>
                        <div class="actions-row">
                            <button class="save-button" id="combined-save-btn">保存并应用</button>
                            <span class="status-msg" id="combined-status-msg">已保存，格子已同步</span>
                        </div>
                    </div>
                `;

                const textInput = container.querySelector("#combined-text-input");
                const fontSelect = container.querySelector("#combined-fontsize-select");
                const saveBtn = container.querySelector("#combined-save-btn");
                const statusSpan = container.querySelector("#combined-status-msg");

                if (textInput) {
                    textInput.value = text;
                }
                if (fontSelect) {
                    fontSelect.value = ["12px", "14px", "16px", "18px"].includes(fontSize) ? fontSize : "14px";
                }

                let statusTimer = null;

                const onSave = async () => {
                    const newSettings = {
                        text: textInput.value.trim(),
                        fontSize: fontSelect.value,
                    };
                    saveBtn.disabled = true;
                    try {
                        await context.saveSettings(newSettings);
                        if (statusSpan) {
                            statusSpan.classList.add("status-msg--visible");
                            if (statusTimer) clearTimeout(statusTimer);
                            statusTimer = setTimeout(() => {
                                statusSpan.classList.remove("status-msg--visible");
                            }, 2000);
                        }
                    } finally {
                        saveBtn.disabled = false;
                    }
                };

                saveBtn.addEventListener("click", onSave);

                return () => {
                    saveBtn.removeEventListener("click", onSave);
                    if (statusTimer) clearTimeout(statusTimer);
                };
            },
        },
    ],
};
