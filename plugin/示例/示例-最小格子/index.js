/**
 * 示例-最小格子
 * 演示：
 * 1. 通过 new URL('./style.css', import.meta.url) 动态挂载样式表（满足 style-src 'self'，不使用内联 style）
 * 2. renderTile 挂载静态内容与退出清理函数（清掉定时器）
 */

// 动态载入同级 style.css 资产
const styleUrl = new URL("./style.css", import.meta.url).href;
if (!document.querySelector(`link[data-plugin-style="plugin.example.minimal"]`)) {
    const link = document.createElement("link");
    link.rel = "stylesheet";
    link.href = styleUrl;
    link.setAttribute("data-plugin-style", "plugin.example.minimal");
    document.head.appendChild(link);
}

export default {
    renderTile(container, context) {
        container.innerHTML = `
            <div class="plugin-example-minimal-tile">
                <div class="tile-title">最小格子示例</div>
                <div class="tile-meta" id="minimal-meta-plugin"></div>
                <div class="tile-meta" id="minimal-meta-instance"></div>
                <div class="tile-clock" id="minimal-runtime-clock">00:00:00</div>
            </div>
        `;

        const pluginMeta = container.querySelector("#minimal-meta-plugin");
        if (pluginMeta) {
            pluginMeta.textContent = `插件: ${context.pluginId}`;
        }

        const instanceMeta = container.querySelector("#minimal-meta-instance");
        if (instanceMeta) {
            instanceMeta.textContent = `实例: ${context.instanceId}`;
        }

        const clockEl = container.querySelector("#minimal-runtime-clock");
        const updateTime = () => {
            if (clockEl) {
                const now = new Date();
                clockEl.textContent = now.toTimeString().split(" ")[0];
            }
        };

        updateTime();
        const timerId = setInterval(updateTime, 1000);

        // 必须返回清理函数，在实例销毁或重新渲染时清掉定时器
        return () => {
            clearInterval(timerId);
        };
    },
};
