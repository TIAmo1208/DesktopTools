/**
 * 示例-设置条目
 * creatable=0，不导出 renderTile，只导出 settingsEntries。
 * 演示：
 * 1. 通过 new URL('./style.css', import.meta.url) 动态挂载样式表（满足 style-src 'self'，不使用内联 style）
 * 2. 读取 context.settings 与 saveSettings 保存并重绘
 * 3. 用户编辑文本使用 textContent / value 赋值，避免 innerHTML 注入
 */

// 动态载入同级 style.css 资产
const styleUrl = new URL("./style.css", import.meta.url).href;
if (!document.querySelector(`link[data-plugin-style="plugin.example.settings"]`)) {
    const link = document.createElement("link");
    link.rel = "stylesheet";
    link.href = styleUrl;
    link.setAttribute("data-plugin-style", "plugin.example.settings");
    document.head.appendChild(link);
}

export default {
    settingsEntries: [
        {
            id: "greeting-config",
            name: "问候语与启用状态",
            render(container, context) {
                // 读取当前设置（初次或无设置时为 null，取默认值）
                const currentSettings = (context.settings && typeof context.settings === "object")
                    ? context.settings
                    : { greeting: "你好，欢迎使用桌面小工具！", enabled: true };

                container.innerHTML = `
                    <div class="plugin-example-settings-panel">
                        <label class="checkbox-label">
                            <input type="checkbox" id="cfg-enabled" />
                            <span>启用问候消息</span>
                        </label>
                        <div class="field-group">
                            <label class="field-hint" for="cfg-greeting">自定义问候语内容：</label>
                            <input type="text" id="cfg-greeting" />
                        </div>
                        <div class="actions-row">
                            <button class="save-btn" id="cfg-save">保存配置</button>
                            <span class="status-text" id="cfg-status">已保存</span>
                        </div>
                    </div>
                `;

                const checkbox = container.querySelector("#cfg-enabled");
                const textInput = container.querySelector("#cfg-greeting");
                const saveBtn = container.querySelector("#cfg-save");
                const statusSpan = container.querySelector("#cfg-status");

                // 使用属性与 DOM 赋值，避免 HTML 注入与内联样式
                if (checkbox) {
                    checkbox.checked = Boolean(currentSettings.enabled);
                }
                if (textInput) {
                    textInput.value = typeof currentSettings.greeting === "string" ? currentSettings.greeting : "";
                }

                let statusTimer = null;

                const handleSave = async () => {
                    const newSettings = {
                        enabled: Boolean(checkbox.checked),
                        greeting: textInput.value.trim(),
                    };
                    saveBtn.disabled = true;
                    try {
                        await context.saveSettings(newSettings);
                        if (statusSpan) {
                            statusSpan.classList.add("status-text--visible");
                            if (statusTimer) clearTimeout(statusTimer);
                            statusTimer = setTimeout(() => {
                                statusSpan.classList.remove("status-text--visible");
                            }, 2000);
                        }
                    } finally {
                        saveBtn.disabled = false;
                    }
                };

                saveBtn.addEventListener("click", handleSave);

                // 返回清理函数
                return () => {
                    saveBtn.removeEventListener("click", handleSave);
                    if (statusTimer) clearTimeout(statusTimer);
                };
            },
        },
    ],
};
