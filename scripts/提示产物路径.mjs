// 启动开发宿主前打印本轮真正会加载的产物路径，避免把 Vite 页面或仓库 plugin/ 当成运行中的产物。
import { existsSync } from "node:fs"; // 判断产物是否已生成。
import { dirname, join, resolve } from "node:path"; // 仓库根与路径拼接。
import { fileURLToPath } from "node:url"; // 当前模块路径。

const 仓库根 = resolve(dirname(fileURLToPath(import.meta.url)), ".."); // scripts/ 的上一级。
const 前端产物 = join(仓库根, "build", "frontend"); // 设置窗与桌面宿主加载的前端。
const 插件产物 = join(仓库根, "build", "target", "debug", "plugin"); // 开发运行扫描的插件目录。

console.log(`[产物] 设置窗/前端：${前端产物}${existsSync(前端产物) ? "" : "（缺失，先 npm run build）"}`);
console.log(`[产物] 调试插件目录：${插件产物}${existsSync(插件产物) ? "" : "（尚未创建，首次构建后生成）"}；仓库 plugin/ 改动后先 npm run sync:plugin`);
