import { describe, expect, it } from 'vitest'

import { validateConfigText } from './configValidate'

describe('validateConfigText —— 原始文件编辑的保存前校验', () => {
  it('合法 JSONC 通过,语法错误返回带上下文的中文说明', () => {
    expect(validateConfigText('a.jsonc', '{ "a": 1 } // 注释')).toBeNull()
    const err = validateConfigText('a.jsonc', '{ "a": }')
    expect(err).toContain('JSON/JSONC 语法错误')
  })

  it('空文件放行(不拦用户清空重写的意图)', () => {
    expect(validateConfigText('a.json', '   ')).toBeNull()
    expect(validateConfigText('a.yml', '')).toBeNull()
  })

  it('合法/非法 YAML', () => {
    expect(validateConfigText('p.yml', 'providers:\n  a:\n    baseURL: x\n')).toBeNull()
    const err = validateConfigText('p.yml', 'a: [\n  b: {')
    expect(err).toContain('YAML 语法错误')
  })

  it('按扩展名路由;未知扩展名不校验', () => {
    expect(validateConfigText('c.json', '{ ok }')).toContain('JSON')
    expect(validateConfigText('c.yaml', 'a: {')).toContain('YAML')
    expect(validateConfigText('c.toml', '{{{ 不是 toml 校验器')).toBeNull()
  })
})
