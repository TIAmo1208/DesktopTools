// 以工作树 build/tmp 作为 TMP 启动子命令：Rust 的 C 依赖只认 TMP，且新 shell 不继承上一次调用的环境。
import { mkdirSync } from "node:fs"; // 确保 build/tmp 存在，否则 TMP 指向不存在的目录仍会失败。
import { spawn } from "node:child_process"; // 启动目标命令并转发退出码。
import { dirname, join, resolve } from "node:path"; // 由脚本位置推导仓库根。
import { fileURLToPath } from "node:url"; // 当前模块的文件路径。

const 仓库根 = resolve(dirname(fileURLToPath(import.meta.url)), ".."); // scripts/ 的上一级即工作树根。
const 用法 = "用法：node scripts/with-tmp.mjs <命令> [参数...]"; // 参数缺失时打印。

// 计算工作树内的临时目录；单独导出以便测试与复用。
export function 临时目录(根 = 仓库根) {
    return join(根, "build", "tmp");
}

// 在基础环境上只覆盖 TMP：文档明确 TEMP 不能替代 TMP，避免顺带改变其他临时文件位置。
export function 运行环境(根 = 仓库根, 基础 = process.env) {
    return { ...基础, TMP: 临时目录(根) };
}

// 拼接 Windows 下经 shell 执行的命令行；含空白或引号的参数补引号，避免路径被拆开。
export function 组装命令(命令, 参数 = [], 平台 = process.platform) {
    return [命令, ...参数].map((项) => (平台 === "win32" && /[\s"]/.test(项) ? `"${项.replaceAll('"', '\\"')}"` : 项)).join(" ");
}

// 只有直接运行本文件时才执行主流程，被测试 import 时不启动子命令。
function 是入口() {
    return Boolean(process.argv[1]) && resolve(process.argv[1]) === fileURLToPath(import.meta.url);
}

// 设置 TMP 后启动命令；stdin/stdout/stderr 全部继承，退出码原样返回。
function 主流程() {
    const 参数 = process.argv.slice(2);
    if (参数.length === 0 || 参数[0] === "--help" || 参数[0] === "-h") {
        console.error(`[环境] 缺少要执行的命令\n${用法}`);
        process.exit(2);
    }
    const [命令, ...命令参数] = 参数;
    try {
        mkdirSync(临时目录(), { recursive: true });
    } catch (错误) {
        console.error(`[环境] 无法创建临时目录：${错误.message}`);
        process.exit(2);
    }
    console.log(`[TMP] ${临时目录()}`);
    const 子进程 =
        process.platform === "win32"
            ? spawn(组装命令(命令, 命令参数), { cwd: process.cwd(), env: 运行环境(), stdio: "inherit", shell: true })
            : spawn(命令, 命令参数, { cwd: process.cwd(), env: 运行环境(), stdio: "inherit" });
    for (const 信号 of ["SIGINT", "SIGTERM"]) {
        process.on(信号, () => 子进程.kill(信号));
    }
    子进程.on("error", (错误) => {
        console.error(`[环境] 启动失败：${错误.message}`);
        process.exit(2);
    });
    子进程.on("exit", (退出码, 信号) => {
        if (信号) process.exit(1);
        process.exit(退出码 ?? 1);
    });
}

if (是入口()) 主流程();
