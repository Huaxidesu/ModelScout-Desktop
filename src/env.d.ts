/**
 * Vite 环境变量的类型声明。
 * 与 vite.config.ts 中的 `envPrefix: ['VITE_', 'TAURI_']` 配套。
 */
/// <reference types="vite/client" />

declare module '*.vue' {
  import type { DefineComponent } from 'vue'
  const component: DefineComponent<Record<string, unknown>, Record<string, unknown>, unknown>
  export default component
}

interface ImportMetaEnv {
  readonly TAURI_ENV_PLATFORM?: string
  readonly TAURI_ENV_DEBUG?: string
  readonly TAURI_ENV_ARCH?: string
  readonly TAURI_ENV_FAMILY?: string
}

interface ImportMeta {
  readonly env: ImportMetaEnv
}
