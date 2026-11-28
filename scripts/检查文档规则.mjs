// 文档规则门禁：扫描仓库文档的结构、引用与措辞，只报相对基线的**新增**违规。
// 存量违规登记在 文档规则基线.json；`--write-baseline` 用当前扫描结果重写基线。
import { existsSync, readFileSync, readdirSync, writeFileSync } from "node:fs"; // 文档枚举与基线读写。
import { dirname, join, relative, resolve } from "node:path"; // 路径解析。
import { fileURLToPath } from "node:url"; // 当前模块路径。

const 仓库根 = process.cwd();
// 参与检查的固定文档：常驻规则、领域事实与三份使用者文档。
const 固定文档 = ["AGENTS.md", "CONTEXT.md", "README.md", "src/README.md", "src-tauri/README.md"];
// 不参与检查的目录：本地临时区、agent 工作区、依赖与构建产物。
const 跳过目录 = new Set(["node_modules", "build", "dist", "target", ".scratch", ".agents", ".git"]);
// 无法判定的措辞：出现即视为条款不可执行。
const 不可判定措辞 = /应尽量|酌情|适当|必要时/g;
// ADR 文件名：四位编号加标题。
const ADR文件名 = /^(\d{4})-.+\.md$/;

// 递归收集 docs 下的 markdown；返回仓库相对路径。
function 收集文档(根, 目录, 结果) {
    for (const 项 of readdirSync(目录, { withFileTypes: true })) {
        if (跳过目录.has(项.name)) continue;
        const 绝对 = join(目录, 项.name);
        if (项.isDirectory()) {
            收集文档(根, 绝对, 结果);
        } else if (项.name.endsWith(".md")) {
            结果.push(relative(根, 绝对).split("\\").join("/"));
        }
    }
    return 结果;
}

// 读取一份文档的文本行；不存在时返回空数组。
function 读取行(根, 相对路径) {
    try {
        return readFileSync(resolve(根, 相对路径), "utf8").split(/\r?\n/);
    } catch {
        return [];
    }
}

// 链接里的百分号编码还原成可比较的路径；非法编码原样返回。
function 解码(目标) {
    try {
        return decodeURI(目标);
    } catch {
        return 目标;
    }
}

// 从 markdown 链接里取出相对目标：跳过外链、锚点、模板与站点绝对路径。
function 提取链接(行) {
    const 结果 = [];
    for (const 匹配 of 行.matchAll(/\[[^\]]*\]\(([^)]+)\)/g)) {
        let 目标 = 匹配[1].trim();
        if (目标.startsWith("<") && 目标.endsWith(">")) 目标 = 目标.slice(1, -1).trim();
        目标 = 目标.split(/\s+/)[0];
        const 去锚点 = 目标.split("#")[0];
        if (!去锚点) continue;
        if (/^(https?:|mailto:|data:|#)/i.test(去锚点)) continue;
        if (去锚点.startsWith("/") || /[{}*<>]/.test(去锚点)) continue;
        结果.push(解码(去锚点));
    }
    return 结果;
}

// 一条违规的稳定键：不含行号，避免无关编辑让基线失效。
function 记录违规(违规列表, 类别, 文件, 行号, 说明) {
    违规列表.push({ 类别, 文件, 行号, 说明, 键: `${类别}|${文件}|${说明}` });
}

// 检查 ADR 文件名格式、编号唯一性，以及每份 ADR 是否已登记进索引。
function 检查ADR(根, 违规列表) {
    const 目录 = resolve(根, "docs/adr");
    if (!existsSync(目录)) return;
    const 已用 = new Map();
    for (const 名 of readdirSync(目录).filter((项) => 项.endsWith(".md") && 项 !== "README.md").sort()) {
        const 匹配 = 名.match(ADR文件名);
        if (!匹配) {
            记录违规(违规列表, "ADR命名", `docs/adr/${名}`, 0, 名);
            continue;
        }
        if (已用.has(匹配[1])) {
            记录违规(违规列表, "ADR编号重复", `docs/adr/${名}`, 0, `编号 ${匹配[1]} 与 ${已用.get(匹配[1])} 重复`);
        } else {
            已用.set(匹配[1], 名);
        }
    }
    // 索引是唯一入口：新 ADR 漏登记时在这里失败，避免索引随目录增长失真。
    const 索引 = resolve(根, "docs/adr/README.md");
    if (!existsSync(索引)) return;
    const 索引文本 = readFileSync(索引, "utf8");
    for (const [编号, 名] of 已用) {
        if (!new RegExp(`(^|[^0-9])${编号}([^0-9]|$)`).test(索引文本)) {
            记录违规(违规列表, "ADR未登记索引", `docs/adr/${名}`, 0, 编号);
        }
    }
}

// 检查文档内相对链接的目标是否存在。
function 检查链接(根, 文档, 违规列表) {
    for (const 文件 of 文档) {
        读取行(根, 文件).forEach((行, 序号) => {
            for (const 目标 of 提取链接(行)) {
                if (!existsSync(resolve(根, dirname(文件), 目标))) {
                    记录违规(违规列表, "失效链接", 文件, 序号 + 1, 目标);
                }
            }
        });
    }
}

// 检查 README 不引用内部事实文档。
function 检查README(根, 文档, 违规列表) {
    for (const 文件 of 文档.filter((项) => 项.endsWith("README.md"))) {
        读取行(根, 文件).forEach((行, 序号) => {
            const 命中 = 行.match(/CONTEXT\.md|docs\/adr\//);
            if (命中) 记录违规(违规列表, "README引用内部文档", 文件, 序号 + 1, 命中[0]);
        });
    }
}

// 检查文档提到的 npm 脚本与 scripts 路径是否真实存在；带「尚未创建」等说明的行跳过。
function 检查交叉引用(根, 文档, 违规列表) {
    const 脚本 = JSON.parse(readFileSync(resolve(根, "package.json"), "utf8")).scripts ?? {};
    for (const 文件 of 文档) {
        const 行集 = 读取行(根, 文件);
        for (let 序号 = 0; 序号 < 行集.length; 序号 += 1) {
            const 行 = 行集[序号];
            for (const 命中 of 行.matchAll(/npm run ([\w:-]+)/g)) {
                if (!(命中[1] in 脚本)) 记录违规(违规列表, "失效命令引用", 文件, 序号 + 1, 命中[1]);
            }
            if (/尚未创建|待验证|未建立/.test(行)) continue;
            for (const 命中 of 行.matchAll(/`(scripts\/[^`\s]+\.(?:mjs|ps1|py))`/g)) {
                if (!existsSync(resolve(根, 命中[1]))) 记录违规(违规列表, "失效脚本引用", 文件, 序号 + 1, 命中[1]);
            }
        }
    }
}

// 检查不可判定措辞。
function 检查措辞(根, 文档, 违规列表) {
    for (const 文件 of 文档) {
        读取行(根, 文件).forEach((行, 序号) => {
            for (const 命中 of 行.matchAll(不可判定措辞)) {
                记录违规(违规列表, "不可判定措辞", 文件, 序号 + 1, 命中[0]);
            }
        });
    }
}

// 检查长文档可跳读：超过 120 行必须有二级或三级标题。
function 检查分节(根, 文档, 违规列表) {
    for (const 文件 of 文档) {
        const 行集 = 读取行(根, 文件);
        if (行集.length <= 120) continue;
        if (!行集.some((行) => /^#{2,3} /.test(行))) {
            记录违规(违规列表, "长文档未分节", 文件, 0, `${行集.length} 行`);
        }
    }
}

// 数值规则：代码常量是权威，文档里的同义数值必须与它一致。
// 正则里的每个捕获组相乘得到值，拼接成文档里应当出现的形式。
// 每条规则必须写明方向：描述性事实「文档跟随代码」，规范性约束「代码服从文档」。
const 数值规则 = [
    {
        名称: "loopback 缓冲条数",
        来源: "src-tauri/src/component_runtime/loopback.rs",
        正则: /BUFFER_LIMIT:\s*usize\s*=\s*(\d+)/g,
        文档: "CONTEXT.md",
        关键词: "原始缓冲",
        方向: "文档跟随代码",
        拼接: (值) => `${值} 条`,
    },
    {
        名称: "loopback 单条 body 上限",
        来源: "src-tauri/src/component_runtime/loopback.rs",
        正则: /BODY_LIMIT:\s*usize\s*=\s*(\d+)\s*\*\s*(\d+)/g,
        文档: "CONTEXT.md",
        关键词: "原始缓冲",
        方向: "文档跟随代码",
        拼接: (值) => `${值 / 1024 / 1024} MiB`,
    },
    {
        名称: "loopback 缓冲保留秒数",
        来源: "src-tauri/src/component_runtime/loopback.rs",
        正则: /RETENTION_SECONDS:\s*i64\s*=\s*(\d+)/g,
        文档: "CONTEXT.md",
        关键词: "原始缓冲",
        方向: "文档跟随代码",
        拼接: (值) => `${值} 秒`,
    },
    {
        名称: "浮岛视口带宽",
        来源: "src/features/island/islandGeometry.ts",
        正则: /ISLAND_VIEWPORT_WIDTH\s*=\s*(\d+)/g,
        文档: "CONTEXT.md",
        关键词: "视口带",
        方向: "文档跟随代码",
        拼接: (值) => `${值} DIP`,
    },
    {
        名称: "浮岛展开内容列宽",
        来源: "src/features/island/islandGeometry.ts",
        正则: /ISLAND_EXPANDED_COLUMN_WIDTH\s*=\s*(\d+)/g,
        文档: "docs/design/浮岛.md",
        关键词: "固定视口",
        方向: "文档跟随代码",
        拼接: (值) => `${值} DIP`,
    },
    {
        名称: "浮岛三态表面不透明度",
        来源: "src/features/island/island.css",
        正则: /--island-surface-alpha:\s*(\d+)%/g,
        文档: "CONTEXT.md",
        关键词: "表面不透明度",
        方向: "文档跟随代码",
        拼接: (值) => `${值}%`,
    },
];

// 检查文档里的数值是否与代码常量一致；关键词用来定位该数值所在的那一行。
function 检查数值(根, 违规列表) {
    for (const 规则 of 数值规则) {
        if (!规则.方向) {
            记录违规(违规列表, "数值规则失效", 规则.来源, 0, `${规则.名称}：缺少方向标注`);
            continue;
        }
        const 来源路径 = resolve(根, 规则.来源);
        if (!existsSync(来源路径)) {
            记录违规(违规列表, "数值规则失效", 规则.来源, 0, `${规则.名称}：来源文件不存在`);
            continue;
        }
        const 值集 = [...readFileSync(来源路径, "utf8").matchAll(规则.正则)].map((匹配) =>
            匹配.slice(1).filter(Boolean).reduce((积, 段) => 积 * Number(段), 1),
        );
        if (值集.length === 0) {
            记录违规(违规列表, "数值规则失效", 规则.来源, 0, `${规则.名称}：常量未找到`);
            continue;
        }
        const 期望 = 值集.map(规则.拼接);
        const 行集 = 读取行(根, 规则.文档);
        // 关键词可能落在术语标题行、数值在紧随的正文里，因此按关键词所在位置取一段窗口比对。
        const 窗口集 = [];
        行集.forEach((行, 序号) => {
            if (行.includes(规则.关键词)) 窗口集.push(行集.slice(序号, 序号 + 4).join("\n"));
        });
        if (窗口集.length === 0) {
            记录违规(违规列表, "数值规则失效", 规则.文档, 0, `${规则.名称}：文档里找不到关键词「${规则.关键词}」`);
            continue;
        }
        if (!窗口集.some((窗口) => 期望.every((片段) => 窗口.includes(片段)))) {
            // 失败信息必须指明改哪一侧，否则 agent 可能反向改代码去迁就过期文档。
            记录违规(违规列表, "数值不一致", 规则.文档, 0, `${规则.名称}：代码为 ${期望.join("、")}，按「${规则.方向}」处理`);
        }
    }
}

// 收集全部违规；纯函数，便于测试与基线比对。
export function collectViolations(根 = 仓库根) {
    const 违规列表 = [];
    const 文档 = 收集文档(根, resolve(根, "docs"), []);
    for (const 固定 of 固定文档) if (existsSync(resolve(根, 固定))) 文档.push(固定);
    检查ADR(根, 违规列表);
    检查链接(根, 文档, 违规列表);
    检查README(根, 文档, 违规列表);
    检查交叉引用(根, 文档, 违规列表);
    检查措辞(根, 文档, 违规列表);
    检查数值(根, 违规列表);
    检查分节(根, 文档, 违规列表);
    return 违规列表;
}

// 相对基线的新增违规，按类别分组；基线里的存量不阻塞。
export function newViolations(违规列表, 基线) {
    const 已登记 = new Set(Object.values(基线).flat());
    const 新增 = {};
    for (const 项 of 违规列表) {
        if (已登记.has(项.键)) continue;
        (新增[项.类别] ??= []).push(项);
    }
    return 新增;
}

// 把当前扫描结果写成基线文件。
function 写基线(违规列表) {
    const 基线 = {};
    for (const 项 of 违规列表) (基线[项.类别] ??= []).push(项.键);
    for (const 类别 of Object.keys(基线)) 基线[类别].sort((左, 右) => 左.localeCompare(右, "zh"));
    writeFileSync(resolve(仓库根, "scripts/文档规则基线.json"), `${JSON.stringify(基线, null, 4)}\n`, "utf8");
    console.log(`[文档规则] 基线已写入：${违规列表.length} 条存量违规`);
}

const 是入口 = Boolean(process.argv[1]) && resolve(process.argv[1]) === fileURLToPath(import.meta.url); // 被测试 import 时不执行。
if (是入口) {
    const 违规列表 = collectViolations();
    if (process.argv.includes("--write-baseline")) {
        写基线(违规列表);
    } else {
        const 基线路径 = resolve(仓库根, "scripts/文档规则基线.json");
        const 基线 = existsSync(基线路径) ? JSON.parse(readFileSync(基线路径, "utf8")) : {};
        const 新增 = newViolations(违规列表, 基线);
        const 类别集 = Object.keys(新增);
        if (类别集.length === 0) {
            console.log(`[文档规则] 通过：${违规列表.length} 条存量已登记，无新增`);
        } else {
            for (const 类别 of 类别集) {
                console.log(`[文档规则] ${类别}：${新增[类别].length} 条新增`);
                for (const 项 of 新增[类别]) console.log(`- ${项.文件}:${项.行号} ${项.说明}`);
            }
            process.exitCode = 1;
        }
    }
}
