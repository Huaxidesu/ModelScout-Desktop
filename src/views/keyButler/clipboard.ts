/**
 * ============================================================================
 * ModelScout —— Key 管家：剪贴板写入
 * ============================================================================
 *
 * 【为什么要三级降级】
 * 与 `ConfigPreview.vue` 里复制配置时是同一套考虑：
 *   1. `@tauri-apps/plugin-clipboard-manager` —— 原生实现，最可靠；
 *   2. `navigator.clipboard` —— 需要安全上下文（https / localhost），
 *      打包后的 WebView 里不保证可用；
 *   3. `document.execCommand('copy')` —— 最古老，但兼容面最广。
 *
 * 单独放在模块目录里而不是抽到公共 util：这是 Key 管家内部的使用细节，
 * 探测模块那边已经有一份等价实现，没必要为了去重去改公共文件。
 * ============================================================================
 */

import { writeText } from '@tauri-apps/plugin-clipboard-manager'

/** 把文本写入剪贴板；全部方案失败时抛错。 */
export async function copyText(text: string): Promise<void> {
  // 方案 1：Tauri 原生插件
  try {
    await writeText(text)
    return
  } catch (error) {
    console.warn('[ModelScout] 原生剪贴板不可用，尝试降级方案：', error)
  }

  // 方案 2：浏览器剪贴板 API
  try {
    if (navigator.clipboard && window.isSecureContext) {
      await navigator.clipboard.writeText(text)
      return
    }
  } catch (error) {
    console.warn('[ModelScout] navigator.clipboard 失败，继续降级：', error)
  }

  // 方案 3：临时 textarea + execCommand
  const textarea = document.createElement('textarea')
  textarea.value = text
  textarea.style.position = 'fixed'
  textarea.style.opacity = '0'
  textarea.style.pointerEvents = 'none'
  document.body.appendChild(textarea)
  try {
    textarea.select()
    const ok = document.execCommand('copy')
    if (!ok) throw new Error('execCommand("copy") 返回 false')
  } finally {
    document.body.removeChild(textarea)
  }
}
