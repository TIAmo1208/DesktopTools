// 无头运行真实组件浏览器断言页：每个页面一个独立的 Edge headless 实例 + CDP 读取 #result，输出 PASS/FAIL；样张页记 INSPECT，真机专用页记 SKIP。
import { execFileSync, spawn } from "node:child_process"; // 进程控制：结束残留 Edge 与启动无头 Edge。
import { createHash } from "node:crypto";
import { connect } from "node:net"; // 探测 127.0.0.1:1420 是否监听。
import { appendFile, mkdir, mkdtemp, readFile, readdir, rm, writeFile } from "node:fs/promises"; // 临时 user-data-dir、日志与清理。
import { existsSync } from "node:fs"; // 探测 Edge 可执行文件与端口文件。
import { tmpdir } from "node:os"; // 系统临时目录。
import { join, resolve } from "node:path"; // 路径拼接与绝对路径归一。
import { request } from "node:http"; // 直连本机 CDP 端点，不受系统代理影响。
import { expectedPixelFrames, validatePixelFrame, summarizePixelFrames } from "./浮岛像素门禁.mjs";
import { 断言页, 真机专用页, 检查页 } from "./断言页目录.mjs"; // 断言页、真机专用页与只核对加载的检查页清单。

const 开发服务器地址 = "http://127.0.0.1:1420"; // 由 npm run dev 提供的断言页服务地址。
const 轮询间隔毫秒 = 100; // 每页读取 #result 的固定间隔，不引入常驻轮询。
const 默认超时毫秒 = 20000; // 单页从打开到出现结论的上限；4 秒内无结论基本就是页面缺依赖或脚本异常。
const 端点超时毫秒 = 30000; // 等待 Edge 写出调试端点的上限；实测首次启动 1 秒内即可写出，留足慢机器余量。

// 退出码语义：0 全部通过，1 存在失败页，2 环境问题。
const 退出码 = { 通过: 0, 失败: 1, 环境: 2 }; // 与设计文档一致的退出码表。

// 解析命令行参数，未识别参数按环境问题（用法错误）退出，不抛未捕获堆栈。
const 参数 = process.argv.slice(2); // 原始命令行参数。
const 选项 = { 超时毫秒: 默认超时毫秒, 页过滤: "", 详细: false, 浮岛像素: false, 像素筛选: "", 像素对照: "normal", 自然帧: false, 像素基线: "", 稳态对照: false }; // 运行选项。
const 用法 = "用法：node scripts/跑断言页.mjs [--页 <路径子串>] [--超时 30000] [--verbose] [--浮岛像素 [--像素筛选 <形态/边/DPR/方向/毫秒/背景的正则>] [--像素基线 <旧证据目录>] [--像素对照 normal|card-solid|patch-solid|all-solid|overlap] [--自然帧] [--稳态对照]]"; // 默认不进行像素取证；取证入口只针对islandFillet独立运行。

// 参数不属于页面判定结果：打印用法后以「环境」退出码结束，避免被读成断言失败。
function 参数错误(说明) {
    console.error(`[环境] ${说明}`);
    console.error(用法);
    process.exit(退出码.环境);
}

// 逐项识别参数，保证运行范围与等待上限都由使用者显式确定。
for (let 索引 = 0; 索引 < 参数.length; 索引 += 1) {
    const 当前 = 参数[索引]; // 当前参数项。
    if (当前 === "--页") {
        const 值 = 参数[(索引 += 1)]; // 页面路径子串。
        if (!值) 参数错误("--页 缺少路径子串");
        选项.页过滤 = 值;
    } else if (当前 === "--超时") {
        const 值 = Number(参数[(索引 += 1)]); // 单页超时毫秒数。
        if (!Number.isFinite(值) || 值 <= 0) 参数错误("--超时需要正整数毫秒");
        选项.超时毫秒 = 值;
    } else if (当前 === "--浮岛像素") {
        选项.浮岛像素 = true;
    } else if (当前 === "--像素筛选") {
        选项.像素筛选 = 参数[(索引 += 1)] ?? "";
    } else if (当前 === "--像素对照") {
        选项.像素对照 = 参数[(索引 += 1)] ?? "normal";
        if (!["normal", "card-solid", "patch-solid", "all-solid", "overlap"].includes(选项.像素对照)) 参数错误("未知像素对照");
    } else if (当前 === "--像素基线") {
        选项.像素基线 = 参数[(索引 += 1)] ?? "";
        if (!选项.像素基线) 参数错误("--像素基线缺少目录");
    } else if (当前 === "--稳态对照") {
        选项.稳态对照 = true;
    } else if (当前 === "--自然帧") {
        选项.自然帧 = true;
    } else if (当前 === "--verbose") {
        选项.详细 = true;
    } else {
        参数错误(`未知参数：${当前}`);
    }
}

// 有界等待：只等一次，不做重试。
function 等待(毫秒) {
    return new Promise((完成) => setTimeout(完成, 毫秒));
}

// 带超时的 HTTP 请求，避免环境问题把运行器吊死；用 node:http 直连本机端点，绕开 fetch 的系统代理解析。
function 取文本(地址, 超时毫秒 = 5000, 方法 = "GET") {
    return new Promise((完成, 失败) => {
        const 目标 = new URL(地址); // 解析主机、端口与路径。
        const 请求 = request(
            { host: 目标.hostname, port: 目标.port, path: `${目标.pathname}${目标.search}`, method: 方法, timeout: 超时毫秒 },
            (响应) => {
                let 体 = ""; // 累积响应体。
                响应.setEncoding("utf8");
                响应.on("data", (块) => (体 += 块));
                响应.on("end", () => {
                    if (响应.statusCode >= 400) 失败(new Error(`HTTP ${响应.statusCode} ${目标.pathname} ${体.slice(0, 200)}`));
                    else 完成(体);
                });
            },
        ); // 本机 CDP HTTP 请求。
        请求.on("timeout", () => 请求.destroy(new Error(`请求超时 ${超时毫秒}ms`)));
        请求.on("error", 失败);
        请求.end();
    });
}

// 查找本机 Edge，DESKTOP_TOOLS_EDGE 可覆盖；路径统一转成正斜杠绝对路径，便于 spawn 直接启动。
function 定位浏览器() {
    const 覆盖路径 = process.env.DESKTOP_TOOLS_EDGE; // 使用者指定的 Edge 路径。
    const 安装目录 = ["ProgramFiles(x86)", "ProgramFiles"].map((键) => process.env[键]); // 常见安装根目录。
    const 候选列表 = [覆盖路径, ...安装目录.map((目录) => (目录 ? join(目录, "Microsoft", "Edge", "Application", "msedge.exe") : undefined))]
        .filter(Boolean)
        .map((路径) => resolve(路径).replaceAll("\\", "/")); // 绝对路径 + 正斜杠。
    const 命中 = 候选列表.find((路径) => existsSync(路径)); // 实际存在的 Edge 可执行文件。
    if (!命中) {
        throw new Error(覆盖路径 ? `DESKTOP_TOOLS_EDGE 指向的文件不存在：${覆盖路径}` : "未找到 Edge，可用环境变量 DESKTOP_TOOLS_EDGE 指定 msedge.exe 路径");
    }
    return 命中;
}

// 检查本机 1420 端口是否有进程监听；127.0.0.1 无代理，可直接依赖最小原语。
async function 检查端口() {
    const 连接 = connect({ host: "127.0.0.1", port: 1420 }); // 探测连接。
    try {
        await new Promise((完成, 失败) => {
            连接.once("connect", 完成);
            连接.once("error", 失败);
            setTimeout(() => 失败(new Error("超时")), 3000);
        });
    } catch {
        throw new Error("127.0.0.1:1420 未监听，请先在另一个终端运行 npm run dev 后重试");
    } finally {
        连接.destroy();
    }
}

// 结束一个实例的浏览器进程树并删除它的临时目录；失败不改变判定。
async function 结束实例(实例) {
    if (实例.pid) {
        try {
            // 按 pid 结束本实例的进程树（Edge 有大量子进程），不触碰使用者日常打开的 Edge 与其他实例。
            execFileSync("taskkill", ["/PID", String(实例.pid), "/T", "/F"], { stdio: "ignore" });
        } catch {}
        实例.pid = 0;
    }
    // Windows 下文件句柄释放稍慢，有限次数重试删除，仍在 2 秒内收敛。
    for (let 尝试 = 0; 尝试 < 10; 尝试 += 1) {
        try {
            await rm(实例.临时目录, { recursive: true, force: true });
            return;
        } catch {
            await 等待(200);
        }
    }
}

// 启动一个新的无头 Edge 实例并等待 CDP 端点应答；每页一个实例，返回本次实例的句柄。
async function 启动浏览器(可执行文件) {
    const 临时目录 = await mkdtemp(join(tmpdir(), "desktop-tools-browser-")); // 本次实例独立 user-data-dir，避免影响日常 Edge 与其他页。
    const 端点文件 = join(临时目录, "DevToolsActivePort"); // Edge 写给自己的调试端口文件。
    const 日志路径 = join(临时目录, "edge.log"); // stderr 日志，仅用于失败时给出证据。
    // detached 启动是实测必需：同一脚本里普通 spawn 时 Edge 只建立 Crashpad 就退出，detached 则稳定写出端点文件。
    const 浏览器进程 = spawn(
        可执行文件,
        [
            "--headless=new", // 新版无头模式，保留完整渲染管线。
            "--remote-debugging-port=0", // 端口交给系统分配，避免与日常 Edge 撞车。
            `--user-data-dir=${临时目录}`, // 隔离配置目录。
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-gpu",
            "--allow-running-insecure-content",
        ],
        { stdio: ["ignore", "ignore", "pipe"], windowsHide: true, detached: true },
    ); // 无头浏览器进程，独立进程组便于按进程树结束。
    浏览器进程.unref();
    const 实例 = { pid: 浏览器进程.pid, 临时目录, 端点地址: "" }; // 本次实例的可清理句柄。
    // stderr 实时落盘，失败时可读末尾片段作为证据。
    let 错误输出 = ""; // 累积的 stderr（最多保留 2 万字符）。
    浏览器进程.stderr.on("data", (块) => {
        错误输出 += 块.toString("utf8");
        if (错误输出.length > 20000) 错误输出 = 错误输出.slice(-20000);
    });
    const 写日志 = () => writeFile(日志路径, 错误输出).catch(() => {});

    // 有界等待端点文件出现：单次尝试，不重连、不重试页面。
    const 截止 = Date.now() + 端点超时毫秒; // 端点发现截止时刻。
    while (Date.now() < 截止) {
        await 等待(轮询间隔毫秒);
        if (existsSync(端点文件)) {
            const 端口 = (await readFile(端点文件, "utf8").catch(() => "")).split(/\r?\n/)[0].trim(); // 首行即端口。
            if (/^\d+$/.test(端口)) {
                实例.端点地址 = `127.0.0.1:${端口}`;
                return 实例;
            }
        }
    }
    await 写日志();
    let 尾部 = 错误输出.slice(-800); // 失败现场。
    if (!尾部) 尾部 = await readFile(日志路径, "utf8").then((值) => 值.slice(-800)).catch(() => "");
    await 结束实例(实例); // 启动失败也要回收进程与临时目录。
    throw new Error(`等待 Edge 写出调试端点超时（${端点超时毫秒 / 1000} 秒），pid=${浏览器进程.pid ?? "无"}，临时目录文件=${(await readdir(临时目录).catch(() => [])).join(",")}${尾部 ? "\nEdge stderr：\n" + 尾部 : ""}`);
}

// 建立一条页面级 CDP 连接，只连一次，不做重连。
function 连接页面(地址, onEvent) {
    return new Promise((完成, 失败) => {
        const 套接字 = new WebSocket(地址); // Node 24 全局 WebSocket。
        const 待响应 = new Map(); // 命令 id 到回调。
        let 序号 = 0; // 命令自增 id。
        const 发送 = (方法, 参数) =>
            new Promise((解析, 拒绝) => {
                const id = (序号 += 1); // 当前命令 id。
                const 定时器 = setTimeout(() => {
                    待响应.delete(id);
                    拒绝(new Error(`CDP 命令 ${方法} 超时`));
                }, 5000); // 单条命令时限。
                待响应.set(id, {
                    解析: (值) => {
                        clearTimeout(定时器);
                        解析(值);
                    },
                    拒绝: (错误) => {
                        clearTimeout(定时器);
                        拒绝(错误);
                    },
                });
                套接字.send(JSON.stringify({ id, method: 方法, params: 参数 }));
            });
        套接字.addEventListener("message", (事件) => {
            const 消息 = JSON.parse(事件.data);
            if (消息.method) onEvent?.(消息); // CDP 消息。
            const 等待项 = 消息.id === undefined ? undefined : 待响应.get(消息.id); // 对应命令回调。
            if (!等待项) return;
            待响应.delete(消息.id);
            if (消息.error) 等待项.拒绝(new Error(消息.error.message));
            else 等待项.解析(消息.result);
        });
        套接字.addEventListener("error", () => {
            if (套接字.readyState !== WebSocket.OPEN) {
                // 握手失败后套接字不会自动关闭，必须主动 close，否则事件循环被占住不退出。
                套接字.close();
                失败(new Error("CDP WebSocket 连接失败"));
            }
        });
        套接字.addEventListener("open", () => 完成({ 发送, 关闭: () => 套接字.close() }));
    });
}

// 只读取页面 #result、脚本异常与渲染状态，不在页面内做任何操作。
const 读取脚本 = `(() => {
    const 节点 = document.getElementById("result"); // 断言页结论节点，缺省表示文档尚未就绪。
    // Vite 构建或运行错误浮层：出现即表示页面整体崩溃，不能算渲染成功。
    const 浮层 = document.querySelector("vite-error-overlay");
    const 内容元素 = [...(document.body ? document.body.getElementsByTagName("*") : [])].filter((元素) => 元素.tagName !== "VITE-ERROR-OVERLAY").length;
    return { 文本: 节点 ? 节点.textContent || "" : "", 报错: window.__lastError ? String(window.__lastError) : "", 元素数: 内容元素, 浮层: Boolean(浮层), 可见: document.visibilityState === "visible" };
})()`; // Runtime.evaluate 使用的只读表达式；元素数排除错误浮层后供检查页判断是否已渲染，可见性供前台切换判断。

// 打开断言页并读取结论；检查页只要求加载，不要求 PASS/FAIL。
async function 打开并读取(相对路径, 是否检查页, 端点地址) {
    const 开始 = Date.now(); // 单页计时起点。
    const 地址 = `${开发服务器地址}/${相对路径}`; // 断言页 URL。
    let 页面; // 新建标签页的 CDP 信息。
    let 连接; // 页面级 CDP 连接。
    let 结论; // 判定结果。

    // 页面自身写出的结论：PASS 原样保留，FAIL 带上脚本异常。
    const 判定 = (当前) =>
        当前.文本.startsWith("PASS")
            ? { 状态: "PASS", 文本: 当前.文本 }
            : { 状态: "FAIL", 文本: 当前.报错 ? `${当前.文本}\n__lastError: ${当前.报错}` : 当前.文本 };
    // 只读页面 #result 与脚本异常，不在页面内做任何操作。
    const 读取 = async () => {
        const 结果集 = await 连接.发送("Runtime.evaluate", { expression: 读取脚本, returnByValue: true, awaitPromise: false });
        if (结果集.exceptionDetails) return { 文本: `FAIL: 页面脚本异常 ${结果集.exceptionDetails.text}`, 报错: "" };
        return 结果集.result.value;
    };

    try {
        const 新建 = await 取文本(`http://${端点地址}/json/new?about:blank`, 10000, "PUT"); // 新建空标签页，随后页内导航。
        页面 = JSON.parse(新建);
        if (!页面.webSocketDebuggerUrl) throw new Error(`新建标签页失败：${新建.slice(0, 200)}`);
        连接 = await 连接页面(页面.webSocketDebuggerUrl);
        await 连接.发送("Runtime.enable", {});
        // 断言页用 requestAnimationFrame 等布局提交，隐藏标签页的 rAF 不触发，页面会停在 RUNNING。
        // 先在空标签页里建立会话并完成前台切换，再在页内导航到断言页：这样页面从一开始就是可见状态启动。
        await 连接.发送("Page.enable", {}).catch(() => {});
        await 连接.发送("Emulation.setFocusEmulationEnabled", { enabled: true }).catch(() => {});
        await 连接.发送("Page.bringToFront", {}).catch(() => {});
        await 连接.发送("Page.navigate", { url: 地址 }).catch(() => {});
        // 轮询开始时再切一次前台：新建标签页在前序页面仍在跑时会滞留在后台。
        const 等待可见 = async () => {
            for (let i = 0; i < 20; i += 1) {
                await 连接.发送("Page.bringToFront", {}).catch(() => {});
                if ((await 读取()).可见 !== false) return true;
                await 等待(轮询间隔毫秒);
            }
            return false;
        }; // 有界重试，不做无上限轮询。
        if (!(await 等待可见())) throw new Error("标签页始终处于隐藏状态，无法采集结论");
        // 有界轮询 #result：出现 PASS/FAIL 立即判定；检查页渲染出内容即结束，全程只读不改页面。
        const 截止 = 开始 + 选项.超时毫秒; // 本页判定截止时刻。
        let 最后文本 = "RUNNING"; // 最近一次读到的文本。
        while (Date.now() < 截止) {
            const 当前 = await 读取();
            最后文本 = 当前.文本 || 最后文本;
            if (/^(PASS|FAIL)/.test(当前.文本)) {
                结论 = 判定(当前);
                break;
            }
            // 检查页没有结论文本：排除错误浮层后仍有内容才视为渲染完成；浮层在场直接判失败。
            if (是否检查页 && 当前.浮层) {
                结论 = { 状态: "FAIL", 文本: "检查页出现 vite-error-overlay：页面整体渲染失败" };
                break;
            }
            if (是否检查页 && 当前.元素数 > 1) {
                结论 = { 状态: "INSPECT", 文本: `检查页：已渲染 ${当前.元素数} 个元素，不输出 PASS/FAIL` };
                break;
            }
            await 等待(轮询间隔毫秒);
        }
        if (!结论) {
            结论 = 是否检查页
                ? { 状态: "FAIL", 文本: `检查页超时 ${选项.超时毫秒}ms 仍未渲染任何内容` }
                : { 状态: "FAIL", 文本: `超时 ${选项.超时毫秒}ms 仍未出现结论，最后文本：${最后文本}` };
        }
    } catch (错误) {
        结论 = { 状态: "FAIL", 文本: `运行异常：${错误.message}` };
    } finally {
        try {
            if (连接) 连接.关闭();
        } catch {}
        // 只关闭本次打开的标签页；关闭失败不影响判定。
        try {
            if (页面?.id) await 取文本(`http://${端点地址}/json/close/${页面.id}`, 5000);
        } catch {}
    }

    return { 页: 相对路径, ...结论, 耗时毫秒: Date.now() - 开始 };
}

// 打印汇总表，返回是否全部通过：PASS、INSPECT 与 SKIP 都算通过，FAIL 算失败。
function 汇总(记录) {
    const 失败数 = 记录.filter((项) => 项.状态 === "FAIL").length; // 未通过页数。
    const 检查数 = 记录.filter((项) => 项.状态 === "INSPECT").length; // 只核对加载的页数。
    const 跳过数 = 记录.filter((项) => 项.状态 === "SKIP").length; // 真机专用页数。
    for (const 项 of 记录) {
        console.log(`${项.状态.padEnd(7)} ${String(项.耗时毫秒).padStart(6)}ms  ${项.页}`);
        if (项.状态 === "FAIL") console.log(`        ${项.文本.split("\n").slice(0, 20).join("\n        ")}`);
        else if (选项.详细) console.log(`        ${项.文本}`);
    }
    console.log(`\n共 ${记录.length} 页：断言通过 ${记录.length - 失败数 - 检查数 - 跳过数}，检查页 ${检查数}，真机专用跳过 ${跳过数}，失败 ${失败数}`); // 检查页只核对加载，真机专用页不计失败。
    return 失败数 === 0;
}

// 自然rAF仅由显式诊断开关运行，截取Page.screencastFrame并保留真实时间间隔，不声称达到60fps。
async function 自然帧取证() {
    await 检查端口();
    const 目录=resolve('build/validation/浮岛修复/02b-切态接缝',new Date().toISOString().replace(/[:.]/g,'-')+'-natural');
    await mkdir(目录,{recursive:true});
    let 实例,连接;
    const images=[];
    const evaluate=async(expression)=>{
        const value=await 连接.发送('Runtime.evaluate',{expression,returnByValue:true,awaitPromise:true});
        if(value.exceptionDetails)throw new Error(value.exceptionDetails.text);
        return value.result.value;
    };
    try {
        实例=await 启动浏览器(定位浏览器());
        const page=JSON.parse(await 取文本(`http://${实例.端点地址}/json/new?about:blank`,10000,'PUT'));
        连接=await 连接页面(page.webSocketDebuggerUrl,(event)=>{
            if(event.method!=='Page.screencastFrame')return;
            images.push(event.params);
            void 连接.发送('Page.screencastFrameAck',{sessionId:event.params.sessionId}).catch(()=>{});
        });
        await 连接.发送('Page.enable',{});
        await 连接.发送('Emulation.setDeviceMetricsOverride',{width:1280,height:800,deviceScaleFactor:1,mobile:false});
        await 连接.发送('Page.bringToFront',{});
        await 连接.发送('Page.navigate',{url:`${开发服务器地址}/src/features/island/islandFillet.browser.html?mode=natural-seam`});
        const deadline=Date.now()+10000;
        while(Date.now()<deadline && !(await evaluate('Boolean(window.__islandNaturalCapture)')))await 等待(20);
        const box=await evaluate('document.querySelector(".island-card").getBoundingClientRect().toJSON()');
        await 连接.发送('Page.startScreencast',{format:'png',everyNthFrame:1});
        await 连接.发送('Input.dispatchMouseEvent',{type:'mouseMoved',x:0,y:0});
        await 等待(100);
        await 连接.发送('Input.dispatchMouseEvent',{type:'mouseMoved',x:box.x+box.width/2,y:box.y+box.height/2});
        await 等待(900);
        await 连接.发送('Input.dispatchMouseEvent',{type:'mouseMoved',x:700,y:500});
        await 等待(900);
        await 连接.发送('Page.stopScreencast',{});
        const records=await evaluate('window.__islandNaturalCapture.stop();window.__islandNaturalCapture.records');
        const timeline=[];
        for(let i=0;i<images.length;i++) {
            const name=`frame-${String(i).padStart(3,'0')}.png`;
            await writeFile(join(目录,name),Buffer.from(images[i].data,'base64'));
            timeline.push({file:name,...images[i].metadata});
        }
        const summary={records,timeline,intervals:timeline.slice(1).map((frame,i)=>(frame.timestamp-timeline[i].timestamp)*1000),maxGeometryDelta:Math.max(...records.map((frame)=>frame.delta))};
        await writeFile(join(目录,'自然帧时间线.json'),JSON.stringify(summary,null,2));
        console.log(`自然帧证据：${目录}，CSS记录${records.length}，合成帧${images.length}，最大几何差${summary.maxGeometryDelta} DIP`);
        return records.length>1 && images.length>1 && summary.maxGeometryDelta<0.05;
    } finally { if(连接)连接.关闭();if(实例)await 结束实例(实例); }
}

// 只在显式--浮岛像素入口执行：保留同一受控帧PNG与像素剖面，其他断言页的默认执行完全不变。
async function 浮岛像素回归() {
    await 检查端口();
    const 目录 = resolve("build/validation/浮岛修复/02b-切态接缝", new Date().toISOString().replace(/[:.]/g, "-"));
    await mkdir(目录, { recursive: true });
    const 记录 = [];
    const 期望帧 = expectedPixelFrames({ steady: 选项.稳态对照, filter: 选项.像素筛选 });
    const 保存记录 = async (record) => {
        记录.push(record);
        await appendFile(join(目录, "逐帧.jsonl"), JSON.stringify(record) + "\n");
        if (记录.length % 64 === 0) console.log(`已记录${记录.length}帧：${record.name}`);
    };
    let 实例, 页面, 连接;
    let currentPageKey = "";
    const sources = ["src/features/island/island.tsx", "src/features/island/island.css", "src/features/island/islandGeometry.ts"];
    await writeFile(join(目录, "源码指纹.json"), JSON.stringify(await Promise.all(sources.map(async (path) => ({ path, sha256: createHash("sha256").update(await readFile(path)).digest("hex") }))), null, 2));
    const 读值 = async (expression) => {
        const value = await 连接.发送("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true });
        if (value.exceptionDetails) throw new Error(value.exceptionDetails.text + " " + JSON.stringify(value.exceptionDetails.exception));
        return value.result.value;
    };
    try {
        实例 = await 启动浏览器(定位浏览器());
        页面 = JSON.parse(await 取文本(`http://${实例.端点地址}/json/new?about:blank`, 10000, "PUT"));
        连接 = await 连接页面(页面.webSocketDebuggerUrl);
        await 连接.发送("Page.enable", {});
        await 连接.发送("Emulation.setFocusEmulationEnabled", { enabled: true });
        await 连接.发送("Page.bringToFront", {});
        for (const expected of 期望帧) {
            const { form, edge, dpr, direction, requestedMs: elapsed, background, name } = expected;
            let frame;
            try {
            const pageKey = `${form}/${edge}/${dpr}/${direction}`;
            if (pageKey !== currentPageKey) {
                await 连接.发送("Emulation.setDeviceMetricsOverride", { width: 1920, height: 1080, deviceScaleFactor: dpr, mobile: false });
                const url = `${开发服务器地址}/src/features/island/islandFillet.browser.html?mode=seam-frame&form=${form}&edge=${edge}&direction=${direction}&elapsed=${elapsed}&background=${background}`;
                await 连接.发送("Page.navigate", { url });
                const deadline = Date.now() + 选项.超时毫秒;
                let ready = false;
                while (Date.now() < deadline) {
                    const value = await 读值("({frame:window.__islandSeamFrame,result:document.querySelector('#result')?.textContent})");
                    if (value?.frame) { ready = true; break; }
                    if (value?.result?.startsWith("FAIL")) throw new Error(`${name}: ${value.result}`);
                    await 等待(20);
                }
                if (!ready) throw new Error(`${name}: 冻结帧等待超时`);
                currentPageKey = pageKey;
            }
            frame = await 读值(`(async()=>({...(await window.__islandSeamAdvance(${elapsed},${JSON.stringify(background)})),mode:${JSON.stringify(expected.mode)},dpr:window.devicePixelRatio}))()`);
            const validity = validatePixelFrame(frame, expected);
            if (!validity.valid) {
                await 保存记录({ name, ...frame, scanned: false, reason: validity.reasons.join("；") });
                continue;
            }
            if (选项.像素对照 === "overlap") {
                await 读值(`(() => {
                    const node = document.querySelector('.island__surface');
                    const style = getComputedStyle(node);
                    const positions = style.maskPosition.split(',').map((part) => part.trim().split(/\\s+/).map(parseFloat));
                    const sizes = style.maskSize.split(',').map((part) => part.trim().split(/\\s+/).map(parseFloat));
                    const vertical = ['left','right'].includes(${JSON.stringify(edge)});
                    for(let i=1;i<Math.min(3,sizes.length);i++) {
                        const axis=vertical?1:0;
                        sizes[i][axis]+=2;
                        if(i%3===2) positions[i][axis]-=2;
                    }
                    node.style.maskPosition=positions.map((pair)=>pair.map((value)=>value+'px').join(' ')).join(',');
                    node.style.maskSize=sizes.map((pair)=>pair.map((value)=>value+'px').join(' ')).join(',');
                })()`);
            } else if (选项.像素对照 !== "normal") {
                await 读值(`(() => {
                    const node = document.querySelector('.island__surface');
                    const style = getComputedStyle(node);
                    const image = style.maskImage;
                    const end = image.indexOf('\")') + 2;
                    const card = ${JSON.stringify(选项.像素对照)} === 'patch-solid' ? image.slice(0,end) : 'linear-gradient(#000,#000)';
                    const patches = ${JSON.stringify(选项.像素对照)} === 'card-solid' ? image.slice(end) : ', '+Array(style.maskSize.split(',').length-1).fill('linear-gradient(#000 0px,#000 1px)').join(',');
                    node.style.maskImage = card + patches;
                })()`);
            }
            const x = Math.max(0, Math.floor((frame.card.x - 20) / 4) * 4);
            const y = Math.max(0, Math.floor((frame.card.y - 20) / 4) * 4);
            const clip = { x, y, width: Math.ceil((frame.card.x + frame.card.width + 20 - x) / 4) * 4, height: Math.ceil((frame.card.y + frame.card.height + 20 - y) / 4) * 4, scale: 1 };
            const image = await 连接.发送("Page.captureScreenshot", { format: "png", clip, captureBeyondViewport: true });
            const filename = `${选项.像素对照}_${name.replaceAll("/", "_")}.png`;
            await writeFile(join(目录, filename), Buffer.from(image.data, "base64"));
            // 浏览器自身解码CDP截图，避免额外PNG依赖；只返回接缝附近的有界像素剖面。
            let steadyBaseline = null;
            if (选项.稳态对照) {
                // 重构t5原三层mask字面值：同一稳态DOM/几何/背景仅切换新旧mask，RGBA逐字节比较。
                await 读值(`(() => {
                    const node=document.querySelector('.island__surface');
                    const style=getComputedStyle(node), image=style.maskImage;
                    if(image==='none')return;
                    const first=image.slice(0,image.indexOf('\\")')+2);
                    const sizes=style.maskSize.split(',').slice(1,3).map((part)=>Math.min(...part.trim().split(/\\s+/).map(parseFloat)));
                    const centers={top:['0% 100%','100% 100%'],bottom:['0% 0%','100% 0%'],left:['100% 0%','100% 100%'],right:['0% 0%','0% 100%']}[${JSON.stringify(edge)}];
                    node.style.maskImage=first+', '+sizes.map((radius,i)=>'radial-gradient(circle at '+centers[i]+',transparent '+(radius-.5)+'px,#000 '+radius+'px)').join(', ');
                })()`);
                steadyBaseline=(await 连接.发送('Page.captureScreenshot',{format:'png',clip,captureBeyondViewport:true})).data;
                await writeFile(join(目录,filename.replace('.png','_legacy-steady.png')),Buffer.from(steadyBaseline,'base64'));
            }
            const baseline = 选项.像素基线 ? (await readFile(join(选项.像素基线, filename))).toString('base64') : null;
            const scan = await 读值(`(${像素扫描.toString()})(${JSON.stringify({ png: image.data, frame, clip, dpr, baseline, steadyBaseline, validity })})`);
            if(选项.稳态对照) {
                const layers=frame.maskSize.split(',').length;
                if(frame.form==='notch' && layers!==4)throw new Error('稳态负角必须为原三层+根尺寸单路径');
                await 读值(`(() => {const node=document.querySelector('.island__surface');node.style.maskImage=${JSON.stringify(frame.maskImage)};document.querySelector('main.island').dataset.dragging='true';})()`);
                const dragNew=(await 连接.发送('Page.captureScreenshot',{format:'png',clip,captureBeyondViewport:true})).data;
                await 读值(`(() => {const node=document.querySelector('.island__surface');const image=getComputedStyle(node).maskImage;if(image==='none')return;const first=image.slice(0,image.indexOf('\\")')+2);const sizes=getComputedStyle(node).maskSize.split(',').slice(1,3).map((p)=>Math.min(...p.trim().split(/\\s+/).map(parseFloat)));const centers={top:['0% 100%','100% 100%'],bottom:['0% 0%','100% 0%'],left:['100% 0%','100% 100%'],right:['0% 0%','0% 100%']}[${JSON.stringify(edge)}];node.style.maskImage=first+', '+sizes.map((r,i)=>'radial-gradient(circle at '+centers[i]+',transparent '+(r-.5)+'px,#000 '+r+'px)').join(', ');})()`);
                const dragOld=(await 连接.发送('Page.captureScreenshot',{format:'png',clip,captureBeyondViewport:true})).data;
                const dragCompare=await 读值(`(${像素扫描.toString()})(${JSON.stringify({png:dragNew,frame,clip,dpr,steadyBaseline:dragOld,validity})})`);
                scan.dragSteadyChangedBytes=dragCompare.steadyChangedBytes;
                scan.dragSteadyOutsideChanged=dragCompare.outsideBandChanged+dragCompare.outsideNewCoverage;
                await writeFile(join(目录,filename.replace('.png','_drag-steady.png')),Buffer.from(dragNew,'base64'));
                await 读值(`(() => { delete document.querySelector('main.island').dataset.dragging; document.querySelector('.island__surface').style.maskImage=${JSON.stringify(frame.maskImage)}; })()`);
            }
            const record = { name, variant: 选项.像素对照, screenshot: join(目录, filename), ...frame, ...scan };
            await 保存记录(record);
            if (选项.像素对照 !== "normal") await 读值(`(() => { const node=document.querySelector('.island__surface'); node.style.maskImage=${JSON.stringify(frame.maskImage)};node.style.maskPosition=${JSON.stringify(frame.maskPosition)};node.style.maskSize=${JSON.stringify(frame.maskSize)}; })()`);
            } catch (error) {
                await 保存记录({ name, mode: expected.mode, form, edge, dpr, direction, elapsed, ...frame, scanned: false, reason: `像素驱动停止：${error.message}` });
                break; // CDP/截图失败时不反复驱动坏页面；缺少的期望帧由同一门禁明确判失败。
            }
        }
    } finally {
        if (连接) 连接.关闭();
        if (实例) await 结束实例(实例);
        await writeFile(join(目录, "像素剖面.json"), JSON.stringify(记录, null, 2));
        console.log(`像素证据：${目录}`);
    }
    const summary = summarizePixelFrames(记录, 期望帧);
    const { failures, ...counts } = summary;
    await writeFile(join(目录, "门禁汇总.json"), JSON.stringify({ ...counts, failures: failures.map((failure) => ({ name: failure.expected?.name || failure.frame?.name, actualElapsed: failure.frame?.actualElapsed, reasons: failure.reasons })) }, null, 2));
    console.log(`像素帧${记录.length}，接缝失败${failures.length}；capsule无外凸接缝，不以未复现代替真机通过。`);
    console.log(`未扫描 ${summary.unscannedCount} / 期望 ${summary.expectedCount}；实际扫描 ${summary.scannedCount}，记录 ${summary.recordCount}`);
    console.log(`样本构成：动画 ${summary.sampleCounts.animation}，终态 ${summary.sampleCounts.terminal}；沿用较早paint ${summary.sampleCounts.reusedPaint}（动画子集）`);
    if (!summary.passed) {
        console.log(summary.firstFailure);
        console.log(`数量问题：${summary.countReasons.join("；") || "无"}；原因汇总：${JSON.stringify(summary.reasonCounts)}`);
    }
    return summary.passed;
}

// 接缝位于圆弧与卡片共同内部，而非轮廓外沿；白/深背景分别重建同一单层mask覆盖alpha。
async function 像素扫描({ png, frame, clip, dpr, baseline, steadyBaseline, validity }) {
    if (!validity?.valid) return { scanned: false, reason: validity?.reasons?.join("；") || "帧有效性未验证" };
    const image = new Image();
    image.src = `data:image/png;base64,${png}`;
    await image.decode();
    const canvas = document.createElement("canvas");
    canvas.width = image.width; canvas.height = image.height;
    const context = canvas.getContext("2d");
    context.drawImage(image, 0, 0);
    const pixels = context.getImageData(0, 0, image.width, image.height).data;
    const numbers = (frame.form === "capsule" ? frame.cardBackground : frame.surfaceColor).match(/[\d.]+/g).map(Number);
    const expected = numbers[3] ?? 1;
    const box = frame.card;
    const horizontal = frame.edge === "top" || frame.edge === "bottom";
    const joins = horizontal ? [box.x, box.x + box.width] : [box.y, box.y + box.height];
    const rows = [];
    // 回弹可能把补面余量夹到小于12；扫描线必须仍位于理想圆弧的实心内部，不能把真实外轮廓当亮缝。
    const radii = frame.maskSize.split(',').slice(1,3).map((part) => Math.min(...part.trim().split(/\s+/).map(parseFloat)));
    const radius = frame.form === "capsule" ? 12 : Math.min(...radii);
    const insets = [Math.min(2, radius / 6), Math.min(4, radius / 4), Math.min(6, radius / 3)];
    for (const join of joins) for (const inset of insets) {
        const fixed = horizontal ? (frame.edge === "top" ? box.y + inset : box.y + box.height - inset) : (frame.edge === "left" ? box.x + inset : box.x + box.width - inset);
        const base = horizontal ? clip.x : clip.y;
        const center = (join - base) * dpr;
        const values = [];
        for (let axis = Math.floor(center - dpr); axis <= Math.ceil(center + dpr); axis++) {
            const css = base + (axis + 0.5) / dpr;
            if (Math.abs(css - join) > 1) continue;
            const px = horizontal ? axis : Math.floor((fixed - clip.x) * dpr);
            const py = horizontal ? Math.floor((fixed - clip.y) * dpr) : axis;
            const offset = (py * image.width + px) * 4;
            const rgb = [...pixels.slice(offset, offset + 3)];
            const alpha = rgb.reduce((sum, color, channel) => sum + (frame.background[channel] - color) / (frame.background[channel] - numbers[channel]), 0) / 3;
            values.push({ css, rgb, alpha });
        }
        rows.push({ join, inset, values });
    }
    if (frame.form === "capsule") {
        // Capsule没有fillet；只在四条直边内部4px做连续表面对照，不把其外轮廓的正常AA算接缝。
        rows.length = 0;
        for (const [x,y] of [[box.x+4,box.y+box.height/2],[box.x+box.width-4,box.y+box.height/2],[box.x+box.width/2,box.y+4],[box.x+box.width/2,box.y+box.height-4]]) {
            const offset=(Math.floor((y-clip.y)*dpr)*image.width+Math.floor((x-clip.x)*dpr))*4;
            const rgb=[...pixels.slice(offset,offset+3)];
            const alpha=rgb.reduce((sum,color,channel)=>sum+(frame.background[channel]-color)/(frame.background[channel]-numbers[channel]),0)/3;
            rows.push({ location:[x,y], values:[{css:x,rgb,alpha}] });
        }
    }
    const centerOffset=(Math.floor((box.y+box.height/2-clip.y)*dpr)*image.width+Math.floor((box.x+box.width/2-clip.x)*dpr))*4;
    const singleSurfaceRgb=[...pixels.slice(centerOffset,centerOffset+3)];
    let maxSingleSurfaceDelta=0;
    let maxLightWidth = 0, maxDarkAlpha = 0, minAlpha = 1;
    for (const row of rows) {
        let width = 0;
        for (const value of row.values) {
            maxSingleSurfaceDelta=Math.max(maxSingleSurfaceDelta,...value.rgb.map((color,channel)=>Math.abs(color-singleSurfaceRgb[channel])));
            minAlpha = Math.min(minAlpha, value.alpha);
            maxDarkAlpha = Math.max(maxDarkAlpha, value.alpha - expected);
            width = value.alpha < expected - 0.06 ? width + 1 / dpr : 0;
            maxLightWidth = Math.max(maxLightWidth, width);
        }
    }
    let outsideBandChanged = 0, outsideBandMaxDelta = 0, outsideNewCoverage = 0, steadyChangedBytes = 0;
    if (baseline || steadyBaseline) {
        const reference = new Image();
        reference.src = `data:image/png;base64,${baseline || steadyBaseline}`;
        await reference.decode();
        if (reference.width !== image.width || reference.height !== image.height) throw new Error('基线截图尺寸不一致，不能比较轮廓');
        context.clearRect(0,0,image.width,image.height);
        context.drawImage(reference,0,0);
        const before = context.getImageData(0,0,image.width,image.height).data;
        for(let py=0;py<image.height;py++) for(let px=0;px<image.width;px++) {
            const x=clip.x+(px+0.5)/dpr, y=clip.y+(py+0.5)/dpr;
            const normal = horizontal ? x : y;
            const along = horizontal ? y : x;
            const attached = horizontal ? (frame.edge==='top'?box.y:box.y+box.height) : (frame.edge==='left'?box.x:box.x+box.width);
            const inward = frame.edge==='top'||frame.edge==='left' ? along-attached : attached-along;
            const seamBand = frame.form==='notch' && joins.some((join)=>Math.abs(normal-join)<=2) && inward>=-1/dpr && inward<=Math.max(...radii)+1/dpr;
            const offset=(py*image.width+px)*4;
            if(steadyBaseline)steadyChangedBytes += [0,1,2,3].filter((channel)=>pixels[offset+channel]!==before[offset+channel]).length;
            if(seamBand)continue;
            const difference=Math.max(...[0,1,2].map((channel)=>Math.abs(pixels[offset+channel]-before[offset+channel])));
            outsideBandMaxDelta=Math.max(outsideBandMaxDelta,difference);
            if(difference>2)outsideBandChanged++;
            if([0,1,2].every((channel)=>Math.abs(before[offset+channel]-frame.background[channel])<=2) && [0,1,2].some((channel)=>Math.abs(pixels[offset+channel]-frame.background[channel])>2))outsideNewCoverage++;
        }
    }
    return { scanned: true, singleSurfaceRgb, maxSingleSurfaceDelta, expectedAlpha: expected, minAlpha, maxLightWidth, maxDarkAlpha, outsideBandChanged, outsideBandMaxDelta, outsideNewCoverage, steadyChangedBytes, steadyCompared:Boolean(steadyBaseline), bandDefinition:{normalHalfWidthCss:2,inwardStartCss:-1/dpr,inwardEndCss:Math.max(...radii)+1/dpr,clipHalfWidthCss:1+Math.max(.5,.5/dpr)}, baselineCompared:Boolean(baseline), rows };
}

// 清理被中断的运行遗留的无头 Edge：用户开着可见 Edge 窗口时跳过全局清理，本次实例仍按进程树回收。
function 清理遗留无头浏览器() {
    try {
        const 可见窗口数 = execFileSync("powershell", ["-NoProfile", "-Command", "(Get-Process msedge -ErrorAction SilentlyContinue | Where-Object { $_.MainWindowTitle } | Measure-Object).Count"], { encoding: "utf8", timeout: 15000, windowsHide: true }).trim(); // 用户可见 Edge 窗口数。
        if (可见窗口数 !== "0") return; // 用户正在用 Edge，交给实例回收处理自己的进程树。
        execFileSync("taskkill", ["/IM", "msedge.exe", "/F"], { stdio: "ignore", timeout: 15000 }); // 无可见窗口时清掉无头残留。
    } catch {
        // 清理失败不影响断言判定；本次实例的回收仍然执行。
    }
}

// 主流程：环境预检 → 逐页独立浏览器实例判定 → 汇总退出。
async function 主流程() {
    const 全部页 = [...断言页, ...真机专用页, ...检查页]; // 断言页 + 真机专用页 + 只核对加载的检查页。
    const 待跑 = 选项.页过滤 ? 全部页.filter((页) => 页.includes(选项.页过滤)) : 全部页; // 本次要跑的页面。
    if (待跑.length === 0) throw new Error(`没有匹配 --页 ${选项.页过滤} 的断言页`);
    await 检查端口();
    const 可执行文件 = 定位浏览器(); // Edge 可执行文件；找不到属于环境问题。
    const 记录 = []; // 逐页结果。

    // 串行执行，每页一个新实例：上一页的残留标签页、后台节流与渲染状态都不会影响下一页。
    for (const 页 of 待跑) {
        if (真机专用页.includes(页)) {
            const 跳过 = { 页, 状态: "SKIP", 文本: "真机专用：headless 下时序不成立，需在真实宿主中由用户操作确认", 耗时毫秒: 0 }; // 不计失败。
            记录.push(跳过);
            console.log(`${跳过.状态.padEnd(7)} ${String(跳过.耗时毫秒).padStart(6)}ms  ${页}`);
            continue;
        }
        const 开始 = Date.now(); // 本页实例从启动到回收的总耗时。
        let 实例; // 本页的浏览器实例。
        let 结果; // 本页判定结果。
        try {
            实例 = await 启动浏览器(可执行文件);
        } catch (错误) {
            // 浏览器起不来属于环境问题，与页面断言无关：立即整体退出，不把余下页面记成失败。
            console.error(`[环境] 启动浏览器失败：${错误.message}`);
            process.exit(退出码.环境);
        }
        try {
            结果 = await 打开并读取(页, 检查页.includes(页), 实例.端点地址);
        } catch (错误) {
            结果 = { 页, 状态: "FAIL", 文本: `运行异常：${错误.message}` };
        } finally {
            if (实例) await 结束实例(实例); // 判定完成立即回收进程树与临时目录。
        }
        结果.耗时毫秒 = Date.now() - 开始;
        记录.push(结果);
        console.log(`${结果.状态.padEnd(7)} ${String(结果.耗时毫秒).padStart(6)}ms  ${页}`);
    }

    return 汇总(记录);
}

// 启动与收尾各清理一次：覆盖被上游中断、实例树没来得及回收的运行。
清理遗留无头浏览器();
let 退出值 = 退出码.失败; // 默认按失败退出，异常路径改走环境问题退出码。
try {
    退出值 = (await (选项.浮岛像素 ? (选项.自然帧 ? 自然帧取证() : 浮岛像素回归()) : 主流程())) ? 退出码.通过 : 退出码.失败;
} catch (错误) {
    console.error(`[环境] ${错误.message}`);
    退出值 = 退出码.环境;
} finally {
    清理遗留无头浏览器();
}
process.exit(退出值);
