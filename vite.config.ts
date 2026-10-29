/**
 * ============================================================================
 * Vite 构建配置
 * ============================================================================
 *
 * 【Tauri 专属要点（不可随意改动）】
 *   1. `server.port = 1420` 与 `strictPort: true`
 *      —— 必须与 src-tauri/tauri.conf.json 的 `build.devUrl` 完全一致，
 *         否则 Tauri 窗口加载不到前端。
 *      `devUrl` 用 `127.0.0.1` 而不是 `localhost`：Windows 上 `localhost`
 *      会被解析成 IPv6 的 `::1`，Vite 随之只绑 IPv6，而 WebView2 走 IPv4
 *      就连不上 —— 表现为窗口白屏/黑屏（2026-10-29 实机踩中）。
 *   2. `clearScreen: false`
 *      —— 让 Rust 端的编译错误不被 Vite 清屏覆盖，便于排障。
 *   3. `envPrefix: ['VITE_', 'TAURI_']`
 *      —— Tauri 的环境变量（TAURI_PLATFORM 等）需要被注入到前端。
 *   4. `host: TAURI_DEV_HOST`
 *      —— 支持真机/局域网调试（移动端与远程设备场景）。
 *   5. `build.target`
 *      —— macOS/Linux 要求 'safari13'，Windows 用 'chrome105' 即可。
 * ============================================================================
 */

import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import tailwindcss from '@tailwindcss/postcss'
import { fileURLToPath, URL } from 'node:url'

// Tauri 在开发时会注入 TAURI_DEV_HOST（局域网调试用）
const host = process.env.TAURI_DEV_HOST

export default defineConfig({
  plugins: [vue()],

  // 路径别名：@ 指向 src，避免 ../../../ 地狱
  resolve: {
    alias: {
      '@': fileURLToPath(new URL('./src', import.meta.url)),
    },
  },

  css: {
    postcss: {
      plugins: [tailwindcss()],
    },
  },

  // Tauri 期望前端固定运行在 1420 端口
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    // 显式绑 IPv4 回环：`false` 会绑到 `localhost`，Windows 上常只绑 `::1`，
    // WebView2 连 127.0.0.1 时直接失败（窗口空白）。与 devUrl 保持一致。
    host: host || '127.0.0.1',
    hmr: host
      ? {
          protocol: 'ws',
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 不要监听 Rust 端文件，避免 Rust 编译时触发前端 HMR 风暴
      ignored: ['**/src-tauri/**'],
    },
  },

  // 让 Tauri 的环境变量可被前端读取
  envPrefix: ['VITE_', 'TAURI_'],

  build: {
    // Windows 用 Edge WebView2（Chromium 105+）；macOS/Linux 回退 safari13
    target: process.env.TAURI_ENV_PLATFORM === 'windows' ? 'chrome105' : 'safari13',
    // 开发构建不压缩，便于调试
    minify: !process.env.TAURI_ENV_DEBUG ? 'esbuild' : false,
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
    // 输出目录必须与 tauri.conf.json 的 frontendDist 一致
    outDir: 'dist',
    emptyOutDir: true,
  },
})
