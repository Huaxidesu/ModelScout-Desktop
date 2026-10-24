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

/** 合并选项 */
export interface OpenCodeMergeOptions {
  /**
   * 写在 provider 条目上方的一行基础信息注释（名字 · 模型数 · 日期）。
   * 仅 JSONC 语义下有意义；同名替换时若上一行已是 ModelScout 标记注释，
   * 原位更新而不是追加（重复填写不堆积）。
   */
  providerComment?: string
}

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
  options: OpenCodeMergeOptions = {},
): OpenCodeMergeResult {
  // ---- 形态 1：没有有效配置 → 写完整文件 ----
  if (existingText.trim() === '' || isEmptyJsoncObject(existingText)) {
    const body = renderOpenCodeJsonc(context)
    const text = options.providerComment
      ? `// ${options.providerComment}
${body}`
      : body
    return {
      mode: 'create',
      text: ensureTrailingNewline(text),
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
    return spliceIntoExisting(existingText, context, options)
  } catch {
    // ---- 形态 3：对象级合并重写 ----（JSON.stringify 不含注释，注释选项被丢弃）
    return rewriteWholeFile(parsed as Record<string, unknown>, context)
  }
}

/**
 * 在指定成员键上方放置/更新一行 ModelScout 标记注释。
 *
 * 从键所在行向上回溯（越过空行与其他注释行）：先遇到的 ModelScout
 * 标记注释被**原位替换**（重复填写不堆积）。回溯遇到普通代码行时，
 * 再向上一行看一眼 —— 标记可能挂在「外层容器成员」（如 `provider`）
 * 的上方，中间隔着一行容器的键名行。
 */
function withMarkerComment(text: string, keyStart: number, comment: string): string {
  const replaceLine = (lineStartIndex: number, lineEndIndex: number): string => {
    const target = text.slice(lineStartIndex, lineEndIndex)
    const oldIndent = target.match(/^[ \t]*/)?.[0] ?? ''
    return text.slice(0, lineStartIndex) + `${oldIndent}// ${comment}` + text.slice(lineEndIndex)
  }

  const lineOf = (endExclusive: number): { start: number; end: number; line: string } => {
    const start = text.lastIndexOf('\n', Math.max(endExclusive - 1, 0)) + 1
    return { start, end: endExclusive, line: text.slice(start, endExclusive) }
  }

  let cursor = text.lastIndexOf('\n', keyStart) + 1 // 键所在行的行首
  const insertStart = cursor
  const indent = text.slice(insertStart, keyStart)

  for (let i = 0; i < 6 && cursor > 0; i++) {
    const prev = lineOf(cursor - 1)
    if (prev.line.trim() === '') {
      cursor = prev.start
      continue
    }
    if (/^\s*\/\/.*ModelScout/.test(prev.line)) {
      return replaceLine(prev.start, prev.end)
    }
    if (/^\s*\/\//.test(prev.line)) {
      cursor = prev.start
      continue
    }
    // 普通代码行：再向上一行确认是否是挂在容器上方的旧标记
    if (prev.start > 0) {
      const above = lineOf(prev.start - 1)
      if (/^\s*\/\/.*ModelScout/.test(above.line)) {
        return replaceLine(above.start, above.end)
      }
    }
    break
  }
  return text.slice(0, insertStart) + `${indent}// ${comment}\n` + text.slice(insertStart)
}

// ---- 形态 2 的实现 ----

/**
 * v1 provider 对象 → v2 形态（opencode 2 的键名，2026-10 迁移文档）。
 *
 * 对照：`npm`→`package`（加 `aisdk:` 前缀）、`options`→`settings`、
 * 模型 `id`→`modelID`、`tool_call`/`attachment`→`capabilities.{tools,input}`；
 * v1 的 `reasoning` 在 v2 被忽略（迁移文档列为「带警告忽略」），不迁移。
 */
export function toOpenCodeV2Provider(v1: Record<string, unknown>): Record<string, unknown> {
  const options = (v1.options ?? {}) as Record<string, unknown>
  const models: Record<string, unknown> = {}
  const rawModels = (v1.models ?? {}) as Record<string, Record<string, unknown>>

  for (const [key, raw] of Object.entries(rawModels)) {
    const entry: Record<string, unknown> = {}
    if (typeof raw.id === 'string') entry.modelID = raw.id
    if (typeof raw.name === 'string') entry.name = raw.name

    const capabilities: Record<string, unknown> = {}
    if (typeof raw.tool_call === 'boolean') capabilities.tools = raw.tool_call
    const input: string[] = ['text']
    if (raw.attachment === true) input.push('image')
    capabilities.input = input
    capabilities.output = ['text']
    entry.capabilities = capabilities

    if (raw.limit && typeof raw.limit === 'object') entry.limit = raw.limit
    models[key] = entry
  }

  const npm = typeof v1.npm === 'string' ? v1.npm : '@ai-sdk/openai-compatible'
  return {
    package: npm.startsWith('aisdk:') ? npm : `aisdk:${npm}`,
    ...(typeof v1.name === 'string' ? { name: v1.name } : {}),
    settings: { ...options },
    models,
  }
}

/** 在原文上做最小替换/插入；结构不符合预期时抛错（由上层降级） */
function spliceIntoExisting(
  text: string,
  context: GenerateContext,
  options: OpenCodeMergeOptions = {},
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
  const generatedValue = generated.provider[providerKey]

  // 跟随文件的 schema 世代：已有 v2 的 providers 容器就写 v2 形态
  //（v2 对 v1 键只做内存归一化，同文件冲突时 v2 原生值胜出 ——
  //  往 v2 文件里塞 v1 的 provider 键会被无视）。
  const memberKey = rootMembers.has('providers') ? 'providers' : 'provider'
  const providerValue =
    memberKey === 'providers'
      ? toOpenCodeV2Provider(generatedValue as unknown as Record<string, unknown>)
      : generatedValue

  // ---- 情况 A：顶层已有 provider 容器 ----
  const providerMember = rootMembers.get(memberKey)
  if (providerMember) {
    if (text[providerMember.valueStart] !== '{') {
      throw new Error(`"${memberKey}" 的值不是对象`)
    }
    const providerClose = scanner.findMatchingBrace(providerMember.valueStart)
    const providerMembers = scanner.listMembers(providerMember.valueStart)

    const existing = providerMembers.get(providerKey)
    if (existing) {
      // A-1 同名 provider：原位替换它的值。缩进跟随该成员所在行的行首，
      // 保证替换块与文件原有排版一致。
      const keyIndent = lineIndentOf(text, existing.keyStart)
      const renderedValue = renderValueAt(providerValue, keyIndent)
      let next =
        text.slice(0, existing.valueStart) + renderedValue + text.slice(existing.valueEnd)
      if (options.providerComment) {
        next = withMarkerComment(next, existing.keyStart, options.providerComment)
      }
      return { mode: 'splice', text: ensureTrailingNewline(next), warnings: [] }
    }

    // A-2 无同名 provider：插到 provider 对象末尾（闭括号前）
    const insertText = withCommentPrefix(
      buildMemberInsert(scanner, providerMembers, providerClose, providerKey, providerValue),
      options.providerComment,
    )
    const next =
      text.slice(0, providerClose) + insertText + text.slice(providerClose)
    return { mode: 'splice', text: ensureTrailingNewline(next), warnings: [] }
  }

  // ---- 情况 B：顶层没有 provider 容器 → 在根对象末尾插入整个成员 ----
  // 注意包裹一层 { <providerKey>: ... }：成员的值是 provider 容器，
  // 容器里才是各 provider 条目
  const insertText = withCommentPrefix(
    buildMemberInsert(scanner, rootMembers, rootClose, memberKey, {
      [providerKey]: providerValue,
    }),
    options.providerComment,
  )
  const next = text.slice(0, rootClose) + insertText + text.slice(rootClose)
  return { mode: 'splice', text: ensureTrailingNewline(next), warnings: [] }
}

/** 在「插入成员文本」之前补一行注释（与成员同缩进） */
function withCommentPrefix(insertText: string, comment?: string): string {
  if (!comment) return insertText
  // insertText 形如 `",\n<indent>"key": {...}"`，注释插到逗号换行之后、键之前
  const newlineIdx = insertText.indexOf('\n')
  if (newlineIdx < 0) return insertText
  const head = insertText.slice(0, newlineIdx + 1)
  const rest = insertText.slice(newlineIdx + 1)
  const indentMatch = rest.match(/^[ \t]*/)
  const indent = indentMatch ? indentMatch[0] : ''
  return `${head}${indent}// ${comment}\n${rest}`
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

// ---- 配置管理页的通用 provider 改写（upsert / remove） ----

/** 顶层 provider 容器的键名：有 v2 的 providers 就跟随 v2，否则 v1 的 provider */
function providerMemberKey(root: Record<string, unknown>): 'providers' | 'provider' {
  return typeof root.providers === 'object' && root.providers !== null ? 'providers' : 'provider'
}

/**
 * 把一个 provider 对象写入配置（存在则原位替换，不存在则插入末尾）。
 *
 * 与 `mergeOpenCodeIntoText` 的区别：这里操作的是**任意 provider 对象**
 * （配置管理页绑定密钥时会在解析结果上改动后回写），而不是从探测结果
 * 构造的新 provider。注释保留行为相同（原文拼接优先，失败降级整体重写）。
 * 写入的容器跟随文件世代（v2 providers / v1 provider）。
 */
export function upsertOpenCodeProvider(
  existingText: string,
  providerKey: string,
  providerValue: Record<string, unknown>,
): OpenCodeMergeResult {
  let parsed: unknown
  try {
    parsed = parseJsonc(existingText)
  } catch (err) {
    throw new MergeError(
      `现有文件不是合法的 JSON/JSONC（${err instanceof Error ? err.message : String(err)}），无法改写。`,
    )
  }
  const root =
    typeof parsed === 'object' && parsed !== null && !Array.isArray(parsed)
      ? (parsed as Record<string, unknown>)
      : null
  if (!root) {
    throw new MergeError('现有文件的顶层不是 JSON 对象，无法改写。')
  }
  const memberKey = providerMemberKey(root)

  if (existingText.trim() === '' || isEmptyJsoncObject(existingText)) {
    // 空文件：构造只有该 provider 的最小骨架
    const text = `{\n  "${memberKey}": ${JSON.stringify(
      { [providerKey]: providerValue },
      null,
      2,
    ).replace(/\n/g, '\n  ')}\n}\n`
    return { mode: 'create', text: ensureTrailingNewline(text), warnings: [] }
  }

  try {
    const scanner = new JsoncScanner(existingText)
    const rootOpen = findRootObject(existingText)
    if (rootOpen < 0) throw new Error('root object not found')
    const rootClose = scanner.findMatchingBrace(rootOpen)
    const rootMembers = scanner.listMembers(rootOpen)

    const providerMember = rootMembers.get(memberKey)
    if (!providerMember) {
      // 顶层没有 provider 容器：插入整个成员
      const insert = buildMemberInsert(
        scanner,
        rootMembers,
        rootClose,
        memberKey,
        { [providerKey]: providerValue },
      )
      const next = existingText.slice(0, rootClose) + insert + existingText.slice(rootClose)
      return { mode: 'splice', text: ensureTrailingNewline(next), warnings: [] }
    }
    if (existingText[providerMember.valueStart] !== '{') {
      throw new Error(`"${memberKey}" 的值不是对象`)
    }

    const providerClose = scanner.findMatchingBrace(providerMember.valueStart)
    const providerMembers = scanner.listMembers(providerMember.valueStart)
    const rendered = renderValueAt(providerValue, lineIndentOf(existingText, providerMember.keyStart))

    const existing = providerMembers.get(providerKey)
    if (existing) {
      const next =
        existingText.slice(0, existing.valueStart) +
        rendered +
        existingText.slice(existing.valueEnd)
      return { mode: 'splice', text: ensureTrailingNewline(next), warnings: [] }
    }
    const insert = buildMemberInsert(
      scanner,
      providerMembers,
      providerClose,
      providerKey,
      providerValue,
    )
    const next =
      existingText.slice(0, providerClose) + insert + existingText.slice(providerClose)
    return { mode: 'splice', text: ensureTrailingNewline(next), warnings: [] }
  } catch {
    // 降级：对象级合并重写
    const providers =
      typeof root[memberKey] === 'object' && root[memberKey] !== null && !Array.isArray(root[memberKey])
        ? (root[memberKey] as Record<string, unknown>)
        : {}
    const merged: Record<string, unknown> = {
      ...root,
      [memberKey]: { ...providers, [providerKey]: providerValue },
    }
    return {
      mode: 'rewrite',
      text: ensureTrailingNewline(JSON.stringify(merged, null, 2)),
      warnings: ['文件结构特殊，改为整体重写：注释会丢失，其他配置项保留。'],
    }
  }
}

/**
 * 从配置里删除一个 provider（原位摘除，注释保留）。
 *
 * @returns 不存在该 provider 时返回 null
 */
export function removeOpenCodeProvider(
  existingText: string,
  providerKey: string,
): OpenCodeMergeResult | null {
  if (existingText.trim() === '' || isEmptyJsoncObject(existingText)) {
    return null
  }

  let parsed: unknown
  try {
    parsed = parseJsonc(existingText)
  } catch (err) {
    throw new MergeError(
      `现有文件不是合法的 JSON/JSONC（${err instanceof Error ? err.message : String(err)}），无法改写。`,
    )
  }
  const root =
    typeof parsed === 'object' && parsed !== null && !Array.isArray(parsed)
      ? (parsed as Record<string, unknown>)
      : null
  if (!root) {
    throw new MergeError('现有文件的顶层不是 JSON 对象，无法改写。')
  }
  const memberKey = providerMemberKey(root)

  try {
    const scanner = new JsoncScanner(existingText)
    const rootOpen = findRootObject(existingText)
    if (rootOpen < 0) throw new Error('root object not found')
    const rootMembers = scanner.listMembers(rootOpen)
    const providerMember = rootMembers.get(memberKey)
    if (!providerMember || existingText[providerMember.valueStart] !== '{') return null

    const providerMembers = scanner.listMembers(providerMember.valueStart)
    const target = providerMembers.get(providerKey)
    if (!target) return null

    // 摘除区间：从成员键到值结束；把其后紧随的逗号（若有）一并吞掉，
    // 避免留下空元素；成员前的换行/缩进保留（不影响合法性）。
    let end = target.valueEnd
    const after = scanner.skipInsignificant(end)
    if (scanner.source[after] === ',') {
      end = after + 1
    }
    const next = existingText.slice(0, target.keyStart) + existingText.slice(end)
    return { mode: 'splice', text: ensureTrailingNewline(next), warnings: [] }
  } catch {
    // 降级：对象级删除后重写
    const providers =
      typeof root[memberKey] === 'object' && root[memberKey] !== null && !Array.isArray(root[memberKey])
        ? { ...(root[memberKey] as Record<string, unknown>) }
        : {}
    if (!(providerKey in providers)) return null
    delete providers[providerKey]
    const merged: Record<string, unknown> = { ...root, [memberKey]: providers }
    return {
      mode: 'rewrite',
      text: ensureTrailingNewline(JSON.stringify(merged, null, 2)),
      warnings: ['文件结构特殊，改为整体重写：注释会丢失，其他配置项保留。'],
    }
  }
}
