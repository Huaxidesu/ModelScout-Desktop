/**
 * 行级 diff 单元测试。
 *
 * 锁住三件事：变更行被正确标记（del/add）、上下文行保持 same、
 * 行号在两侧各自连续递增（左右对比视图的行号列依赖它）。
 */

import { describe, expect, it } from 'vitest'

import { diffLines } from './diffLines'

describe('diffLines', () => {
  it('完全相同的文本全是 same', () => {
    const rows = diffLines('a\nb\nc', 'a\nb\nc')
    expect(rows.map((r) => r.type)).toEqual(['same', 'same', 'same'])
    expect(rows[0]).toMatchObject({ leftNo: 1, rightNo: 1, left: 'a', right: 'a' })
  })

  it('中间插入一行 → 一条 add', () => {
    const rows = diffLines('a\nc', 'a\nb\nc')
    expect(rows.map((r) => r.type)).toEqual(['same', 'add', 'same'])
    expect(rows[1]).toMatchObject({ leftNo: null, rightNo: 2, right: 'b' })
  })

  it('删除一行 → 一条 del', () => {
    const rows = diffLines('a\nb\nc', 'a\nc')
    expect(rows.map((r) => r.type)).toEqual(['same', 'del', 'same'])
    expect(rows[1]).toMatchObject({ leftNo: 2, rightNo: null, left: 'b' })
  })

  it('修改一行表现为先 del 后 add', () => {
    const rows = diffLines('a\nold\nc', 'a\nnew\nc')
    expect(rows.map((r) => r.type)).toEqual(['same', 'del', 'add', 'same'])
  })

  it('空文本 → 全部是 add（新建文件场景）', () => {
    const rows = diffLines('', 'x\ny')
    expect(rows.map((r) => r.type)).toEqual(['add', 'add'])
    expect(rows.every((r) => r.leftNo === null && r.left === null)).toBe(true)
  })

  it('行号在两侧各自连续', () => {
    const rows = diffLines('1\n2\n3\n4', '1\nb\nc\n4\n5')
    const leftNos = rows.map((r) => r.leftNo).filter((n): n is number => n !== null)
    const rightNos = rows.map((r) => r.rightNo).filter((n): n is number => n !== null)
    expect(leftNos).toEqual([1, 2, 3, 4])
    expect(rightNos).toEqual([1, 2, 3, 4, 5])
  })

  it('CRLF 与 LF 混用不影响比较', () => {
    const rows = diffLines('a\r\nb\r\n', 'a\nb\n')
    expect(rows.map((r) => r.type)).toEqual(['same', 'same'])
  })
})
