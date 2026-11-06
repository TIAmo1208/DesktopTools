// 只读探针：连真实宿主的 CDP 端点，按清单一次性读取运行状态与计算样式，需要时同一轮截取窗口图像。
// 只读约束：不注入状态、不驱动操作、不改页面；探针结论属于「运行时探针」证据，不能替代人眼确认。
import { mkdir, writeFile } from "node:fs/promises"; // 写出截图与结果。
import { dirname, join, resolve } from "node:path"; // 路径拼接与绝对路径归一。
import { pathToFileURL } from "node:url"; // 清单按文件路径动态导入。

const 默认端口 = 9223; // 开发宿主远程调试端口，与「验收流程」L3 探针档一致。
const 默认超时毫秒 = 10000; // 单次探测与单张截图的上限，超时即失败。
const 截图质量 = 60; // 探针截图只服务回归判断，压缩到 60 足够辨识。

// 退出码语义：0 全部完成，1 有检查项失败，2 环境问题。
const 退出码 = { 通过: 0, 失败: 1, 环境: 2 }; // 与既有脚本一致的退出码表。

const 用法 = "用法：node scripts/取证探针.mjs --清单 <检查清单.mjs> [--端口 9223] [--目标 <URL 子串>] [--截图 <输出目录>] [--超时 10000]"; // 参数错误时的用法。

// 参数错误属于环境问题：打印用法后按环境码退出，不抛未捕获堆栈。
function 参数错误(说明) {
    console.error(`[环境] ${说明}`);
    console.error(用法);
    process.exit(退出码.环境);
}

// 逐项识别参数，把运行范围与等待上限都交给使用者显式确定。
const 选项 = { 端口: 默认端口, 目标: "", 清单: "", 截图目录: "", 超时毫秒: 默认超时毫秒 }; // 运行选项。
const 参数 = process.argv.slice(2); // 原始命令行参数。
for (let 索引 = 0; 索引 < 参数.length; 索引 += 1) {
    const 当前 = 参数[索引]; // 当前参数项。
    if (当前 === "--端口") 选项.端口 = Number(参数[(索引 += 1)]); // 调试端口。
    else if (当前 === "--目标") 选项.目标 = 参数[(索引 += 1)] ?? ""; // 目标页 URL 子串。
    else if (当前 === "--清单") 选项.清单 = 参数[(索引 += 1)] ?? ""; // 检查清单文件。
    else if (当前 === "--截图") 选项.截图目录 = 参数[(索引 += 1)] ?? ""; // 截图输出目录。
    else if (当前 === "--超时") 选项.超时毫秒 = Number(参数[(索引 += 1)]); // 单次操作超时。
    else if (!当前) 参数错误("缺少参数");
    else 参数错误(`未知参数：${当前}`);
}
if (!选项.清单) 参数错误("--清单 缺少检查清单文件");
if (!Number.isInteger(选项.端口) || 选项.端口 <= 0) 参数错误("--端口 需要正整数");
if (!Number.isFinite(选项.超时毫秒) || 选项.超时毫秒 <= 0) 参数错误("--超时需要正整数毫秒");

// 单次操作超时：探针与截图都必须有界，超时按失败处理，不循环重试。
const 带超时 = (任务, 说明) =>
    Promise.race([任务, new Promise((_, 拒绝) => setTimeout(() => 拒绝(new Error(`${说明}超时（${选项.超时毫秒}ms）`)), 选项.超时毫秒).unref())]);

// 读取调试端点上的可用页面，按 URL 子串挑一个目标。
async function 选定目标() {
    let 响应;
    try {
        响应 = await 带超时(fetch(`http://127.0.0.1:${选项.端口}/json/list`), "读取调试端点");
    } catch (错误) {
        throw new Error(`调试端点不可用（端口 ${选项.端口}）：${错误 instanceof Error ? 错误.message : String(错误)}`); // 端口没开或宿主未带调试参数，属于环境问题。
    }
    const 页面 = (await 响应.json()).filter((条目) => 条目.type === "page"); // 只接受真实页面。
    const 目标 = 选项.目标 ? 页面.find((条目) => 条目.url.includes(选项.目标)) : 页面[0]; // 未指定时取第一页。
    if (!目标) throw new Error(`调试端点上没有匹配的页面：${选项.目标 || "（任意）"}`);
    return 目标;
}

// 建立一条 CDP 连接：按 id 收发，方法级错误直接抛出，避免把失败读成空值。
async function 连接(endpoint) {
    const socket = new WebSocket(endpoint); // Node 内置 WebSocket。
    let 序号 = 0; // 命令序号。
    const 等待 = new Map(); // 未完成的命令。
    await 带超时(
        new Promise((完成, 失败) => {
            socket.addEventListener("open", 完成, { once: true });
            socket.addEventListener("error", () => 失败(new Error("调试连接失败")), { once: true });
        }),
        "建立调试连接",
    );
    socket.addEventListener("message", (事件) => {
        const 载荷 = JSON.parse(事件.data);
        const 回调 = 等待.get(载荷.id);
        if (!回调) return;
        等待.delete(载荷.id);
        载荷.error ? 回调.失败(new Error(JSON.stringify(载荷.error))) : 回调.完成(载荷.result);
    });
    return {
        发送: (method, params = {}) =>
            带超时(
                new Promise((完成, 失败) => {
                    const id = (序号 += 1);
                    等待.set(id, { 完成, 失败 });
                    socket.send(JSON.stringify({ id, method, params }));
                }),
                method,
            ),
        关闭: () => socket.close(),
    };
}

// 读取清单：检查项与截图项都在同一份文件里，一轮跑完再统一读结果。
async function 读取清单(路径) {
    const 模块 = await import(pathToFileURL(resolve(路径)).href);
    const 清单 = 模块.default;
    if (!清单 || !Array.isArray(清单.检查)) throw new Error("清单必须默认导出 { 检查: [{ 名称, 表达式 }], 截图?: [{ 名称, 选择器 }] }");
    return { 检查: 清单.检查, 截图: Array.isArray(清单.截图) ? 清单.截图 : [] };
}

// 取目标元素的外接矩形；截图裁剪只覆盖该元素，不抓整屏。
async function 元素矩形(cdp, 选择器) {
    const 结果 = await cdp.发送("Runtime.evaluate", {
        expression: `(() => { const node = document.querySelector(${JSON.stringify(选择器)}); if (!node) return null; const box = node.getBoundingClientRect(); return { x: box.x, y: box.y, width: box.width, height: box.height }; })()`,
        returnByValue: true,
    });
    const 矩形 = 结果.result?.value;
    if (!矩形 || 矩形.width <= 0 || 矩形.height <= 0) throw new Error(`找不到可截图的元素：${选择器}`);
    return { x: Math.max(0, Math.floor(矩形.x)), y: Math.max(0, Math.floor(矩形.y)), width: Math.ceil(矩形.width), height: Math.ceil(矩形.height), scale: 1 };
}

// 主流程：先跑完全部检查，再一次性截完所有图，最后统一打印结果。
async function 主流程(清单) {
    const 目标 = await 选定目标();
    const cdp = await 连接(目标.webSocketDebuggerUrl);
    const 输出 = { 目标: 目标.url, 检查: [], 截图: [] }; // 本轮全部结论，供一次读完。
    try {
        for (const 项 of 清单.检查) {
            const 结果 = await cdp.发送("Runtime.evaluate", { expression: 项.表达式, returnByValue: true, awaitPromise: true });
            if (结果.exceptionDetails) throw new Error(`${项.名称}：${结果.exceptionDetails.text}`);
            输出.检查.push({ 名称: 项.名称, 值: 结果.result?.value });
        }
        if (清单.截图.length) {
            if (!选项.截图目录) throw new Error("清单含截图项时必须给 --截图 输出目录");
            await mkdir(选项.截图目录, { recursive: true });
            for (const 项 of 清单.截图) {
                const clip = await 元素矩形(cdp, 项.选择器);
                const { data } = await cdp.发送("Page.captureScreenshot", { format: "jpeg", quality: 截图质量, fromSurface: true, clip });
                const 路径 = join(选项.截图目录, `${项.名称}.jpg`);
                await writeFile(路径, Buffer.from(data, "base64"));
                输出.截图.push({ 名称: 项.名称, 路径 });
            }
        }
    } finally {
        cdp.关闭();
    }
    console.log(JSON.stringify(输出, null, 2));
    if (选项.截图目录) {
        const 结果文件 = join(选项.截图目录, "探针结果.json"); // 与截图同目录，便于一次读取。
        await mkdir(dirname(结果文件), { recursive: true });
        await writeFile(结果文件, JSON.stringify(输出, null, 2), "utf8");
        console.log(`[探针] 结果已写入 ${结果文件}`);
    }
    return 退出码.通过;
}

// 清单读不出来属于用法问题，按环境码退出，避免被读成检查失败。
let 清单;
try {
    清单 = await 读取清单(选项.清单);
} catch (错误) {
    console.error(`[环境] 读取清单失败：${错误 instanceof Error ? 错误.message : String(错误)}`);
    process.exit(退出码.环境);
}

try {
    process.exit(await 主流程(清单));
} catch (错误) {
    const 说明 = 错误 instanceof Error ? 错误.message : String(错误); // 统一的失败说明。
    console.error(`[失败] ${说明}`);
    if (说明.includes("超时") || 说明.includes("调试连接") || 说明.includes("调试端点")) process.exit(退出码.环境);
    process.exit(退出码.失败);
}
