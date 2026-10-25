import { describe, expect, it } from 'vitest'

import type { ModelInfo } from '@/types/contracts'
import {
  buildOpenCodeV2Config,
  buildV2ModelEntry,
  renderOpenCodeV2Json,
  renderOpenCodeV2Jsonc,
  renderOpenCodeV2ProviderFragment,
} from './opencode'
import { detectOpenCodeVersion } from './configScan'

function makeModel(id: string, overrides: Partial<ModelInfo> = {}): ModelInfo {
  return {
    id,
    displayName: id,
    supportsTools: 'supported',
    supportsVision: 'unsupported',
    isReasoning: false,
    contextLimit: 32768,
    maxOutputLimit: 8192,
    reasonEvidence: '',
    error: null,
    durationMs: null,
    declaredCaps: null,
    latencyMs: null,
    visionEvidence: null,
    toolsEvidence: null,
    ...overrides,
  } as ModelInfo
}

function makeContextExtras() {
  return {
    baseUrl: 'https://api.example.com/v1',
    models: [makeModel('r1', { isReasoning: true }), makeModel('chat-1')],
    providerKey: 'my-gateway',
    apiKey: 'sk-test',
  }
}

describe('buildV2ModelEntry —— v2 模型条目', () => {
  it('能力映射到 capabilities(input/output/tools),推理模型可带 effort/budget', () => {
    const entry = buildV2ModelEntry(
      makeModel('r1', { isReasoning: true, supportsVision: 'supported' }),
      { effort: ['low', 'high'], budgetMin: 1024, budgetMax: 16384, temperature: true, inputLimit: 65536 },
    )
    expect(entry.modelID).toBe('r1')
    expect(entry.capabilities?.tools).toBe(true)
    expect(entry.capabilities?.input).toEqual(['text', 'image'])
    expect(entry.capabilities?.output).toEqual(['text'])
    expect(entry.capabilities?.temperature).toBe(true)
    expect(entry.capabilities?.reasoning).toEqual({
      supported: true,
      effort: ['low', 'high'],
      toggle: true,
      budget: { min: 1024, max: 16384 },
    })
    expect(entry.limit).toEqual({ context: 32768, output: 8192, input: 65536 })
  })

  it('非推理模型无 reasoning;未知的上限不硬编码(extra 缺省时 limit 不含 input)', () => {
    const entry = buildV2ModelEntry(makeModel('chat-1', { contextLimit: 0, maxOutputLimit: 0 }))
    expect(entry.capabilities?.reasoning).toBeUndefined()
    expect(entry.limit).toBeUndefined()
  })

  it('extras 的 effort 让条目带上 reasoning(即使 ModelInfo 未判为推理)', () => {
    const entry = buildV2ModelEntry(makeModel('m'), { effort: ['high'] })
    expect(entry.capabilities?.reasoning).toEqual({ supported: true, effort: ['high'], toggle: true })
  })
})

describe('OpenCode V2 渲染器', () => {
  it('buildOpenCodeV2Config:providers 容器 + aisdk: 前缀 + settings', () => {
    const config = buildOpenCodeV2Config(makeContextExtras())
    expect(Object.keys(config.providers!)).toEqual(['my-gateway'])
    const provider = config.providers!['my-gateway']
    expect(provider.package).toBe('aisdk:@ai-sdk/openai-compatible')
    expect(provider.settings.baseURL).toBe('https://api.example.com/v1')
    expect(provider.settings.apiKey).toBe('sk-test')
    expect(Object.keys(provider.models)).toEqual(['r1', 'chat-1'])
  })

  it('renderOpenCodeV2Jsonc 输出 providers/capabilities/effort,且是合法 JSON 去注释后可解析', () => {
    const text = renderOpenCodeV2Jsonc(makeContextExtras(), {
      r1: { effort: ['low', 'medium', 'high'] },
    })
    expect(text).toContain('"providers"')
    expect(text).toContain('"package": "aisdk:')
    expect(text).toContain('"effort"')
    expect(text).toContain('"toggle"')
    // 去掉 // 注释行后应为合法 JSON
    const stripped = text
      .split('\n')
      .filter((l) => !l.trim().startsWith('//'))
      .join('\n')
    expect(() => JSON.parse(stripped)).not.toThrow()
  })

  it('renderOpenCodeV2Json 是严格 JSON(无注释行)', () => {
    const text = renderOpenCodeV2Json(makeContextExtras())
    expect(() => JSON.parse(text)).not.toThrow()
    // URL 里的 https:// 不算注释;检查没有以 // 开头的注释行
    expect(text.split('\n').some((l) => l.trim().startsWith('//'))).toBe(false)
  })

  it('片段形态包一层 "providers": { … }', () => {
    const text = renderOpenCodeV2ProviderFragment(makeContextExtras())
    // 头部是注释块;第一行代码必须是 "providers": {
    const firstCodeLine = text
      .split('\n')
      .find((l) => l.trim() !== '' && !l.trim().startsWith('//'))
    expect(firstCodeLine?.trim()).toBe('"providers": {')
    expect(text).toContain('"modelID"')
  })
})

describe('detectOpenCodeVersion —— 世代识别', () => {
  it('providers 容器 → v2;provider 容器 → v1;两者都有 → v2', () => {
    expect(detectOpenCodeVersion('{ "providers": {} }')).toBe('v2')
    expect(detectOpenCodeVersion('{ "provider": {} }')).toBe('v1')
    expect(detectOpenCodeVersion('{ "provider": {}, "providers": {} }')).toBe('v2')
  })

  it('空对象 / 别的内容 / 解析失败 → null', () => {
    expect(detectOpenCodeVersion('{}')).toBeNull()
    expect(detectOpenCodeVersion('{ "mcp": {} }')).toBeNull()
    expect(detectOpenCodeVersion('not json {')).toBeNull()
    expect(detectOpenCodeVersion('')).toBeNull()
  })

  it('真实 v2 文件(含 capabilities/limit)识别为 v2', () => {
    const real = `{
  "providers": {
    "shusheng": {
      "package": "aisdk:@ai-sdk/openai-compatible",
      "settings": { "baseURL": "https://example.com/v1" },
      "models": {
        "Agents-A1": { "modelID": "Agents-A1", "capabilities": { "tools": true, "input": ["text"] }, "limit": { "context": 262144, "output": 262144 } }
      }
    }
  }
}`
    expect(detectOpenCodeVersion(real)).toBe('v2')
  })
})
