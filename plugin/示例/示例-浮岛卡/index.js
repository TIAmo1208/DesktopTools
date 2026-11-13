/**
 * 示例-浮岛卡
 * 演示：
 * 1. 清单 island = 1，插件内容作为浮岛卡显示（creatable = 0，不占桌面格子）；
 * 2. 只用 --fence-* 令牌画状态色，浮岛根持有整组令牌，主题替换与深浅切换时跟随；
 * 3. renderTile 返回清理函数：折叠与展开切换会重挂内容，定时器必须在清理函数里撤掉。
 */

const styleUrl = new URL("./style.css", import.meta.url).href;
if (!document.querySelector(`link[data-plugin-style="plugin.example.island"]`)) {
    const link = document.createElement("link");
    link.rel = "stylesheet";
    link.href = styleUrl;
    link.setAttribute("data-plugin-style", "plugin.example.island");
    document.head.appendChild(link);
}

export default {
    renderTile(container, context) {
        const root = document.createElement("div");
        root.className = "plugin-example-island";
        root.dataset.collapsed = String(context.collapsed); // 常态卡片 inert，容器高 28 DIP，只画一行。

        const dot = document.createElement("span");
        dot.className = "plugin-example-island__dot";
        dot.setAttribute("aria-hidden", "true");

        const label = document.createElement("span");
        label.className = "plugin-example-island__label";
        label.textContent = "示例浮岛卡";

        const time = document.createElement("time");
        time.className = "plugin-example-island__time";

        root.append(dot, label, time);
        container.replaceChildren(root);

        const tick = () => {
            time.textContent = new Date().toTimeString().slice(0, 8);
        };
        tick();
        const timer = setInterval(tick, 1000);

        return () => clearInterval(timer);
    },
};
