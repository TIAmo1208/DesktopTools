import { cp, mkdir } from "node:fs/promises";

// 复制安装包中的运行资源到 Vite public，开发与正式构建使用相同本地地址。
const source = new URL("../node_modules/vditor/dist/", import.meta.url);
const destination = new URL("../public/vditor/dist/", import.meta.url);
await mkdir(destination, { recursive: true });
for (const name of ["js", "css", "images"]) {
    await cp(new URL(`${name}/`, source), new URL(`${name}/`, destination), { recursive: true });
}
