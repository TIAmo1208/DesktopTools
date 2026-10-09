import { readFile } from "node:fs/promises";
import { resolve } from "node:path";

const root = resolve(import.meta.dirname, ".."); // 项目根目录。
const cargo = await readFile(resolve(root, "src-tauri/Cargo.toml"), "utf8"); // Rust 应用唯一版本来源。
const packageJson = JSON.parse(await readFile(resolve(root, "package.json"), "utf8")); // 前端工程元数据。
const tauri = JSON.parse(await readFile(resolve(root, "src-tauri/tauri.conf.json"), "utf8")); // Tauri 构建配置。
const cargoVersion = cargo.match(/^version\s*=\s*"([^"]+)"/m)?.[1]; // Cargo 应用版本。

if (!cargoVersion) throw new Error("src-tauri/Cargo.toml 未定义应用版本");
if (packageJson.version !== undefined) throw new Error("package.json 不得重复定义应用版本，请修改 src-tauri/Cargo.toml");
if (tauri.version !== undefined) throw new Error("src-tauri/tauri.conf.json 不得重复定义应用版本，请修改 src-tauri/Cargo.toml");

console.log(`应用版本：${cargoVersion}`);
