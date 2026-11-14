// 把仓库 plugin/ 下的插件同步到开发运行的插件目录：仓库目录不会自动进入 build/target/debug/plugin。
import { cp, mkdir, readFile, readdir } from "node:fs/promises"; // 读取清单、列出目录并复制。
import { existsSync } from "node:fs"; // 快速判断清单与目标目录是否存在。
import { dirname, join, resolve } from "node:path"; // 路径拼接与仓库根推导。
import { fileURLToPath } from "node:url"; // 当前模块路径。

const 仓库根 = resolve(dirname(fileURLToPath(import.meta.url)), ".."); // scripts/ 的上一级。
const 默认源根 = join(仓库根, "plugin"); // 仓库内插件源。
const 默认目标根 = join(仓库根, "build", "target", "debug", "plugin"); // 开发运行的插件目录。
const 用法 = "用法：node scripts/同步插件.mjs [--dry-run] [--include-examples] [--source <目录>] [--target <目录>]"; // 参数错误时打印。

// 解析 manifest.ini 的 [plugin] 段；只取 name、author、id，值为首个等号后的原文。
export function 解析清单(文本) {
    const 结果 = {}; // 清单字段。
    let 区段 = ""; // 当前所在段。
    for (const 原始行 of 文本.replace(/^\uFEFF/, "").split(/\r?\n/)) {
        const 行 = 原始行.trim();
        if (!行 || 行.startsWith("#") || 行.startsWith(";")) continue;
        const 段匹配 = 行.match(/^\[(.+)\]$/);
        if (段匹配) {
            区段 = 段匹配[1].trim();
            continue;
        }
        if (区段 !== "plugin") continue;
        const 等号 = 行.indexOf("=");
        if (等号 < 0) continue;
        const 键 = 行.slice(0, 等号).trim();
        const 值 = 行
            .slice(等号 + 1)
            .trim()
            .replace(/^"(.*)"$/, "$1");
        if (键) 结果[键] = 值;
    }
    return 结果;
}

// 安装写入的目录名是「作者@名称」；没有作者时退回名称，与宿主的扫描规则一致。
export function 目标目录名(清单) {
    const 名称 = (清单.name ?? "").trim();
    if (!名称) throw new Error("清单缺少 name");
    const 作者 = (清单.author ?? "").trim();
    return 作者 ? `${作者}@${名称}` : 名称;
}

// 扫描目标目录现有清单，按 id 建立索引；同 id 的插件沿用已有目录名，避免出现两份注册。
export async function 扫描目标(目标根) {
    const 索引 = new Map(); // id → 已有目录路径。
    if (!existsSync(目标根)) return 索引;
    for (const 条目 of await readdir(目标根, { withFileTypes: true })) {
        if (!条目.isDirectory() || 条目.name.startsWith(".")) continue;
        const 清单路径 = join(目标根, 条目.name, "manifest.ini");
        if (!existsSync(清单路径)) continue;
        const 清单 = 解析清单(await readFile(清单路径, "utf8"));
        if (清单.id) 索引.set(清单.id, join(目标根, 条目.name));
    }
    return 索引;
}

// 生成同步计划：默认包含主题与功能插件；示例插件默认跳过，显式包含时按 示例/ 下一层目录处理。
export async function 生成同步计划({ 源根 = 默认源根, 目标根 = 默认目标根, 包含示例 = false } = {}) {
    const 已有 = await 扫描目标(目标根);
    const 候选目录 = [];
    for (const 条目 of await readdir(源根, { withFileTypes: true })) {
        if (!条目.isDirectory() || 条目.name.startsWith(".")) continue;
        if (条目.name === "示例") {
            if (!包含示例) continue;
            for (const 示例 of await readdir(join(源根, "示例"), { withFileTypes: true })) {
                if (示例.isDirectory() && !示例.name.startsWith(".")) 候选目录.push(join(源根, "示例", 示例.name));
            }
            continue;
        }
        候选目录.push(join(源根, 条目.name));
    }
    const 计划 = [];
    for (const 源目录 of 候选目录) {
        const 清单路径 = join(源目录, "manifest.ini");
        if (!existsSync(清单路径)) continue;
        const 清单 = 解析清单(await readFile(清单路径, "utf8"));
        const 目标目录 = (清单.id && 已有.get(清单.id)) || join(目标根, 目标目录名(清单));
        计划.push({ 源目录, 目标目录, 名称: 清单.name ?? 源目录.split(/[\\/]/).pop(), 是否新增: !existsSync(目标目录) });
    }
    return 计划.sort((左, 右) => 左.名称.localeCompare(右.名称, "zh"));
}

// 逐项复制并合并目录，保留目标里已有的其他文件；复制失败向上抛出。
async function 执行同步(计划) {
    for (const 项 of 计划) {
        await cp(项.源目录, 项.目标目录, { recursive: true, force: true });
        console.log(`[同步] ${项.名称} → ${项.目标目录}`);
    }
}

// 解析命令行；未知参数按环境问题退出，避免把用法错误当成同步失败。
function 读取参数(参数) {
    const 选项 = { 试运行: false, 包含示例: false, 源根: 默认源根, 目标根: 默认目标根 }; // 默认选项。
    for (let 索引 = 0; 索引 < 参数.length; 索引 += 1) {
        const 当前 = 参数[索引];
        if (当前 === "--dry-run") 选项.试运行 = true;
        else if (当前 === "--include-examples") 选项.包含示例 = true;
        else if (当前 === "--source") 选项.源根 = resolve(参数[(索引 += 1)] ?? "");
        else if (当前 === "--target") 选项.目标根 = resolve(参数[(索引 += 1)] ?? "");
        else if (当前 === "--help" || 当前 === "-h") {
            console.log(用法);
            process.exit(0);
        } else {
            console.error(`[环境] 未知参数：${当前}\n${用法}`);
            process.exit(2);
        }
    }
    return 选项;
}

// 只有直接运行时才执行同步，测试 import 不会改文件。
async function 主流程() {
    const 选项 = 读取参数(process.argv.slice(2));
    if (!existsSync(选项.源根)) {
        console.error(`[环境] 插件源目录不存在：${选项.源根}`);
        process.exit(2);
    }
    const 计划 = await 生成同步计划({ 源根: 选项.源根, 目标根: 选项.目标根, 包含示例: 选项.包含示例 });
    if (计划.length === 0) {
        console.log("[同步] 没有可同步的插件");
        return;
    }
    if (选项.试运行) {
        for (const 项 of 计划) console.log(`[计划] ${项.是否新增 ? "新增" : "更新"} ${项.名称} → ${项.目标目录}`);
        return;
    }
    await mkdir(选项.目标根, { recursive: true });
    await 执行同步(计划);
    console.log(`[同步] 完成 ${计划.length} 个插件 → ${选项.目标根}`);
}

// 入口判断：被测试 import 时不执行。
function 是入口() {
    return Boolean(process.argv[1]) && resolve(process.argv[1]) === fileURLToPath(import.meta.url);
}

if (是入口()) {
    主流程().catch((错误) => {
        console.error(`[同步] 失败：${错误.message}`);
        process.exit(1);
    });
}
