/**
 * DeepSeek Harness (dsh) 生成器单元测试。
 *
 * schema 依据：deepseek-harness 官方 config-catalog 的
 * PiAiProviderProfile / PiAiModelProfile（核对日期 2026-10-25）。
 * 字段名错误会直接导致 dsh 拒绝加载配置，因此用键名白名单锁住。
 */

import { describe, expect, it } from 'vitest'
import { load as yamlLoad } from 'js-yaml'

import type { ModelInfo } from '@/types/contracts'

import {
  buildDshProviderProfile,
  mergeDshIntoText,
  renderDshPatchYml,
  renderDshProviderFragmentYml,
  DSH_LLM_PLUGIN_ID,
} from './dsh'

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

function makeContext() {
  return {
    baseUrl: 'https://api.example.com/v1',
    models: [
      makeModel('deepseek-chat', {
        displayName: 'DeepSeek Chat',
        supportsTools: 'supported',
        contextLimit: 64000,
        maxOutputLimit: 8192,
      }),
      makeModel('deepseek-vl', {
        displayName: 'DeepSeek VL',
        supportsVision: 'supported',
        contextLimit: 64000,
        maxOutputLimit: 8192,
      }),
      makeModel('deepseek-reasoner', {
        isReasoning: true,
        contextLimit: 64000,
        maxOutputLimit: 8192,
      }),
    ],
    apiKey: 'sk-test-key',
    providerKey: 'my-gateway',
    providerName: 'My Gateway',
  }
}

/** provider 条目允许的键（PiAiProviderProfile 的子集 + 官方字段） */
const PROVIDER_KEYS = new Set([
  'apiKeyEnv',
  'displayName',
  'api',
  'baseURL',
  'compat',
  'models',
  'modelOverrides',
  'defaultContextWindow',
  'defaultMaxTokens',
  'defaultInput',
  'headers',
  'reasoning',
  'thinkingBudgets',
  'cacheRetention',
  'transport',
  'timeoutMs',
  'websocketConnectTimeoutMs',
  'streamIdleTimeoutMs',
  'maxRequestImageBytes',
  'requestImagePixelBudget',
  'requestImageMaxBytes',
  'retryPolicy',
])

/** 模型条目允许的键（PiAiModelProfile） */
const MODEL_KEYS = new Set(['id', 'name', 'contextWindow', 'maxTokens', 'input', 'reasoningEfforts', 'compat'])

describe('buildDshProviderProfile', () => {
  it('字段名与官方 schema 一致', () => {
    const profile = buildDshProviderProfile(makeContext())
    expect(Object.keys(profile).every((k) => PROVIDER_KEYS.has(k))).toBe(true)
    expect(Object.keys(profile.models[0]).every((k) => MODEL_KEYS.has(k))).toBe(true)
  })

  it('OpenAI 兼容网关：openai-completions + compat 开关 + env 引用', () => {
    const profile = buildDshProviderProfile(makeContext())
    expect(profile.api).toBe('openai-completions')
    expect(profile.compat).toEqual({
      supportsDeveloperRole: false,
      maxTokensField: 'max_tokens',
    })
    expect(profile.apiKeyEnv).toBeTruthy() // 不写明文 Key
  })

  it('Anthropic 官方端点 → anthropic-messages 且无 compat', () => {
    const profile = buildDshProviderProfile({
      ...makeContext(),
      baseUrl: 'https://api.anthropic.com',
    })
    expect(profile.api).toBe('anthropic-messages')
    expect(profile.compat).toBeUndefined()
  })

  it('视觉模型带 input，推理模型带 reasoningEfforts，普通模型都不带', () => {
    const [chat, vl, reasoner] = buildDshProviderProfile(makeContext()).models
    expect(chat.input).toBeUndefined()
    expect(chat.reasoningEfforts).toBeUndefined()
    expect(vl.input).toEqual(['text', 'image'])
    expect(reasoner.reasoningEfforts).toHaveProperty('off', null)
  })

  it('上限未知时写保守回退值而不是凭空大数', () => {
    const profile = buildDshProviderProfile({
      ...makeContext(),
      models: [makeModel('unknown-model')],
    })
    expect(profile.models[0].contextWindow).toBe(8192)
    expect(profile.models[0].maxTokens).toBe(4096)
  })
})

describe('renderDshPatchYml（新建文件）', () => {
  it('输出是合法 YAML，结构为插件补丁数组', () => {
    const text = renderDshPatchYml(makeContext())
    const doc = yamlLoad(text) as Array<{ id: string; config: { providers: Record<string, unknown> } }>
    expect(Array.isArray(doc)).toBe(true)
    expect(doc[0].id).toBe(DSH_LLM_PLUGIN_ID)
    expect(doc[0].config.providers['my-gateway']).toBeDefined()
  })

  it('头注释包含密钥环境变量说明', () => {
    const text = renderDshPatchYml(makeContext())
    expect(text).toContain('apiKeyEnv')
    expect(text).toContain('环境变量')
  })
})

describe('renderDshProviderFragmentYml（片段）', () => {
  it('顶层是 providers 映射，可整体解析', () => {
    const text = renderDshProviderFragmentYml(makeContext())
    const doc = yamlLoad(text) as { providers: Record<string, unknown> }
    expect(doc.providers['my-gateway']).toBeDefined()
  })
})

describe('mergeDshIntoText（并入已有补丁）', () => {
  it('已有 llm-pi-ai 条目：替换同名 provider，保留其他 provider 与其他插件条目', () => {
    const existing = [
      { id: 'some-other-plugin', config: { keep: true } },
      {
        id: DSH_LLM_PLUGIN_ID,
        config: {
          providers: {
            'my-gateway': { apiKeyEnv: 'OLD', api: 'openai-completions', baseURL: 'https://old/v1', models: [{ id: 'stale' }] },
            'another-gw': { api: 'openai-completions' },
          },
        },
      },
    ]
    const result = mergeDshIntoText(
      // 模拟磁盘上的 YAML 文本
      [
        '- id: some-other-plugin',
        '  config:',
        '    keep: true',
        `- id: ${DSH_LLM_PLUGIN_ID}`,
        '  config:',
        '    providers:',
        '      my-gateway:',
        '        apiKeyEnv: OLD',
        '        api: openai-completions',
        '        baseURL: https://old/v1',
        '      another-gw:',
        '        api: openai-completions',
      ].join('\n'),
      makeContext(),
    )
    expect(result.mode).toBe('merge')

    const doc = yamlLoad(result.text) as Array<Record<string, unknown>>
    expect(doc).toHaveLength(2)
    expect((doc[0] as { id: string }).id).toBe('some-other-plugin')
    const piAi = doc[1] as {
      id: string
      config: { providers: Record<string, { apiKeyEnv?: string; baseURL?: string; models?: Array<{ id: string }> }> }
    }
    expect(piAi.id).toBe(DSH_LLM_PLUGIN_ID)
    // 同名替换
    expect(piAi.config.providers['my-gateway'].baseURL).toBe('https://api.example.com/v1')
    expect(piAi.config.providers['my-gateway'].apiKeyEnv).not.toBe('OLD')
    // 其他 provider 保留
    expect(piAi.config.providers['another-gw']).toEqual({ api: 'openai-completions' })
    // 会提示注释丢失
    expect(result.warnings.join()).toContain('注释')
    void existing
  })

  it('没有 llm-pi-ai 条目 → 追加一个，原条目不动', () => {
    const existing = '- id: some-other-plugin\n  config:\n    keep: true\n'
    const result = mergeDshIntoText(existing, makeContext())
    expect(result.mode).toBe('merge')
    const doc = yamlLoad(result.text) as Array<{ id: string }>
    expect(doc.map((e) => e.id)).toEqual(['some-other-plugin', DSH_LLM_PLUGIN_ID])
  })

  it('空文件 → create（全新补丁）', () => {
    const result = mergeDshIntoText('', makeContext())
    expect(result.mode).toBe('create')
    expect(yamlLoad(result.text)).toBeInstanceOf(Array)
  })

  it('非法 YAML 抛错', () => {
    expect(() => mergeDshIntoText('a: [unclosed', makeContext())).toThrow(/YAML/)
  })

  it('顶层不是数组抛错', () => {
    expect(() => mergeDshIntoText('just_a_string', makeContext())).toThrow(/数组/)
  })

  it('只有注释的文件抛错（不静默覆盖用户注释）', () => {
    expect(() => mergeDshIntoText('# only comments\n', makeContext())).toThrow(/可解析/)
  })
})
