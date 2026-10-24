/**
 * JSONC 工具单元测试。
 *
 * 重点锁住三类行为（都是「一键填写」合并的实际依赖）：
 *   1. stripJsoncComments 的行/列偏移稳定性（diff 与错误定位依赖）；
 *   2. parseJsonc 对尾逗号、字符串内假注释的容忍；
 *   3. JsoncScanner 在含注释文本上定位成员区间的正确性。
 */

import { describe, expect, it } from 'vitest'

import {
  JsoncScanner,
  findRootObject,
  isEmptyJsoncObject,
  parseJsonc,
  stripJsoncComments,
} from './jsonc'

describe('stripJsoncComments', () => {
  it('去掉行注释与块注释，其余原样保留', () => {
    const text = `{
  // 行注释
  "a": 1, /* 块
注释 */
  "b": "x"
}`
    const stripped = stripJsoncComments(text)
    expect(parseJsonc(text)).toEqual({ a: 1, b: 'x' })
    // 与原文行数一致
    expect(stripped.split('\n').length).toBe(text.split('\n').length)
  })

  it('字符串内部的 // 与 /* 不被当作注释', () => {
    const text = '{"url": "http://x//y/*z*/"}'
    expect(parseJsonc(text)).toEqual({ url: 'http://x//y/*z*/' })
  })

  it('字符串内的转义引号不提前结束字符串', () => {
    const text = '{"k": "a\\"b // 仍字符串"}'
    expect(parseJsonc(text)).toEqual({ k: 'a"b // 仍字符串' })
  })
})

describe('parseJsonc', () => {
  it('容忍对象与数组的尾逗号', () => {
    expect(parseJsonc('{"a": [1, 2,],}')).toEqual({ a: [1, 2] })
  })

  it('普通 JSON 原样可解析', () => {
    expect(parseJsonc('{"a":1}')).toEqual({ a: 1 })
  })

  it('非法 JSON 抛 SyntaxError', () => {
    expect(() => parseJsonc('{"a": }')).toThrow(SyntaxError)
  })
})

describe('isEmptyJsoncObject', () => {
  it('{}、空白、只有注释都视为空', () => {
    expect(isEmptyJsoncObject('{}')).toBe(true)
    expect(isEmptyJsoncObject('   \n ')).toBe(true)
    expect(isEmptyJsoncObject('{\n  // 只有注释\n}')).toBe(true)
  })

  it('有实际成员则不视为空', () => {
    expect(isEmptyJsoncObject('{"a":1}')).toBe(false)
  })
})

describe('JsoncScanner', () => {
  it('定位顶层成员的值区间（含注释干扰）', () => {
    const text = `{
  // 头部注释
  "alpha": { "x": 1 },  /* 尾注 */
  "beta": "}"
}`
    const scanner = new JsoncScanner(text)
    const root = findRootObject(text)
    expect(root).toBe(0)

    const members = scanner.listMembers(root)
    expect(members.size).toBe(2)

    const alpha = members.get('alpha')!
    expect(text.slice(alpha.valueStart, alpha.valueEnd)).toBe('{ "x": 1 }')

    const beta = members.get('beta')!
    expect(text.slice(beta.valueStart, beta.valueEnd)).toEqual('"}"')
  })

  it('字符串里的假括号不破坏配对', () => {
    const text = '{"a": "}{", "b": {"c": 2}}'
    const scanner = new JsoncScanner(text)
    const members = scanner.listMembers(0)
    expect(members.size).toBe(2)
    expect(text.slice(members.get('b')!.valueStart, members.get('b')!.valueEnd)).toBe('{"c": 2}')
  })

  it('嵌套对象里的成员键不会泄漏到外层（listMembers 只看一层）', () => {
    const text = '{"outer": {"inner": 1}, "second": 2}'
    const members = new JsoncScanner(text).listMembers(0)
    expect([...members.keys()]).toEqual(['outer', 'second'])
  })

  it('数组值与多层嵌套数组能被跳过', () => {
    const text = '{"list": [1, [2, {"k": "v,"}], 3], "next": true}'
    const members = new JsoncScanner(text).listMembers(0)
    expect(text.slice(members.get('list')!.valueStart, members.get('list')!.valueEnd))
      .toBe('[1, [2, {"k": "v,"}], 3]')
    expect(members.has('next')).toBe(true)
  })

  it('括号不配对抛 JsoncSyntaxError', () => {
    const scanner = new JsoncScanner('{"a": {')
    expect(() => scanner.findMatchingBrace(0)).toThrow(/配对/)
  })

  it('顶层不是对象时 findRootObject 返回 -1', () => {
    expect(findRootObject('  // 注释\n[1, 2]')).toBe(-1)
    expect(findRootObject('')).toBe(-1)
  })
})
