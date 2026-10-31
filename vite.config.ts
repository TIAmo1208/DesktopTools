import { defineConfig } from "vite";

// Vite 开发服务器配置：监视范围排除 build/，避免监听 Rust 构建产物时触发 EBUSY。
export default defineConfig({
    // 构建目录（Cargo target-dir 与前端产物）不参与热更新监听；正则同时匹配两种路径分隔符。
    server: {
        watch: { ignored: [/[\\/]build[\\/]/] },
    },
});
