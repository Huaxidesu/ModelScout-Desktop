/**
 * OpenCode 配置里「provider 上方那行 ModelScout 标记注释」的读写。
 *
 * 背景：一键填写会在每个 provider 键的上一行写一行
 * `// 名字 · N 个模型 · ModelScout 日期`。早期删除 provider 时只摘掉
 * 成员、不动注释，删几次之后文件里就堆一串没有归属的注释
 * （`// Intern AI · 8 个模型 · ModelScout 2026-10-25` 之类）。
 *
 * 本模块只做三件事：读、写（含清掉多余堆积）、删。判定标记的规则是
 * **注释里出现 `ModelScout`**，用户自己写的普通注释一律不碰。
 *
 * 注释块的范围取「provider 键所在行向上连续的注释/空行」：连续注释块在
 * 书写习惯上归属它下方那个键，因此不会误删别的 provider 的注释。
 */

import { JsoncScanner, findRootObject, type MemberSpan } from '@/utils/jsonc'

/** 一行注释在原文里的位置；`end` 指向行尾换行符本身（不含它） */
export interface CommentLine {
  /** 行首下标 */
  start: number
  /** 行尾换行符下标；文件末行无换行时为 text.length */
  end: number
  /** 整行原文（不含换行符） */
  line: string
}

/** 标记注释的判定：行注释且正文里出现 ModelScout */
const MARKER_LINE = /^\s*\/\/.*ModelScout/

/** offset 所在行的行首下标 */
function lineStartOf(text: string, offset: number): number {
  return text.lastIndexOf('\n', Math.max(offset - 1, 0)) + 1
}

/** 取一行开头的空白 */
function indentOf(line: string): string {
  return line.match(/^[ \t]*/)?.[0] ?? ''
}

/**
 * 从 `fromLineStart` 所在行向上回溯，收集连续注释块里的标记注释行。
 *
 * 只越过空行与注释行；遇到代码行立即停止 —— 这是「注释块归属下方键」
 * 的边界，越过去就可能吃掉别的条目的注释。
 *
 * @returns 就近优先（下标 0 是紧贴键的那一条）
 */
function markerCommentLines(text: string, fromLineStart: number): CommentLine[] {
  const found: CommentLine[] = []
  let cursor = fromLineStart
  for (let guard = 0; guard < 30 && cursor > 0; guard++) {
    const start = lineStartOf(text, cursor - 1)
    const end = cursor - 1
    const line = text.slice(start, end)
    if (line.trim() === '') {
      cursor = start
      continue
    }
    if (/^\s*\/\//.test(line)) {
      if (MARKER_LINE.test(line)) found.push({ start, end, line })
      cursor = start
      continue
    }
    break
  }
  return found
}

/** 一处文本替换（按 start 降序应用时下标不会失效） */
interface Edit {
  start: number
  end: number
  text: string
}

function applyEdits(text: string, edits: Edit[]): string {
  return [...edits]
    .sort((a, b) => b.start - a.start)
    .reduce((acc, e) => acc.slice(0, e.start) + e.text + acc.slice(e.end), text)
}

/**
 * 构造「整行删除」的编辑。
 *
 * 连同行尾换行一起删；若紧邻的上一行是纯空白行，也一并吃掉 ——
 * 历史堆积的孤儿注释之间常常夹着这种空行，不删的话清理完还剩一串
 * 只有空格的残留行。
 */
function deleteLineEdit(text: string, line: CommentLine): Edit {
  let start = line.start
  const prevEnd = start - 1 // 上一行的换行符下标
  if (prevEnd > 0) {
    const prevStart = text.lastIndexOf('\n', prevEnd - 1) + 1
    if (text.slice(prevStart, prevEnd).trim() === '') start = prevStart
  }
  return { start, end: Math.min(line.end + 1, text.length), text: '' }
}

/**
 * 读取 `keyStart` 所在键上方最近的一条标记注释正文（去掉 `//` 前缀）。
 *
 * @returns 没有标记注释时返回 null
 */
export function readMarkerComment(text: string, keyStart: number): string | null {
  const lines = markerCommentLines(text, lineStartOf(text, keyStart))
  if (lines.length === 0) return null
  return lines[0].line.replace(/^\s*\/\/\s?/, '')
}

/**
 * 删除 `keyStart` 上方**所有**标记注释行（含行尾换行），返回新文本。
 *
 * 用于「删除 provider 时连带删注释」，也顺带清理历史堆积。
 */
export function removeMarkerComments(text: string, keyStart: number): string {
  const lines = markerCommentLines(text, lineStartOf(text, keyStart))
  if (lines.length === 0) return text
  return applyEdits(
    text,
    lines.map((l) => deleteLineEdit(text, l)),
  )
}

/**
 * 写入 / 更新 / 删除 `keyStart` 上方的标记注释。
 *
 * - `comment` 非空：替换最近一条（缩进沿用原行），并把上方多余的堆积行删掉；
 *   上方没有标记注释时在键的上一行插入。
 * - `comment` 为空：删掉所有标记注释行。
 */
export function setMarkerComment(text: string, keyStart: number, comment: string): string {
  const lines = markerCommentLines(text, lineStartOf(text, keyStart))
  const body = comment.trim()
  const edits: Edit[] = []

  if (lines.length === 0) {
    if (!body) return text
    const at = lineStartOf(text, keyStart)
    edits.push({ start: at, end: at, text: `${indentOf(text.slice(at))}// ${body}\n` })
    return applyEdits(text, edits)
  }

  const nearest = lines[0]
  if (body) {
    edits.push({ start: nearest.start, end: nearest.end, text: `${indentOf(nearest.line)}// ${body}` })
  } else {
    edits.push(deleteLineEdit(text, nearest))
  }
  // 上方多余的堆积行一律删掉：一次编辑就把历史遗留清理干净
  for (const extra of lines.slice(1)) {
    edits.push(deleteLineEdit(text, extra))
  }
  return applyEdits(text, edits)
}

/** 顶层 provider 容器的候选键名（v2 的 providers 优先，其次 v1 的 provider） */
const CONTAINER_KEYS = ['providers', 'provider'] as const

/** 一个已定位的 provider 成员 */
interface LocatedProvider {
  scanner: JsoncScanner
  target: MemberSpan
  /** 成员所在容器对象的闭括号下标 */
  containerClose: number
  containerStart: number
}

/** 在文本里定位 provider 成员；结构不符或找不到时返回 null */
function locateProvider(text: string, providerKey: string): LocatedProvider | null {
  try {
    const scanner = new JsoncScanner(text)
    const rootOpen = findRootObject(text)
    if (rootOpen < 0) return null
    const rootMembers = scanner.listMembers(rootOpen)
    for (const key of CONTAINER_KEYS) {
      const container = rootMembers.get(key)
      if (!container || text[container.valueStart] !== '{') continue
      const target = scanner.listMembers(container.valueStart).get(providerKey)
      if (!target) continue
      return {
        scanner,
        target,
        containerStart: container.valueStart,
        containerClose: scanner.findMatchingBrace(container.valueStart),
      }
    }
    return null
  } catch {
    return null
  }
}

/**
 * 读取文件里所有 provider 的标记注释。
 *
 * @returns key → 注释正文；没有标记注释的 provider 不出现在表里
 */
export function readProviderComments(text: string): Map<string, string> {
  const out = new Map<string, string>()
  try {
    const scanner = new JsoncScanner(text)
    const rootOpen = findRootObject(text)
    if (rootOpen < 0) return out
    const rootMembers = scanner.listMembers(rootOpen)
    for (const key of CONTAINER_KEYS) {
      const container = rootMembers.get(key)
      if (!container || text[container.valueStart] !== '{') continue
      for (const [providerKey, span] of scanner.listMembers(container.valueStart)) {
        if (out.has(providerKey)) continue
        const comment = readMarkerComment(text, span.keyStart)
        if (comment !== null) out.set(providerKey, comment)
      }
    }
  } catch {
    // 文件结构异常时当作「没有注释」处理，调用方按无注释渲染即可
  }
  return out
}

/**
 * 改写某个 provider 的标记注释（空串 = 删除），返回新文本。
 *
 * @returns provider 不存在时返回 null（调用方据此提示重新识别）
 */
export function setProviderComment(
  text: string,
  providerKey: string,
  comment: string,
): string | null {
  const located = locateProvider(text, providerKey)
  if (!located) return null
  return setMarkerComment(text, located.target.keyStart, comment)
}

/**
 * 删除某个 provider 上方的标记注释，返回新文本。
 *
 * @returns provider 不存在时返回 null
 */
export function removeProviderComment(text: string, providerKey: string): string | null {
  const located = locateProvider(text, providerKey)
  if (!located) return null
  return removeMarkerComments(text, located.target.keyStart)
}
