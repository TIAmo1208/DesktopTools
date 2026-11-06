// 无头运行真实组件浏览器断言页：每个页面一个独立的 Edge headless 实例 + CDP 读取 #result，输出 PASS/FAIL；样张页记 INSPECT，真机专用页记 SKIP。
import { execFileSync, spawn } from "node:child_process"; // 进程控制：结束残留 Edge 与启动无头 Edge。
import { connect } from "node:net"; // 探测 127.0.0.1:1420 是否监听。
import { mkdtemp, readFile, readdir, rm, writeFile } from "node:fs/promises"; // 临时 user-data-dir、日志与清理。
import { existsSync } from "node:fs"; // 探测 Edge 可执行文件与端口文件。
import { tmpdir } from "node:os"; // 系统临时目录。
import { join, resolve } from "node:path"; // 路径拼接与绝对路径归一。
import { request } from "node:http"; // 直连本机 CDP 端点，不受系统代理影响。
import { 断言页, 真机专用页, 检查页 } from "./断言页目录.mjs"; // 断言页、真机专用页与只核对加载的检查页清单。

const 开发服务器地址 = "http://127.0.0.1:1420"; // 由 npm run dev 提供的断言页服务地址。
const 轮询间隔毫秒 = 100; // 每页读取 #result 的固定间隔，不引入常驻轮询。
const 默认超时毫秒 = 20000; // 单页从打开到出现结论的上限；4 秒内无结论基本就是页面缺依赖或脚本异常。
const 端点超时毫秒 = 30000; // 等待 Edge 写出调试端点的上限；实测首次启动 1 秒内即可写出，留足慢机器余量。

// 退出码语义：0 全部通过，1 存在失败页，2 环境问题。
const 退出码 = { 通过: 0, 失败: 1, 环境: 2 }; // 与设计文档一致的退出码表。

// 解析命令行参数，未识别参数按环境问题（用法错误）退出，不抛未捕获堆栈。
const 参数 = process.argv.slice(2); // 原始命令行参数。
const 选项 = { 超时毫秒: 默认超时毫秒, 页过滤: "", 详细: false }; // 运行选项。
const 用法 = "用法：node scripts/跑断言页.mjs [--页 <路径子串>] [--超时 30000] [--verbose]"; // 参数错误时打印的用法。

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
function 连接页面(地址) {
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
            const 消息 = JSON.parse(事件.data); // CDP 消息。
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

let 退出值 = 退出码.失败; // 默认按失败退出，异常路径改走环境问题退出码。
try {
    退出值 = (await 主流程()) ? 退出码.通过 : 退出码.失败;
} catch (错误) {
    console.error(`[环境] ${错误.message}`);
    退出值 = 退出码.环境;
}
process.exit(退出值);
