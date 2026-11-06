import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";

// 插件根目录与宿主一致：默认是可执行文件同级的 plugin，Cargo target-dir 为 build/target，开发构建即 build/target/debug。
const pluginRoot = () => process.env.DESKTOP_TOOLS_PLUGIN_DIR ?? path.join(fileURLToPath(new URL(".", import.meta.url)), "build", "target", "debug", "plugin");

// 只放行 JS/CSS 与主题字体，与宿主 read_plugin_asset 的白名单一致。
const contentTypes = {
    ".js": "text/javascript; charset=utf-8",
    ".css": "text/css; charset=utf-8",
    ".woff2": "font/woff2",
    ".woff": "font/woff",
    ".ttf": "font/ttf",
    ".otf": "font/otf",
};

/**
 * 开发服务器把 /plugins/<相对路径> 映射到插件根。
 * 生产构建由 Rust 的自定义协议处理同一前缀；缺这条中间件时 vite 会把请求兜底成 index.html，
 * 插件脚本的动态 import 拿到 HTML 便无法解析，桌面格子会报「组件渲染失败」。
 */
const pluginAssetMiddleware = () => ({
    name: "desktop-tools-plugin-assets",
    configureServer(server) {
        server.middlewares.use("/plugins/", (request, response, next) => {
            const relative = decodeURIComponent((request.url ?? "").split("?")[0]); // 去掉查询串后按段解码。
            const segments = relative.split("/").filter(Boolean); // 空段忽略。
            // 拒绝穿越与绝对路径，和宿主的逐段校验保持一致。
            if (segments.some((segment) => segment === "." || segment === ".." || /[\\:\0]/.test(segment))) {
                response.statusCode = 404;
                return response.end("not found");
            }
            const file = path.join(pluginRoot(), ...segments);
            const type = contentTypes[path.extname(file)];
            // 未知扩展名或文件不存在都按 404 处理，不回落到 HTML。
            if (!type || !fs.existsSync(file) || !fs.statSync(file).isFile()) {
                response.statusCode = 404;
                return response.end("not found");
            }
            response.setHeader("content-type", type);
            fs.createReadStream(file).pipe(response);
        });
    },
});

// Vite 开发服务器配置：监视范围排除构建产物与第三方静态库，避免监听时触发 EBUSY。
export default defineConfig({
    // 构建目录（Cargo target-dir 与前端产物）和 prepare-vditor 生成的库都不参与热更新监听；
    // 后者由脚本按需重新生成，运行期不变更，监听到文件被占用会让整个开发服务器崩溃。
    plugins: [pluginAssetMiddleware()],
    server: {
        watch: { ignored: [/[\\/]build[\\/]/, /[\\/]public[\\/]vditor[\\/]/] },
    },
});
