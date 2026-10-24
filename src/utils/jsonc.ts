/**
 * JSONC 读写工具：去注释解析 + 保留原文的成员定位。
 *
 * 「一键填写」需要在用户已有的 opencode.json（JSONC）里替换/插入一个
 * provider 条目。直接 parse → 改对象 → stringify 会把用户手写的注释
 * 全部丢掉，因此这里实现两套互补的能力：
 *
 *   1. parseJsonc()   —— 解析（容忍注释与尾逗号），用于理解文件结构；
 *   2. JsoncScanner   —— 在**原始文本**上定位对象成员的键与值区间，
 *                        供拼接式合并做文本级替换，注释原样保留。
 *
 * 扫描器只覆盖合并所需的最小语法面：对象/数组配对、字符串（含转义）、
 * 两种注释。数字、true/false/null 按普通 token 跳过即可，无需理解。
 */

/** 扫描器遇到的语法问题（带上位置，便于报给用户） */
export class JsoncSyntaxError extends Error {
  /** 出错字符在原文中的偏移（0 起） */
  readonly offset: number

  constructor(message: string, offset: number) {
    super(message)
    this.name = 'JsoncSyntaxError'
    this.offset = offset
  }
}

/** 对象里一个成员在原文中的位置区间（均为偏移，含边界语义见各字段） */
export interface MemberSpan {
  /** 成员键字符串字面量的起点（指向开引号） */
  keyStart: number
  /** 成员键字符串字面量的终点（闭引号之后） */
  keyEnd: number
  /** 值的起点（跳过冒号与空白/注释后的第一个有效字符） */
  valueStart: number
  /** 值的终点（值内容之后，不含其后的逗号） */
  valueEnd: number
}

/**
 * 去掉 JSONC 注释后的等价文本。
 *
 * 注释整体被替换成等长的空格（块注释内部的换行原样保留），
 * 保证输出与原文**行号与列偏移一致**——后续按行 diff、定位错误
 * 都不会错位。
 */
export function stripJsoncComments(text: string): string {
  let out = ''
  let i = 0
  const n = text.length
  let inString = false

  while (i < n) {
    const ch = text[i]

    if (inString) {
      if (ch === '\\' && i + 1 < n) {
        out += ch + text[i + 1]
        i += 2
        continue
      }
      if (ch === '"') inString = false
      out += ch
      i++
      continue
    }

    if (ch === '"') {
      inString = true
      out += ch
      i++
      continue
    }

    if (ch === '/' && text[i + 1] === '/') {
      let j = i
      while (j < n && text[j] !== '\n' && text[j] !== '\r') j++
      out += ' '.repeat(j - i)
      i = j
      continue
    }

    if (ch === '/' && text[i + 1] === '*') {
      let j = i + 2
      while (j < n && !(text[j] === '*' && text[j + 1] === '/')) j++
      const end = j < n ? j + 2 : n
      for (let k = i; k < end; k++) {
        out += text[k] === '\n' ? '\n' : text[k] === '\r' ? '\r' : ' '
      }
      i = end
      continue
    }

    out += ch
    i++
  }

  return out
}

/**
 * 解析 JSONC 文本（容忍注释与尾随逗号）。
 *
 * 失败时抛出的就是 JSON.parse 的 SyntaxError，调用方按需包装。
 */
export function parseJsonc(text: string): unknown {
  const stripped = stripJsoncComments(text)
  // 去掉 ] / } 前的尾逗号。字符串内容不受影响：strip 之后字符串仍带
  // 引号，而此正则要求逗号后仅经空白即到 } 或 ]，字符串字面量内部
  // 不可能满足（闭引号字符会挡在中间）。
  const repaired = stripped.replace(/,(\s*[}\]])/g, '$1')
  return JSON.parse(repaired)
}

/**
 * 判断（去注释后）文本是否是「空对象」—— `{}` 或纯空白。
 *
 * 「一键填写」用它判定文件是否等价于没有配置：
 * opencode 首次生成的骨架就是 `{}`，此时写入完整配置（带 $schema 与
 * 头注释）比塞片段更合适。
 */
export function isEmptyJsoncObject(text: string): boolean {
  // 注释换空格后再去掉全部空白：`{\n // x \n}` 归约成 `{}` 即视为空
  const collapsed = stripJsoncComments(text).replace(/\s/g, '')
  return collapsed === '{}' || collapsed === ''
}

// ---- 扫描器 ----

/**
 * JSONC 文本扫描器：定位对象与其成员在**原文**中的区间。
 *
 * 字符串与注释感知：配对计数会跳过 `"a}"`、`// }` 这类假括号。
 * 每个实例绑定一段文本，方法间以偏移量交流。
 */
export class JsoncScanner {
  private readonly text: string

  constructor(text: string) {
    this.text = text
  }

  /** 文本长度 */
  get length(): number {
    return this.text.length
  }

  /** 原文（供调用方做切片） */
  get source(): string {
    return this.text
  }

  /** 从 `start` 起跳过空白与注释，返回下一个有效字符的位置（可能在文本末尾之外） */
  skipInsignificant(start: number): number {
    const text = this.text
    let i = start
    const n = text.length
    while (i < n) {
      const ch = text[i]
      if (/\s/.test(ch)) {
        i++
        continue
      }
      if (ch === '/' && text[i + 1] === '/') {
        i = this.skipLineComment(i)
        continue
      }
      if (ch === '/' && text[i + 1] === '*') {
        i = this.skipBlockComment(i)
        continue
      }
      return i
    }
    return i
  }

  /** 从 `openIdx`（必须是 `{`）出发，返回配对 `}` 的位置 */
  findMatchingBrace(openIdx: number): number {
    const text = this.text
    if (text[openIdx] !== '{') {
      throw new JsoncSyntaxError(`位置 ${openIdx} 不是 '{'`, openIdx)
    }
    let depth = 0
    let i = openIdx
    while (i < text.length) {
      const ch = text[i]
      if (ch === '"') {
        i = this.skipString(i)
        continue
      }
      if (ch === '/' && text[i + 1] === '/') {
        i = this.skipLineComment(i)
        continue
      }
      if (ch === '/' && text[i + 1] === '*') {
        i = this.skipBlockComment(i)
        continue
      }
      if (ch === '{') depth++
      else if (ch === '}') {
        depth--
        if (depth === 0) return i
      }
      i++
    }
    throw new JsoncSyntaxError('括号不配对：找不到与之匹配的 }', openIdx)
  }

  /**
   * 列出对象的所有顶层成员区间。
   *
   * @param openIdx 对象 `{` 的位置（应为目标对象，而非根之外的什么）
   * @returns 键名 → 区间；重复键后者覆盖前者（与 JSON.parse 的语义一致）
   */
  listMembers(openIdx: number): Map<string, MemberSpan> {
    const text = this.text
    const closeIdx = this.findMatchingBrace(openIdx)
    const members = new Map<string, MemberSpan>()

    let i = openIdx + 1
    while (i < closeIdx) {
      i = this.skipInsignificant(i)
      if (i >= closeIdx) break

      // 期望一个成员键（字符串字面量）
      if (text[i] !== '"') {
        throw new JsoncSyntaxError(`位置 ${i}：期望成员键（字符串），实际是 "${text[i] ?? 'EOF'}"`, i)
      }
      const keyStart = i
      const keyEnd = this.skipString(i)
      const key = JSON.parse(text.slice(keyStart, keyEnd)) as string

      // 冒号
      let j = this.skipInsignificant(keyEnd)
      if (text[j] !== ':') {
        throw new JsoncSyntaxError(`成员 "${key}" 之后缺少冒号`, j)
      }

      // 值
      const valueStart = this.skipInsignificant(j + 1)
      const valueEnd = this.skipValue(valueStart, closeIdx)
      members.set(key, { keyStart, keyEnd, valueStart, valueEnd })

      // 逗号（可选，允许尾逗号）
      i = this.skipInsignificant(valueEnd)
      if (text[i] === ',') i++
    }

    return members
  }

  // ---- 内部：token 跳过 ----

  /** 跳过一个字符串字面量，返回其闭引号之后的位置 */
  private skipString(start: number): number {
    const text = this.text
    if (text[start] !== '"') {
      throw new JsoncSyntaxError(`位置 ${start} 不是字符串起点`, start)
    }
    let i = start + 1
    while (i < text.length) {
      const ch = text[i]
      if (ch === '\\') {
        i += 2
        continue
      }
      if (ch === '"') return i + 1
      // JSON 字符串不允许裸换行；出现即视为未闭合
      if (ch === '\n') {
        throw new JsoncSyntaxError(`位置 ${start} 的字符串未闭合`, start)
      }
      i++
    }
    throw new JsoncSyntaxError(`位置 ${start} 的字符串未闭合`, start)
  }

  private skipLineComment(start: number): number {
    const text = this.text
    let i = start + 2
    while (i < text.length && text[i] !== '\n') i++
    return i
  }

  private skipBlockComment(start: number): number {
    const text = this.text
    let i = start + 2
    while (i < text.length) {
      if (text[i] === '*' && text[i + 1] === '/') return i + 2
      i++
    }
    throw new JsoncSyntaxError('块注释未闭合', start)
  }

  /** 跳过一个值（对象/数组/字符串/标量），返回值的结束位置（不含） */
  private skipValue(start: number, limit: number): number {
    const text = this.text
    const ch = text[start]
    if (ch === undefined) {
      throw new JsoncSyntaxError(`位置 ${start}：期望一个值`, start)
    }
    if (ch === '{') {
      return this.findMatchingBrace(start) + 1
    }
    if (ch === '[') {
      return this.skipArray(start, limit)
    }
    if (ch === '"') {
      return this.skipString(start)
    }
    // 标量：读到逗号 / } / ] / 注释起点 / 空白
    let i = start
    while (i < limit) {
      const c = text[i]
      if (c === ',' || c === '}' || c === ']' || c === '/' || /\s/.test(c)) break
      i++
    }
    return i
  }

  /** 跳过一个数组字面量，返回 `]` 之后的位置 */
  private skipArray(start: number, limit: number): number {
    const text = this.text
    let i = start + 1
    while (i < limit) {
      i = this.skipInsignificant(i)
      if (i >= limit || text[i] === undefined) {
        throw new JsoncSyntaxError(`位置 ${start} 的数组未闭合`, start)
      }
      if (text[i] === ']') return i + 1

      const valueEnd = this.skipValue(i, limit)
      if (valueEnd === i) {
        throw new JsoncSyntaxError(`位置 ${i}：数组内出现无法识别的值`, i)
      }
      i = this.skipInsignificant(valueEnd)
      if (text[i] === ',') i++
    }
    throw new JsoncSyntaxError(`位置 ${start} 的数组未闭合`, start)
  }
}

/**
 * 定位 JSONC 文本的根对象 `{`。
 *
 * @returns 根 `{` 的偏移；文本为空/全是注释/根不是对象时返回 -1
 */
export function findRootObject(text: string): number {
  const scanner = new JsoncScanner(text)
  const idx = scanner.skipInsignificant(0)
  return idx < text.length && text[idx] === '{' ? idx : -1
}
