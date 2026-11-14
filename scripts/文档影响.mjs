// 按改动路径提示可能需同步的事实文档：只输出清单，不修改任何文件。
import { execFileSync } from "node:child_process"; // 读取 git 改动清单。
import { resolve } from "node:path"; // 仓库根。
import { fileURLToPath } from "node:url"; // 当前模块路径。

const 规则 = [
    { 匹配: /^src-tauri\/src\//, 文档: ["CONTEXT.md", "src-tauri/README.md", "docs/adr/"] },
    { 匹配: /^src\/contracts\//, 文档: ["CONTEXT.md", "src/README.md"] },
    { 匹配: /^src\/features\/island\//, 文档: ["CONTEXT.md", "docs/design/浮岛.md", "docs/adr/0036-浮岛作为独立呈现位.md"] },
    { 匹配: /^src\/features\/settings\//, 文档: ["docs/design/设置界面规范.md", "docs/design/视觉规范.md"] },
    { 匹配: /^src\/styles\//, 文档: ["docs/design/视觉规范.md", "docs/design/主题变量契约.md"] },
    { 匹配: /^plugin\//, 文档: ["plugin/README.md", "docs/design/主题变量契约.md"] },
    { 匹配: /^src\/(app|features|bridge|shared)\//, 文档: ["src/README.md"] },
    { 匹配: /^(scripts|\.github)\//, 文档: ["AGENTS.md", "docs/agents/构建运行.md"] },
];

// 纯映射：输入仓库相对路径，输出去重排序后的文档清单。
export function 影响文档(路径列表) {
    const 结果 = new Set();
    for (const 路径 of 路径列表) {
        for (const 规则项 of 规则) {
            if (规则项.匹配.test(路径)) for (const 文档 of 规则项.文档) 结果.add(文档);
        }
    }
    return [...结果].sort((左, 右) => 左.localeCompare(右, "zh"));
}

// 读取相对 HEAD 的改动；没有提交历史时按空清单处理，不阻塞。
function 读取改动() {
    try {
        return execFileSync("git", ["diff", "--name-only", "HEAD"], { encoding: "utf8" })
            .split(/\r?\n/)
            .map((行) => 行.trim())
            .filter(Boolean);
    } catch (错误) {
        console.error(`[环境] 读取 git 改动失败：${错误.message}`);
        process.exit(2);
    }
}

const 是入口 = Boolean(process.argv[1]) && resolve(process.argv[1]) === fileURLToPath(import.meta.url); // 被测试 import 时不执行。
if (是入口) {
    const 改动 = 读取改动();
    if (改动.length === 0) {
        console.log("[文档影响] 没有相对 HEAD 的改动");
    } else {
        const 文档 = 影响文档(改动);
        console.log(`[文档影响] ${改动.length} 个改动文件，可能需同步：`);
        for (const 项 of 文档) console.log(`- ${项}`);
        if (文档.length === 0) console.log("- （无匹配规则，按改动内容人工判断）");
        console.log("[提示] 只列候选，不改写；实际同步范围按事实文档规则判断。");
    }
}
