/**
 * 配置文件文本的语法校验（配置管理「编辑原始文件」保存前用）。
 *
 * 按扩展名选择校验器：json/jsonc 走容注释的 parseJsonc，yml/yaml 走
 * js-yaml；其他扩展名不校验（返回 null）。返回 null = 合法，
 * 返回字符串 = 面向用户的错误说明。
 */

import { load as yamlLoad } from 'js-yaml'
import { parseJsonc } from './jsonc'

export function validateConfigText(path: string, text: string): string | null {
  const lower = path.toLowerCase()
  if (lower.endsWith('.json') || lower.endsWith('.jsonc')) {
    if (!text.trim()) return null // 空文件交由用户意图决定,不拦
    try {
      parseJsonc(text)
      return null
    } catch (err) {
      return `JSON/JSONC 语法错误：${err instanceof Error ? err.message : String(err)}`
    }
  }
  if (lower.endsWith('.yml') || lower.endsWith('.yaml')) {
    if (!text.trim()) return null
    try {
      yamlLoad(text)
      return null
    } catch (err) {
      return `YAML 语法错误：${err instanceof Error ? err.message : String(err)}`
    }
  }
  return null
}
