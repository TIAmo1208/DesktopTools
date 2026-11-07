/**
 * 示例-多类型格子
 * 演示：
 * 1. 清单 [tiles] 声明三种类型，用户创建时先选类型
 * 2. renderTile 按 context.tileType 画不同粒度，同一份时间数据只起一个定时器
 * 3. 类型缺失时回落到默认类型，不假设宿主一定给了
 */

// 动态载入同级 style.css 资产
const styleUrl = new URL("./style.css", import.meta.url).href;
if (!document.querySelector(`link[data-plugin-style="plugin.example.multi"]`)) {
    const link = document.createElement("link");
    link.rel = "stylesheet";
    link.href = styleUrl;
    link.setAttribute("data-plugin-style", "plugin.example.multi");
    document.head.appendChild(link);
}

/** 三种类型的标题与取数方式。键必须与 manifest.ini 的 [tiles] 一致。 */
const TILE_TYPES = {
    clock: {
        title: "时钟",
        text: (now) => now.toTimeString().split(" ")[0],
    },
    date: {
        title: "日期",
        text: (now) => now.toLocaleDateString("zh-CN", { year: "numeric", month: "long", day: "numeric", weekday: "long" }),
    },
    caption: {
        title: "标签",
        text: (now) => `第 ${Math.floor((now - new Date(now.getFullYear(), 0, 0)) / 86400000)} 天`,
    },
};

export default {
    renderTile(container, context) {
        // 类型缺失或不在清单里时按默认类型画，避免空白格子。
        const tileType = TILE_TYPES[context.tileType] ? context.tileType : "clock";
        const type = TILE_TYPES[tileType];

        container.innerHTML = `
            <div class="plugin-example-multi-tile">
                <div class="tile-title">${type.title}</div>
                <div class="tile-value">--</div>
                <div class="tile-meta">${context.pluginId} · ${tileType}</div>
            </div>
        `;

        const valueEl = container.querySelector(".tile-value");
        const render = () => {
            if (valueEl) valueEl.textContent = type.text(new Date());
        };

        render();
        const timerId = setInterval(render, 1000);

        // 实例离开屏幕或重新渲染时清掉定时器
        return () => {
            clearInterval(timerId);
        };
    },
};
