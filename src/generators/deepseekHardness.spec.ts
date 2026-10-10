/**
 * ============================================================================
 * ModelScout —— DeepSeekHardness 配置生成器单元测试
 * ============================================================================
 *
 * 与 opencode.spec.ts 关注点一致的三件事：
 *   1. 密钥到底有没有写进配置（以及以什么形式写）
 *   2. provider 命名是否可用、推断是否合理
 *   3. 注释有没有淹掉配置本身
 *
 * 这个生成器的格式本身是**假设**（见文件头免责说明），
 * 因此测试只锁定「本生成器可控的部分」：字段值对不对、JSONC 是否可解析、
 * 密钥与命名是否一致 —— 不去断言那些本来就要靠人工核对的字段名。
 * ============================================================================
 */

import { describe, expect, it } from 'vitest'

import type { ModelInfo } from '@/types/contracts'
import {
  buildHardnessConfig,
  renderHardnessJson,
  renderHardnessJsonc,
  renderHardnessToml,
  validateNoPlaceholders,
  type GenerateContext,
} from './deepseekHardness'

function makeModel(id: string, overrides: Partial<ModelInfo> = {}): ModelInfo {
  return {
    id,
    displayName: id,
    ownedBy: 'test',
    created: 0,
    supportsTools: 'unknown',
    supportsVision: 'unknown',
    supportsStreaming: 'unknown',
    isReasoning: false,
    reasoningEvidence: '',
    contextLimit: 0,
    maxOutputLimit: 0,
    contextLimitSource: 'unknown',
    maxOutputLimitSource: 'unknown',
    capsSource: '实测探测',
    errors: [],
    elapsedMs: 0,
    finalStage: 'done',
    ...overrides,
  }
}

function makeContext(overrides: Partial<GenerateContext> = {}): GenerateContext {
  return {
    baseUrl: 'http://192.168.66.202:8765/v1',
    models: [
      makeModel('deepseek-chat', {
        supportsTools: 'supported',
        contextLimit: 64000,
        maxOutputLimit: 8192,
      }),
      makeModel('deepseek-reasoner', { isReasoning: true }),
    ],
    apiKey: 'sk-test-key',
    ...overrides,
  }
}

/** 去掉 JSONC 行注释（区分字符串内外，避免误伤 URL 里的 //） */
function stripJsonc(input: string): string {
  let out = ''
  let inString = false
  let escaped = false
  for (let i = 0; i < input.length; i += 1) {
    const ch = input[i]
    if (inString) {
      out += ch
      if (escaped) escaped = false
      else if (ch === '\\') escaped = true
      else if (ch === '"') inString = false
      continue
    }
    if (ch === '"') {
      inString = true
      out += ch
      continue
    }
    if (ch === '/' && input[i + 1] === '/') {
      while (i < input.length && input[i] !== '\n') i += 1
      out += '\n'
      continue
    }
    out += ch
  }
  return out
}

const commentLines = (text: string) =>
  text.split('\n').filter((line) => line.trim().startsWith('//')).length

describe('密钥写入方式', () => {
  it('plain 模式写明文 apiKey，不写 apiKeyEnv', () => {
    const config = buildHardnessConfig(makeContext({ apiKey: 'sk-abc', apiKeyMode: 'plain' }))
    const provider = config.providers[0]
    expect(provider.apiKey).toBe('sk-abc')
    expect(provider).not.toHaveProperty('apiKeyEnv')
  })

  it('env 模式只写 apiKeyEnv，明文不出现在任何地方', () => {
    const config = buildHardnessConfig(
      makeContext({ apiKey: 'sk-secret', apiKeyMode: 'env', providerId: 'deepseek-local' }),
    )
    const provider = config.providers[0]
    expect(provider.apiKeyEnv).toBe('DEEPSEEK_LOCAL_API_KEY')
    expect(provider).not.toHaveProperty('apiKey')
    expect(JSON.stringify(config)).not.toContain('sk-secret')
  })

  it('未填密钥时不写任何密钥字段', () => {
    const provider = buildHardnessConfig(makeContext({ apiKey: '' })).providers[0]
    expect(provider).not.toHaveProperty('apiKey')
    expect(provider).not.toHaveProperty('apiKeyEnv')
  })

  it('三种渲染形态都带上密钥信息', () => {
    const context = makeContext({ apiKey: 'sk-abc' })
    expect(renderHardnessJsonc(context)).toContain('"apiKey": "sk-abc"')
    expect(renderHardnessJson(context)).toContain('"apiKey": "sk-abc"')
    expect(renderHardnessToml(context)).toContain('apiKey = "sk-abc"')
  })

  it('env 模式下三种形态都写环境变量名', () => {
    const context = makeContext({
      apiKey: '',
      apiKeyMode: 'env',
      providerId: 'deepseek-local',
    })
    expect(renderHardnessJsonc(context)).toContain('"apiKeyEnv": "DEEPSEEK_LOCAL_API_KEY"')
    expect(renderHardnessToml(context)).toContain('apiKeyEnv = "DEEPSEEK_LOCAL_API_KEY"')
  })
})

describe('provider 命名', () => {
  it('用户指定的 id 与显示名生效', () => {
    const provider = buildHardnessConfig(
      makeContext({ providerId: 'deepseek-local', providerName: 'DeepSeek Local' }),
    ).providers[0]
    expect(provider.id).toBe('deepseek-local')
    expect(provider.name).toBe('DeepSeek Local')
  })

  it('未指定时按 Base URL 推断', () => {
    const provider = buildHardnessConfig(makeContext()).providers[0]
    expect(provider.id).toBe('local')
    expect(provider.name).toBe('Local')
  })

  it('渲染文本里能看到 id 与显示名', () => {
    const text = renderHardnessJsonc(
      makeContext({ providerId: 'deepseek-local', providerName: 'DeepSeek Local' }),
    )
    expect(text).toContain('"id": "deepseek-local"')
    expect(text).toContain('"name": "DeepSeek Local"')
  })
})

describe('模型条目', () => {
  it('上下文/输出未知时补保守兜底值（此格式下两个字段是必填）', () => {
    const entry = buildHardnessConfig(makeContext({ models: [makeModel('m')] })).providers[0]
      .models[0]
    expect(entry.contextWindow).toBeGreaterThan(0)
    expect(entry.maxOutputTokens).toBeGreaterThan(0)
  })

  it('能力三态 unknown 一律按 false 处理（不确定时宁可关掉）', () => {
    const entry = buildHardnessConfig(makeContext({ models: [makeModel('m')] })).providers[0]
      .models[0]
    expect(entry.supportsToolCall).toBe(false)
    expect(entry.supportsVision).toBe(false)
  })

  it('推理标记与探测结果一致', () => {
    const entries = buildHardnessConfig(
      makeContext({ models: [makeModel('a'), makeModel('b', { isReasoning: true })] }),
    ).providers[0].models
    expect(entries[0].isReasoning).toBe(false)
    expect(entries[1].isReasoning).toBe(true)
  })
})

describe('注释精简（回归：用户要求不要每个模型都写注释）', () => {
  it('注释行数不随模型数增长', () => {
    const one = renderHardnessJsonc(makeContext({ models: [makeModel('a')] }))
    const many = renderHardnessJsonc(
      makeContext({ models: Array.from({ length: 40 }, (_, i) => makeModel(`m${i}`)) }),
    )
    expect(commentLines(many)).toBe(commentLines(one))
    expect(commentLines(many)).toBeLessThanOrEqual(12)
  })

  it('旧的逐条探测注释已移除', () => {
    const text = renderHardnessJsonc(makeContext())
    expect(text).not.toContain('探测结果：')
    expect(text).not.toContain('推理判定依据：')
  })
})

describe('文本形态可解析', () => {
  it('JSONC 去掉注释后能被 JSON.parse 解析', () => {
    const text = renderHardnessJsonc(makeContext())
    expect(() => JSON.parse(stripJsonc(text))).not.toThrow()
  })

  it('JSONC 与严格 JSON 结构一致', () => {
    const context = makeContext({ providerId: 'deepseek-local' })
    expect(JSON.parse(stripJsonc(renderHardnessJsonc(context)))).toEqual(
      JSON.parse(renderHardnessJson(context)),
    )
  })

  it('模型 id / 名称里的引号与反斜杠被转义', () => {
    const context = makeContext({
      models: [makeModel('a"b\\c', { displayName: '带"引号"\\' })],
    })
    const parsed = JSON.parse(stripJsonc(renderHardnessJsonc(context)))
    expect(parsed.providers[0].models[0].id).toBe('a"b\\c')
    expect(parsed.providers[0].models[0].name).toBe('带"引号"\\')
  })

  it('三种形态都不含未替换的占位符', () => {
    const context = makeContext()
    expect(validateNoPlaceholders(renderHardnessJsonc(context))).toEqual([])
    expect(validateNoPlaceholders(renderHardnessJson(context))).toEqual([])
    expect(validateNoPlaceholders(renderHardnessToml(context))).toEqual([])
  })

  it('模型数为 0 时三种形态都不崩', () => {
    const context = makeContext({ models: [] })
    expect(() => renderHardnessJsonc(context)).not.toThrow()
    expect(() => renderHardnessJson(context)).not.toThrow()
    expect(() => renderHardnessToml(context)).not.toThrow()
  })
})
