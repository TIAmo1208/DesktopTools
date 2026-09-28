// 根据锁定依赖整理随安装包交付的许可声明，不修改依赖源码。
import { execFileSync } from "node:child_process";
import { mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), ".."); // 仓库根目录。
const lock = JSON.parse(readFileSync(join(root, "package-lock.json"), "utf8")); // npm 锁定依赖。
const metadata = JSON.parse(
    execFileSync(
        "cargo",
        [
            "metadata",
            "--manifest-path",
            "src-tauri/Cargo.toml",
            "--format-version",
            "1",
            "--locked",
            "--offline",
            "--filter-platform",
            "x86_64-pc-windows-msvc",
        ],
        { cwd: root, encoding: "utf8", maxBuffer: 16 * 1024 * 1024 },
    ),
); // 当前 Windows 构建依赖，包含构建工具的许可声明。
const packages = metadata.packages
    .filter((item) => item.source)
    .map((item) => ({
        name: `Rust: ${item.name}@${item.version}`,
        directory: dirname(item.manifest_path),
        license: item.license,
        licenseFile: item.license_file,
    })); // 待整理的 Rust 依赖。

for (const [path, item] of Object.entries(lock.packages)) {
    if (!path || item.dev) continue;
    packages.push({ name: `npm: ${path}@${item.version}`, directory: join(root, path), license: item.license });
}

const sections = ["第三方许可证\n\n以下声明来自当前锁定依赖；包括构建依赖，不表示每个依赖均进入最终二进制。\n"]; // 随包文本内容。
for (const item of packages.sort((left, right) => left.name.localeCompare(right.name))) {
    const files = readdirSync(item.directory, { withFileTypes: true })
        .filter((entry) => entry.isFile() && /^(license|licence|copying|notice|copyright)([._-]|$)/i.test(entry.name))
        .map((entry) => join(item.directory, entry.name)); // 包根目录的上游许可文件。
    if (item.licenseFile && !files.includes(join(item.directory, item.licenseFile))) files.push(join(item.directory, item.licenseFile));
    sections.push(`\n${"=".repeat(72)}\n${item.name}\n许可证标识：${item.license ?? "见上游许可文件"}\n`);
    for (const file of files.sort()) sections.push(readFileSync(file, "utf8"));
    if (!files.length) sections.push("本依赖未在包根目录附带许可正文，请参阅上游项目及许可证标识。\n");
}

const output = join(root, "build/licenses"); // 本次构建生成的许可目录。
mkdirSync(output, { recursive: true });
writeFileSync(join(output, "第三方许可证.txt"), sections.join("\n"), "utf8");
console.info(`INFO 已整理 ${packages.length} 项第三方依赖许可声明。`);
