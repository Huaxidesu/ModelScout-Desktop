import { describe, expect, it } from 'vitest'

import { collectEnvVarRefs, missingEnvVars } from './envRefs'

describe('collectEnvVarRefs —— 收集配置里的环境变量引用', () => {
  it('opencode 的 env 模式与 dsh 的 apiKeyEnv 都收集,plain/none 跳过', () => {
    const refs = collectEnvVarRefs(
      [
        { key: 'a', apiKey: { mode: 'env', envVar: 'SHARED_KEY' } },
        { key: 'b', apiKey: { mode: 'plain' } },
        { key: 'c', apiKey: { mode: 'none' } },
      ],
      [
        { key: 'd', apiKeyEnv: 'DSH_KEY' },
        { key: 'e', apiKeyEnv: '' },
      ],
    )
    expect([...refs.keys()].sort()).toEqual(['DSH_KEY', 'SHARED_KEY'])
    expect(refs.get('SHARED_KEY')).toEqual([{ tool: 'opencode', providerKey: 'a' }])
    expect(refs.get('DSH_KEY')).toEqual([{ tool: 'dsh', providerKey: 'd' }])
  })

  it('同一变量被多个 provider 引用时全部保留(顺序=传入顺序)', () => {
    const refs = collectEnvVarRefs(
      [
        { key: 'a', apiKey: { mode: 'env', envVar: 'K' } },
        { key: 'b', apiKey: { mode: 'env', envVar: 'K' } },
      ],
      [{ key: 'c', apiKeyEnv: 'K' }],
    )
    expect(refs.get('K')).toEqual([
      { tool: 'opencode', providerKey: 'a' },
      { tool: 'opencode', providerKey: 'b' },
      { tool: 'dsh', providerKey: 'c' },
    ])
  })

  it('空引用名与纯空白被忽略', () => {
    const refs = collectEnvVarRefs(
      [
        { key: 'a', apiKey: { mode: 'env', envVar: '   ' } },
        { key: 'b', apiKey: { mode: 'env', envVar: '' } as never },
      ],
      [{ key: 'c', apiKeyEnv: '  ' }],
    )
    expect(refs.size).toBe(0)
  })
})

describe('missingEnvVars —— 配置引用了但机器上没有的变量', () => {
  it('区分大小写精确匹配,结果按名字排序', () => {
    const refs = collectEnvVarRefs(
      [{ key: 'a', apiKey: { mode: 'env', envVar: 'B_KEY' } }],
      [{ key: 'c', apiKeyEnv: 'A_KEY' }],
    )
    expect(missingEnvVars(refs, ['a_key', 'C_KEY'])).toEqual(['A_KEY', 'B_KEY'])
  })

  it('全部存在时返回空数组', () => {
    const refs = collectEnvVarRefs([{ key: 'a', apiKey: { mode: 'env', envVar: 'K' } }], [])
    expect(missingEnvVars(refs, ['K'])).toEqual([])
    expect(missingEnvVars(new Map(), ['anything'])).toEqual([])
  })
})
