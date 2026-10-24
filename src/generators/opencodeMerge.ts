/**
 * OpenCode 配置的一键填写合并：把生成的 provider 条目并入用户已有的
 * opencode.json / opencode.jsonc。
 *
 * 三种落盘形态，对应三类文件状态：
 *
 *   1. create  —— 文件不存在 / 空白 / 整体就是个 `{}`：
 *      直接写入完整配置（renderOpenCodeJsonc，带 $schema 与头注释）。
 *      「用户没有模型配置时，点击后直接在空文件补全」即此分支。
 *
 *   2. splice  —— 文件已有内容：在**原文**上做最小替换/插入
 *      （等价于把「片段」粘贴到正确位置），用户手写的注释全部保留：
 *        - 顶层已有 "provider"：provider 表里同名键 → 原位替换该键的值；
 *          无同名键 → 在 provider 对象末尾插入新键值；
 *        - 顶层没有 "provider"：在根对象末尾插入 `"provider": { ... }`。
 *
 *   3. rewrite —— splice 无法安全进行（结构出乎扫描器预料）但内容
 *      能被 parseJsonc 解析：退回对象级合并后整体重写（注释会丢失，
 *      结果里带 warning，界面明示，用户在对比预览里确认后才落盘）。
 *
 * 任何形态都不抛异常之外的意外：解析失败抛 MergeError，由界面转为
 * 「无法自动合并」的提示，绝不半写。
 */

import {
  JsoncScanner,
  findRootObject,
  isEmptyJsoncObject,
  parseJsonc,
  type MemberSpan,
} from '@/utils/jsonc'
import { buildOpenCodeConfig, renderOpenCodeJsonc, type GenerateContext } from './opencode'

/** 合并结果 */
export interface OpenCodeMergeResult {
  /** 落盘形态（见文件头注释） */
  mode: 'create' | 'splice' | 'rewrite'
  /** 合并后的完整文件文本 */
  text: string
  /** 需要向用户明示的注意事项（空数组 = 无） */
  warnings: string[]
}

/** 无法合并时抛出；message 面向用户（中文） */
export class MergeError extends Error {
  constructor(message: string) {
    super(message)
    this.name = 'MergeError'
  }
}

/**
 * 把生成的 provider 并入已有（或空缺的）opencode 配置文本。
 *
 * @param existingText 磁盘上的原文件内容；空字符串表示文件不存在
 * @param context      生成上下文（与预览区共用同一份）
 */
export function mergeOpenCodeIntoText(
  existingText: string,
  context: GenerateContext,
): OpenCodeMergeResult {
  // ---- 形态 1：没有有效配置 → 写完整文件 ----
  if (existingText.trim() === '' || isEmptyJsoncObject(existingText)) {
    return {
      mode: 'create',
      text: ensureTrailingNewline(renderOpenCodeJsonc(context)),
      warnings: [],
    }
  }

  // 先确保内容可解析（splice 和 rewrite 都依赖这一点）
  let parsed: unknown
  try {
    parsed = parseJsonc(existingText)
  } catch (err) {
    throw new MergeError(
      `现有文件不是合法的 JSON/JSONC（${err instanceof Error ? err.message : String(err)}），` +
        '无法自动合并。请先修复该文件，或改用手动粘贴片段。',
    )
  }
  if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) {
    throw new MergeError('现有文件的顶层不是 JSON 对象，无法合并 provider 片段。')
  }

  // ---- 形态 2：原文拼接 ----
  try {
    return spliceIntoExisting(existingText, context)
  } catch {
    // ---- 形态 3：对象级合并重写 ----
    return rewriteWholeFile(parsed as Record<string, unknown>, context)
  }
}

// ---- 形态 2 的实现 ----

/** 在原文上做最小替换/插入；结构不符合预期时抛错（由上层降级） */
function spliceIntoExisting(
  text: string,
  context: GenerateContext,
): OpenCodeMergeResult {
  const scanner = new JsoncScanner(text)
  const rootOpen = findRootObject(text)
  if (rootOpen < 0) {
    throw new Error('root object not found')
  }
  const rootClose = scanner.findMatchingBrace(rootOpen)
  const rootMembers = scanner.listMembers(rootOpen)

  const generated = buildOpenCodeConfig(context)
  const providerKey = Object.keys(generated.provider)[0]
  const providerValue = generated.provider[providerKey]

  // ---- 情况 A：顶层已有 "provider" ----
  const providerMember = rootMembers.get('provider')
  if (providerMember) {
    if (text[providerMember.valueStart] !== '{') {
      throw new Error('"provider" 的值不是对象')
    }
    const providerClose = scanner.findMatchingBrace(providerMember.valueStart)
    const providerMembers = scanner.listMembers(providerMember.valueStart)

    const existing = providerMembers.get(providerKey)
    if (existing) {
      // A-1 同名 provider：原位替换它的值。缩进跟随该成员所在行的行首，
      // 保证替换块与文件原有排版一致。
      const keyIndent = lineIndentOf(text, existing.keyStart)
      const renderedValue = renderValueAt(providerValue, keyIndent)
      const next =
        text.slice(0, existing.valueStart) + renderedValue + text.slice(existing.valueEnd)
      return { mode: 'splice', text: ensureTrailingNewline(next), warnings: [] }
    }

    // A-2 无同名 provider：插到 provider 对象末尾（闭括号前）
    const insertText = buildMemberInsert(
      scanner,
      providerMembers,
      providerClose,
      providerKey,
      providerValue,
    )
    const next =
      text.slice(0, providerClose) + insertText + text.slice(providerClose)
    return { mode: 'splice', text: ensureTrailingNewline(next), warnings: [] }
  }

  // ---- 情况 B：顶层没有 "provider" → 在根对象末尾插入整个成员 ----
  const insertText = buildMemberInsert(
    scanner,
    rootMembers,
    rootClose,
    'provider',
    providerValue,
  )
  const next = text.slice(0, rootClose) + insertText + text.slice(rootClose)
  return { mode: 'splice', text: ensureTrailingNewline(next), warnings: [] }
}

/** 取 offset 所在行的行首空白 */
function lineIndentOf(text: string, offset: number): string {
  const lineStart = text.lastIndexOf('\n', offset) + 1
  const match = text.slice(lineStart, offset).match(/^[ \t]*$/)
  return match ? match[0] : ''
}

/**
 * 把值渲染成「放在 `键: ` 之后」的多行文本，缩进跟随 `keyIndent`：
 * 首行顶格（紧跟着冒号），其余行整体加 keyIndent —— stringify 自带的
 * 2 空格内部缩进恰好让一级字段落在 keyIndent+2、闭合括号回到 keyIndent，
 * 与文件原有排版对齐。
 */
function renderValueAt(value: unknown, keyIndent: string): string {
  const raw = JSON.stringify(value, null, 2).split('\n')
  return raw.map((line, i) => (i === 0 ? line : keyIndent + line)).join('\n')
}

/**
 * 构造「插入到对象闭括号前」的成员文本（含前导逗号与缩进换行）。
 *
 * 逗号判断必须显式做：JSONC 允许尾逗号，若原对象最后一个成员后已有
 * 逗号，再无脑补一个会形成 `,,`（空元素），连 JSONC 都无法解析。
 *   - 对象无成员 → 不带逗号；
 *   - 有成员且最后一个成员后已有尾逗号 → 不带逗号；
 *   - 其余 → 带逗号。
 *
 * 成员缩进取闭括号所在行的行首空白 + 2 空格；拿不到时退回 4 空格
 * （OpenCode 常见排版）。
 */
function buildMemberInsert(
  scanner: JsoncScanner,
  members: Map<string, MemberSpan>,
  objectClose: number,
  key: string,
  value: unknown,
): string {
  let comma = ''
  if (members.size > 0) {
    // Map 保留插入序（listMembers 按出现顺序填充），取最后一个成员
    let last: MemberSpan | undefined
    for (const span of members.values()) last = span
    if (last) {
      const afterLast = scanner.skipInsignificant(last.valueEnd)
      const hasTrailingComma = scanner.source[afterLast] === ','
      comma = hasTrailingComma ? '' : ','
    }
  }

  // 闭括号所在行的行首缩进
  const text = scanner.source
  const lineStart = text.lastIndexOf('\n', objectClose) + 1
  const indentMatch = text.slice(lineStart, objectClose).match(/^[ \t]*/)
  const closeIndent = indentMatch ? indentMatch[0] : ''
  const memberIndent = closeIndent ? closeIndent + '  ' : '    '

  const rendered = renderValueAt(value, memberIndent)
  return `${comma}\n${memberIndent}"${escapeJsonKey(key)}": ${rendered}`
}

/** 转义对象键里的特殊字符（与 escapeJsonString 同规则，键一般不含特殊字符） */
function escapeJsonKey(key: string): string {
  return key
    .replace(/\\/g, '\\\\')
    .replace(/"/g, '\\"')
    .replace(/[\u0000-\u001f]/g, (ch) => {
      const code = ch.charCodeAt(0).toString(16).padStart(4, '0')
      return `\\u${code}`
    })
}

// ---- 形态 3 的实现 ----

/** 对象级合并后整体重写（保留其他顶层键，注释会丢失） */
function rewriteWholeFile(
  parsed: Record<string, unknown>,
  context: GenerateContext,
): OpenCodeMergeResult {
  const generated = buildOpenCodeConfig(context)

  const providers =
    typeof parsed.provider === 'object' && parsed.provider !== null && !Array.isArray(parsed.provider)
      ? (parsed.provider as Record<string, unknown>)
      : {}
  // 同名键替换、其余保留 —— 与 splice 的语义一致
  const merged: Record<string, unknown> = {
    ...parsed,
    provider: { ...providers, ...generated.provider },
  }

  return {
    mode: 'rewrite',
    text: ensureTrailingNewline(JSON.stringify(merged, null, 2)),
    warnings: [
      '现有文件的结构超出拼接式合并的能力范围，已改为整体重写：文件里的注释会丢失，其他配置项保留。',
    ],
  }
}

/** 保证文本以单个换行结尾 */
function ensureTrailingNewline(text: string): string {
  return text.endsWith('\n') ? text : `${text}\n`
}
