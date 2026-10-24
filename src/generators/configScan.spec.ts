/**
 * 配置扫描解析单元测试。
 *
 * 锁住配置管理页「一键识别」的核心语义：
 *   - OpenCode：provider 键/名/baseURL/密钥三态（明文、env 引用、无）/模型清单；
 *   - dsh：只认 llm-pi-ai 条目，其余插件条目忽略；
 *   - 容错：坏条目跳过不拖垮整份文件。
 */

import { describe, expect, it } from 'vitest'

import {
  parseDshProviders,
  parseEnvRef,
  parseOpenCodeProviders,
  rawDshProvider,
  rawOpenCodeProvider,
  suggestEnvVarName,
} from './configScan'
import { parseJsonc } from '@/utils/jsonc'

const OPENCODE_SAMPLE = `{
  // 注释也该被容忍
  "mcp": {},
  "provider": {
    "my-relay": {
      "npm": "@ai-sdk/openai-compatible",
      "name": "My Relay",
      "options": { "baseURL": "https://gw.example/v1", "apiKey": "sk-plain-123" },
      "models": { "a": { "id": "a" }, "b": { "id": "b" } }
    },
    "env-ref": {
      "options": { "baseURL": "https://x/v1", "apiKey": "{env:X_API_KEY}" },
      "models": {}
    },
    "no-key": { "options": { "baseURL": "https://y/v1" } }
  }
}`

const DSH_SAMPLE = `- id: some-other-plugin
  config: { keep: true }
- id: llm-pi-ai
  config:
    providers:
      my-relay:
        displayName: My Relay
        apiKeyEnv: MY_RELAY_API_KEY
        api: openai-completions
        baseURL: https://gw.example/v1
        models:
          - id: deepseek-chat
          - id: deepseek-vl
`

describe('parseOpenCodeProviders', () => {
  it('解析 provider 的全部字段与密钥三态', () => {
    const { entries, errors } = parseOpenCodeProviders(OPENCODE_SAMPLE)
    expect(errors).toEqual([])
    expect(entries.map((e) => e.key)).toEqual(['my-relay', 'env-ref', 'no-key'])

    const relay = entries[0]
    expect(relay.name).toBe('My Relay')
    expect(relay.baseURL).toBe('https://gw.example/v1')
    expect(relay.apiKey).toEqual({ mode: 'plain', value: 'sk-plain-123' })
    expect(relay.modelIds).toEqual(['a', 'b'])

    expect(entries[1].apiKey).toEqual({ mode: 'env', envVar: 'X_API_KEY' })
    expect(entries[2].apiKey).toEqual({ mode: 'none' })
  })

  it('顶层没有 provider 时返回错误说明', () => {
    const result = parseOpenCodeProviders('{"mcp": {}}')
    expect(result.entries).toEqual([])
    expect(result.errors[0]).toContain('provider')
  })

  it('非法 JSON 返回错误而不是抛出', () => {
    const result = parseOpenCodeProviders('{"a": oops}')
    expect(result.entries).toEqual([])
    expect(result.errors[0]).toContain('JSON')
  })

  it('rawOpenCodeProvider 返回原始对象（含扫描摘要没有的字段）', () => {
    const raw = rawOpenCodeProvider(OPENCODE_SAMPLE, 'my-relay')
    expect(raw).not.toBeNull()
    expect(raw?.schema).toBe('v1')
    expect(raw?.value.npm).toBe('@ai-sdk/openai-compatible')
    expect((raw?.value.options as Record<string, unknown>)?.apiKey).toBe('sk-plain-123')
    expect(rawOpenCodeProvider(OPENCODE_SAMPLE, 'missing')).toBeNull()
  })

  it('v2 文件（providers/package/settings/modelID）也能解析', () => {
    const v2 = `{
  "providers": {
    "shusheng": {
      "package": "aisdk:@ai-sdk/openai-compatible",
      "settings": { "baseURL": "https://x/v1", "apiKey": "{env:SS_KEY}" },
      "models": { "m1": { "modelID": "m1", "capabilities": { "tools": true } } }
    }
  }
}`
    const { entries, errors } = parseOpenCodeProviders(v2)
    expect(errors).toEqual([])
    expect(entries[0].schema).toBe('v2')
    expect(entries[0].key).toBe('shusheng')
    expect(entries[0].npm).toBe('aisdk:@ai-sdk/openai-compatible')
    expect(entries[0].baseURL).toBe('https://x/v1')
    expect(entries[0].apiKey).toEqual({ mode: 'env', envVar: 'SS_KEY' })
    expect(entries[0].modelIds).toEqual(['m1'])

    const raw = rawOpenCodeProvider(v2, 'shusheng')
    expect(raw?.schema).toBe('v2')
    expect(raw?.value.package).toBe('aisdk:@ai-sdk/openai-compatible')
  })
})

describe('parseDshProviders', () => {
  it('只认 llm-pi-ai 条目，忽略其他插件', () => {
    const { entries, errors } = parseDshProviders(DSH_SAMPLE)
    expect(errors).toEqual([])
    expect(entries).toHaveLength(1)
    expect(entries[0].key).toBe('my-relay')
    expect(entries[0].name).toBe('My Relay')
    expect(entries[0].apiKeyEnv).toBe('MY_RELAY_API_KEY')
    expect(entries[0].api).toBe('openai-completions')
    expect(entries[0].modelIds).toEqual(['deepseek-chat', 'deepseek-vl'])
  })

  it('rawDshProvider 返回深拷贝的原始对象', () => {
    const raw = rawDshProvider(DSH_SAMPLE, 'my-relay')
    expect(raw).not.toBeNull()
    expect(raw?.apiKeyEnv).toBe('MY_RELAY_API_KEY')
    expect(rawDshProvider(DSH_SAMPLE, 'missing')).toBeNull()
  })

  it('顶层不是数组时报错', () => {
    expect(parseDshProviders('just: string').errors[0]).toContain('数组')
  })
})

describe('工具函数', () => {
  it('parseEnvRef 识别 {env:VAR} 形态', () => {
    expect(parseEnvRef('{env:MY_KEY}')).toBe('MY_KEY')
    expect(parseEnvRef('sk-plain')).toBeNull()
    expect(parseEnvRef('{env:}')).toBeNull()
  })

  it('suggestEnvVarName 由键名推导环境变量名', () => {
    expect(suggestEnvVarName('my-relay')).toBe('MY_RELAY_API_KEY')
    expect(suggestEnvVarName('a.b')).toBe('A_B_API_KEY')
  })

  it('parseJsonc 兼容（sanity：与 jsonc 工具一致）', () => {
    expect(parseJsonc(OPENCODE_SAMPLE)).toBeTruthy()
  })
})
