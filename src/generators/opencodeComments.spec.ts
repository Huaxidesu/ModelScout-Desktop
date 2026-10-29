import { describe, expect, it } from 'vitest'

import { parseJsonc } from '@/utils/jsonc'
import {
  readMarkerComment,
  readProviderComments,
  removeProviderComment,
  setProviderComment,
} from './opencodeComments'

/** 带标记注释的 v2 文件（providers 容器） */
const V2 = `{
  "providers": {
    // Intern AI · 8 个模型 · ModelScout 2026-10-25
    "intern": { "package": "aisdk:x" },
    // 用户备注
    "plain": { "package": "aisdk:y" }
  }
}`

/** 历史上堆积了两条标记注释（删 provider 只摘成员留下的孤儿） */
const STACKED = `{
  "providers": {
    // Intern AI · 8 个模型 · ModelScout 2026-10-25
    // Shusheng · 8 个模型 · ModelScout 2026-10-25
    "intern": { "package": "aisdk:x" }
  }
}`

/** v1 文件（provider 容器） */
const V1 = `{
  "provider": {
    // Old · 3 个模型 · ModelScout 2026-10-01
    "old": { "npm": "x" }
  }
}`

describe('readProviderComments', () => {
  it('读出 v2 providers 里的标记注释，用户注释不算', () => {
    const comments = readProviderComments(V2)
    expect(comments.get('intern')).toBe('Intern AI · 8 个模型 · ModelScout 2026-10-25')
    expect(comments.has('plain')).toBe(false)
  })

  it('v1 的 provider 容器同样能读', () => {
    expect(readProviderComments(V1).get('old')).toBe('Old · 3 个模型 · ModelScout 2026-10-01')
  })

  it('文件非法时返回空表而不是抛错', () => {
    expect(readProviderComments('{ 不是 json').size).toBe(0)
  })
})

describe('readMarkerComment', () => {
  it('取不到标记注释时返回 null', () => {
    const comments = readProviderComments(V2)
    expect(comments.has('plain')).toBe(false)
    // 用户注释「用户备注」不是标记注释
    const text = V2
    const plainKey = text.indexOf('"plain"')
    expect(readMarkerComment(text, plainKey)).toBeNull()
  })
})

describe('setProviderComment —— 编辑 / 新增 / 删除', () => {
  it('替换已有标记注释，缩进与文件保持一致', () => {
    const next = setProviderComment(V2, 'intern', '主力网关 · 已换 V4.1')
    expect(next).not.toBeNull()
    expect(next!).toContain('    // 主力网关 · 已换 V4.1')
    expect(next!).not.toContain('Intern AI · 8 个模型')
    // 其他条目不受影响
    expect(next!).toContain('// 用户备注')
    const parsed = parseJsonc(next!) as { providers: Record<string, unknown> }
    expect(Object.keys(parsed.providers)).toEqual(['intern', 'plain'])
  })

  it('原本没有标记注释时插到键的上一行', () => {
    const next = setProviderComment(V2, 'plain', '给 plain 加一行')
    expect(next).toContain('    // 给 plain 加一行\n    "plain"')
    expect(next).toContain('// 用户备注')
    // 插入后仍是合法 JSONC
    expect(() => parseJsonc(next!)).not.toThrow()
  })

  it('传空串 = 删除该行注释', () => {
    const next = setProviderComment(V2, 'intern', '   ')
    expect(next).not.toBeNull()
    expect(next!).not.toContain('Intern AI · 8 个模型')
    expect(next!).toContain('// 用户备注')
    expect(() => parseJsonc(next!)).not.toThrow()
  })

  it('历史堆积的多条标记注释只留一条', () => {
    const next = setProviderComment(STACKED, 'intern', '只留这一条')
    expect(next).not.toBeNull()
    expect(next!).toContain('只留这一条')
    // 旧的孤儿注释（Shusheng 那条）一并清掉
    expect(next!).not.toContain('Shusheng')
    expect(next!.match(/ModelScout/g) ?? []).toHaveLength(0)
  })

  it('provider 不存在时返回 null', () => {
    expect(setProviderComment(V2, 'ghost', 'x')).toBeNull()
  })
})

describe('removeProviderComment', () => {
  it('删掉标记注释，用户自己的注释保留', () => {
    const mixed = `{
  "providers": {
    // 我自己的说明
    // A · 1 个模型 · ModelScout 2026-10-29
    "a": { "package": "aisdk:x" }
  }
}`
    const next = removeProviderComment(mixed, 'a')
    expect(next).not.toBeNull()
    expect(next!).not.toContain('ModelScout')
    expect(next!).toContain('我自己的说明')
    expect(() => parseJsonc(next!)).not.toThrow()
  })

  it('堆积的多条一起清掉', () => {
    const next = removeProviderComment(STACKED, 'intern')
    expect(next).not.toBeNull()
    expect(next!).not.toContain('ModelScout')
  })

  it('provider 不存在时返回 null', () => {
    expect(removeProviderComment(V2, 'ghost')).toBeNull()
  })
})
