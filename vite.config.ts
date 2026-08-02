import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri 开发约定：固定端口 1420，frontendDist 输出到 ../src（与 tauri.conf.json 对齐）
export default defineConfig({
  plugins: [react()],
  // 前端源码在 src/ 下，入口 index.html 也在那里
  root: "src",
  // 防止 Vite 混淆 Tauri 的环境变量
  envPrefix: ["VITE_", "TAURI_"],
  build: {
    // 输出到项目根的 dist/（tauri.conf.json 的 frontendDist 指到这里）
    outDir: "../dist",
    emptyOutDir: true,
    target: "es2021",
    minify: "esbuild",
    sourcemap: false,
  },
  server: {
    port: 1420,
    strictPort: true,
    host: "localhost",
  },
});
