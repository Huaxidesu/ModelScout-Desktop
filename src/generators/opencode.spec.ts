/**
 * OpenCode 配置生成器单元测试。
 *
 * 配置生成器是本应用的直接交付物——用户拿走的就是它输出的文本，
 * 字段名错误、密钥缺失等问题只能靠用例发现。模型能力开关曾被写成
 * 嵌套的 `capabilities` 对象，而官方 schema 对模型条目设了
 * `additionalProperties: false`，该键非法且会导致整份配置校验失败；
 * 「模型条目字段」一组用例即为此类错误设置。
 *
 * schema 依据：https://opencode.ai/config.json
 * 键名白名单见 `OFFICIAL_MODEL_KEYS`，核对日期 2026-10-10。
 * 官方 schema 变更时这里会失败，此即本测试期望的提醒方式。
 */

import { describe, expect, it } from 'vitest'

import type { ModelInfo } from '@/types/contracts'
import {
  buildModelEntry,
  buildOpenCodeConfig,
  renderOpenCodeJson,
  renderOpenCodeJsonc,
  renderOpenCodeProviderFragment,
  validateNoPlaceholders,
  type GenerateContext,
} from './opencode'

// ---- 测试数据工厂 ----

/** 构造字段完整的 ModelInfo，避免每个用例写 17 个字段 */
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

/** 构造一个典型上下文 */
function makeContext(overrides: Partial<GenerateContext> = {}): GenerateContext {
  return {
    baseUrl: 'http://192.168.66.202:8765/v1',
    models: [
      makeModel('deepseek-chat', {
        displayName: 'DeepSeek Chat',
        supportsTools: 'supported',
        supportsVision: 'unsupported',
        contextLimit: 64000,
        maxOutputLimit: 8192,
      }),
      makeModel('deepseek-reasoner', {
        displayName: 'DeepSeek Reasoner',
        supportsTools: 'supported',
        isReasoning: true,
        contextLimit: 64000,
        maxOutputLimit: 8192,
      }),
    ],
    apiKey: 'sk-test-key',
    ...overrides,
  }
}

/**
 * OpenCode 官方 schema 中**模型条目**允许的全部键名。
 * 取自 https://opencode.ai/config.json 的
 * `$defs.ProviderConfig.properties.models.additionalProperties.properties`
 */
const OFFICIAL_MODEL_KEYS = new Set([
  'id',
  'name',
  'family',
  'release_date',
  'attachment',
  'reasoning',
  'temperature',
  'tool_call',
  'interleaved',
  'cost',
  'limit',
  'modalities',
  'experimental',
  'status',
  'provider',
  'options',
  'headers',
  'variants',
])

/** OpenCode 官方 schema 中**provider 条目**允许的全部键名 */
const OFFICIAL_PROVIDER_KEYS = new Set([
  'api',
  'name',
  'env',
  'id',
  'npm',
  'whitelist',
  'blacklist',
  'options',
  'models',
])

/**
 * 去掉 JSONC 的行注释，得到可 `JSON.parse` 的严格 JSON。
 *
 * 必须**区分字符串内外的 `//`** —— 本用例的 baseURL 是
 * `http://192.168.66.202:8765/v1`，朴素地按 `//` 切会把 URL 拦腰截断。
 */
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

// ---- 模型条目字段 ----

describe('buildModelEntry —— 模型条目必须是官方顶层字段', () => {
  it('能力开关平铺在顶层，而不是塞进 capabilities 嵌套对象', () => {
    const entry = buildModelEntry(
      makeModel('m', {
        supportsTools: 'supported',
        supportsVision: 'supported',
        isReasoning: true,
      }),
    )

    expect(entry.tool_call).toBe(true)
    expect(entry.attachment).toBe(true)
    expect(entry.reasoning).toBe(true)
    // 这是回归断言：capabilities 不在官方 schema 的允许键里
    expect(entry).not.toHaveProperty('capabilities')
  })

  it('条目里出现的每一个键都在官方 schema 的允许集合内', () => {
    const entry = buildModelEntry(
      makeModel('m', {
        supportsTools: 'supported',
        supportsVision: 'unsupported',
        isReasoning: true,
        contextLimit: 128000,
        maxOutputLimit: 4096,
      }),
    )

    for (const key of Object.keys(entry)) {
      expect(OFFICIAL_MODEL_KEYS, `模型条目出现了非法键：${key}`).toContain(key)
    }
  })

  it('能力为 unknown 时省略该字段，让 OpenCode 用自己的默认判定', () => {
    const entry = buildModelEntry(makeModel('m'))
    expect(entry).not.toHaveProperty('tool_call')
    expect(entry).not.toHaveProperty('attachment')
    expect(entry).not.toHaveProperty('reasoning')
  })

  it('明确不支持时写入 false（与 unknown 的省略语义区分开）', () => {
    const entry = buildModelEntry(
      makeModel('m', { supportsTools: 'unsupported', supportsVision: 'unsupported' }),
    )
    expect(entry.tool_call).toBe(false)
    expect(entry.attachment).toBe(false)
  })

  it('非推理模型不写 reasoning 字段', () => {
    expect(buildModelEntry(makeModel('m', { isReasoning: false }))).not.toHaveProperty('reasoning')
  })

  it('同时写 id 与 name（id 与 models 里的 key 一致）', () => {
    const entry = buildModelEntry(makeModel('glm-5.3', { displayName: 'GLM 5.3' }))
    expect(entry.id).toBe('glm-5.3')
    expect(entry.name).toBe('GLM 5.3')
  })

  it('displayName 为空或与 id 相同时，name 回退为 id', () => {
    expect(buildModelEntry(makeModel('m', { displayName: '' })).name).toBe('m')
    expect(buildModelEntry(makeModel('m', { displayName: 'm' })).name).toBe('m')
  })
})

describe('limit 字段 —— schema 要求 context 与 output 同时存在', () => {
  it('两个上限都已知时原样写入', () => {
    const entry = buildModelEntry(makeModel('m', { contextLimit: 64000, maxOutputLimit: 8192 }))
    expect(entry.limit).toEqual({ context: 64000, output: 8192 })
  })

  it('只知其一也要把另一个补上兜底值（不能只写一个）', () => {
    const entry = buildModelEntry(makeModel('m', { contextLimit: 0, maxOutputLimit: 8192 }))
    expect(entry.limit).toBeDefined()
    expect(entry.limit!.output).toBe(8192)
    expect(entry.limit!.context).toBeGreaterThan(0)
  })

  it('两个都未知时完全不写 limit，而不是编造数字', () => {
    const entry = buildModelEntry(makeModel('m', { contextLimit: 0, maxOutputLimit: 0 }))
    expect(entry).not.toHaveProperty('limit')
  })

  it('limit 内部只含 context 与 output（schema 是 additionalProperties:false）', () => {
    const entry = buildModelEntry(makeModel('m', { contextLimit: 1000, maxOutputLimit: 100 }))
    expect(Object.keys(entry.limit!).sort()).toEqual(['context', 'output'])
  })
})

// ---- provider 层：命名与密钥 ----

describe('provider 层 —— 命名与密钥', () => {
  it('provider 条目的每一个键都在官方允许集合内', () => {
    const config = buildOpenCodeConfig(makeContext({ providerKey: 'deepseek-local' }))
    const provider = config.provider['deepseek-local']
    for (const key of Object.keys(provider)) {
      expect(OFFICIAL_PROVIDER_KEYS, `provider 出现了非法键：${key}`).toContain(key)
    }
  })

  it('密钥写进 options.apiKey，与 baseURL 并列', () => {
    const config = buildOpenCodeConfig(makeContext({ apiKey: 'sk-abc123' }))
    const options = config.provider['local'].options
    expect(options.apiKey).toBe('sk-abc123')
    expect(options.baseURL).toBe('http://192.168.66.202:8765/v1')
  })

  it('用户自定义的键名与显示名生效', () => {
    const config = buildOpenCodeConfig(
      makeContext({ providerKey: 'deepseek-local', providerName: 'DeepSeek Local' }),
    )
    expect(Object.keys(config.provider)).toEqual(['deepseek-local'])
    expect(config.provider['deepseek-local'].name).toBe('DeepSeek Local')
  })

  it('未指定命名时按 Base URL 自动推断（192.168.x.x → local / Local）', () => {
    const config = buildOpenCodeConfig(makeContext())
    expect(config.provider.local).toBeDefined()
    expect(config.provider.local.name).toBe('Local')
  })

  it('未指定命名时对公网域名推断出可读名字', () => {
    const config = buildOpenCodeConfig(
      makeContext({ baseUrl: 'https://discovery-api.intern-ai.org.cn/v1' }),
    )
    expect(config.provider['intern-ai']).toBeDefined()
    expect(config.provider['intern-ai'].name).toBe('Intern AI')
  })

  it('密钥为空时完全不写 apiKey 字段（写空串会导致鉴权失败）', () => {
    const config = buildOpenCodeConfig(makeContext({ apiKey: '' }))
    expect(config.provider.local.options).not.toHaveProperty('apiKey')
  })

  it('env 模式下写 {env:VAR} 引用，且不泄露明文', () => {
    const config = buildOpenCodeConfig(
      makeContext({ apiKey: 'sk-secret', apiKeyMode: 'env', providerKey: 'deepseek-local' }),
    )
    const options = config.provider['deepseek-local'].options
    expect(options.apiKey).toBe('{env:DEEPSEEK_LOCAL_API_KEY}')
    expect(JSON.stringify(config)).not.toContain('sk-secret')
  })

  it('npm 字段按端点类型选择（OpenAI 兼容端点用兼容包）', () => {
    expect(buildOpenCodeConfig(makeContext()).provider.local.npm).toBe(
      '@ai-sdk/openai-compatible',
    )
    expect(
      buildOpenCodeConfig(makeContext({ baseUrl: 'https://api.anthropic.com/v1' })).provider[
        'anthropic'
      ].npm,
    ).toBe('@ai-sdk/anthropic')
  })

  it('顶层带 $schema，指向官方 schema 地址', () => {
    expect(buildOpenCodeConfig(makeContext()).$schema).toBe('https://opencode.ai/config.json')
  })
})

// ---- JSONC 文本渲染 ----

describe('renderOpenCodeJsonc —— 文本形态', () => {
  it('去掉注释后能被 JSON.parse 解析（URL 里的 // 不能误伤）', () => {
    const text = renderOpenCodeJsonc(makeContext())
    expect(() => JSON.parse(stripJsonc(text))).not.toThrow()
  })

  it('JSONC 与严格 JSON 的结构完全一致', () => {
    const context = makeContext({ providerKey: 'deepseek-local', providerName: 'DeepSeek Local' })
    const fromJsonc = JSON.parse(stripJsonc(renderOpenCodeJsonc(context)))
    const fromJson = JSON.parse(renderOpenCodeJson(context))
    expect(fromJsonc).toEqual(fromJson)
  })

  it('模型 id 与显示名里的引号、反斜杠被正确转义', () => {
    const context = makeContext({
      models: [
        makeModel('weird"id\\x', {
          displayName: '带"引号"的名字\\',
          contextLimit: 1000,
          maxOutputLimit: 100,
        }),
      ],
    })
    const parsed = JSON.parse(stripJsonc(renderOpenCodeJsonc(context)))
    const models = parsed.provider.local.models
    expect(models['weird"id\\x'].name).toBe('带"引号"的名字\\')
  })

  it('不再给每个模型写注释 —— 注释行数不随模型数增长', () => {
    const one = renderOpenCodeJsonc(makeContext({ models: [makeModel('a')] }))
    const many = renderOpenCodeJsonc(
      makeContext({ models: Array.from({ length: 40 }, (_, i) => makeModel(`m${i}`)) }),
    )

    const commentCount = (text: string) =>
      text.split('\n').filter((line) => line.trim().startsWith('//')).length

    // 40 个模型的注释行数不应比 1 个模型多
    expect(commentCount(many)).toBe(commentCount(one))
    // 且总量保持在一个很小的常数级
    expect(commentCount(many)).toBeLessThanOrEqual(10)
  })

  it('旧的逐条探测注释已移除（回归断言）', () => {
    const text = renderOpenCodeJsonc(makeContext())
    expect(text).not.toContain('探测结果：')
    expect(text).not.toContain('推理判定：')
  })

  it('明文模式下给出「含明文密钥」的醒目提示', () => {
    const text = renderOpenCodeJsonc(makeContext({ apiKey: 'sk-x', apiKeyMode: 'plain' }))
    expect(text).toContain('明文 API Key')
  })

  it('env 模式下提示需要设置的环境变量名', () => {
    const text = renderOpenCodeJsonc(
      makeContext({ apiKey: '', apiKeyMode: 'env', providerKey: 'deepseek-local' }),
    )
    expect(text).toContain('DEEPSEEK_LOCAL_API_KEY')
    expect(text).not.toContain('明文 API Key')
  })

  it('未填密钥时明确告知会回退到其它鉴权来源', () => {
    const text = renderOpenCodeJsonc(makeContext({ apiKey: '' }))
    expect(text).toContain('未填写 API Key')
  })

  it('输出中不存在未替换的占位符', () => {
    const text = renderOpenCodeJsonc(makeContext())
    expect(validateNoPlaceholders(text)).toEqual([])
  })
})

// ---- 片段形态（粘贴进已有配置） ----

describe('renderOpenCodeProviderFragment —— 可粘贴片段', () => {
  it('片段不含根大括号与 $schema，包一层 {} 后即为合法 JSON', () => {
    const fragment = renderOpenCodeProviderFragment(makeContext())
    const stripped = stripJsonc(fragment)

    expect(stripped).not.toContain('$schema')
    // 片段自己不能有根大括号 —— 有的话粘进已有配置就会多一层
    expect(stripped.trimStart().startsWith('"provider"')).toBe(true)

    expect(() => JSON.parse(`{${stripped}}`)).not.toThrow()
  })

  it('片段与完整配置的 provider 内容完全一致（防止两处渲染漂移）', () => {
    const context = makeContext({
      providerKey: 'intern-ai',
      providerName: 'Intern AI',
      apiKey: 'sk-abc',
      baseUrl: 'https://discovery-api.intern-ai.org.cn/v1',
    })

    const fromFragment = JSON.parse(`{${stripJsonc(renderOpenCodeProviderFragment(context))}}`)
    const fromFull = buildOpenCodeConfig(context)

    expect(fromFragment.provider).toEqual(fromFull.provider)
  })

  it('片段末尾给出「不是最后一项要补逗号」的提示', () => {
    const fragment = renderOpenCodeProviderFragment(makeContext())
    expect(fragment).toContain('补一个逗号')
  })

  it('片段不带尾逗号（避免恰好是最后一项时反而语法错误）', () => {
    const stripped = stripJsonc(renderOpenCodeProviderFragment(makeContext())).trimEnd()
    expect(stripped.endsWith('}')).toBe(true)
    expect(stripped.endsWith('},')).toBe(false)
  })

  it('env 模式下片段引用环境变量而非明文', () => {
    const fragment = renderOpenCodeProviderFragment(
      makeContext({
        apiKey: 'sk-secret',
        apiKeyMode: 'env',
        providerKey: 'deepseek-local',
      }),
    )
    expect(fragment).toContain('{env:DEEPSEEK_LOCAL_API_KEY}')
    expect(fragment).not.toContain('sk-secret')
  })

  it('片段里模型 ID 的引号同样被转义', () => {
    const fragment = renderOpenCodeProviderFragment(
      makeContext({ models: [makeModel('a"b\\c', { contextLimit: 1, maxOutputLimit: 1 })] }),
    )
    const parsed = JSON.parse(`{${stripJsonc(fragment)}}`)
    expect(Object.keys(parsed.provider.local.models)).toEqual(['a"b\\c'])
  })

  it('模型数为 0 时片段依然合法', () => {
    const fragment = renderOpenCodeProviderFragment(makeContext({ models: [] }))
    const parsed = JSON.parse(`{${stripJsonc(fragment)}}`)
    expect(parsed.provider.local.models).toEqual({})
  })
})

// ---- 端到端：与用户提供的参考格式逐字段对齐 ----

describe('端到端 —— 对齐用户提供的参考格式', () => {
  it('生成结果包含 name / options.apiKey / options.baseURL 三件套', () => {
    const config = buildOpenCodeConfig(
      makeContext({
        providerKey: 'deepseek-local',
        providerName: 'DeepSeek Local',
        apiKey: 'sk-deekseep-x4Adk2sGGpSvTUMAx-b1YYJPUEQl4dfmZCT38sSKTlc',
      }),
    )

    const provider = config.provider['deepseek-local']
    expect(provider.name).toBe('DeepSeek Local')
    expect(provider.options.baseURL).toBe('http://192.168.66.202:8765/v1')
    expect(provider.options.apiKey).toBe(
      'sk-deekseep-x4Adk2sGGpSvTUMAx-b1YYJPUEQl4dfmZCT38sSKTlc',
    )
    expect(Object.keys(provider.models)).toEqual(['deepseek-chat', 'deepseek-reasoner'])
  })

  it('渲染文本里能直接看到 provider 键名与显示名', () => {
    const text = renderOpenCodeJsonc(
      makeContext({ providerKey: 'deepseek-local', providerName: 'DeepSeek Local' }),
    )
    expect(text).toContain('"deepseek-local": {')
    expect(text).toContain('"name": "DeepSeek Local"')
    expect(text).toContain('"apiKey": "sk-test-key"')
  })

  it('模型数为 0 时不崩，产出合法的空 models 对象', () => {
    const config = buildOpenCodeConfig(makeContext({ models: [] }))
    expect(config.provider.local.models).toEqual({})
    const parsed = JSON.parse(stripJsonc(renderOpenCodeJsonc(makeContext({ models: [] }))))
    expect(parsed.provider.local.models).toEqual({})
  })
})
