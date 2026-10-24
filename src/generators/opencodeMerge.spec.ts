/**
 * OpenCode 合并逻辑单元测试。
 *
 * 核心验收点（对应「一键填写」的需求语义）：
 *   1. 空文件 / `{}` → 生成完整配置（create）；
 *   2. 已有配置 → 片段并入且**用户注释保留**（splice）；
 *      - 同名 provider 原位替换；
 *      - 无同名 provider 追加到 provider 对象末尾；
 *      - 顶层没有 provider 键时插入整个成员；
 *   3. 所有输出都是**可被 JSON.parse 的合法配置**（JSONC 注释不影响
 *      opencode 自己的解析器，但合法结构是最基本要求，splice 结果
 *      去注释后必须可解析）；
 *   4. 双逗号陷阱：原对象带尾逗号时插入不产生 `,,`。
 */

import { describe, expect, it } from 'vitest'

import type { ModelInfo } from '@/types/contracts'
import { parseJsonc } from '@/utils/jsonc'

import { mergeOpenCodeIntoText, removeOpenCodeProvider, upsertOpenCodeProvider } from './opencodeMerge'
import type { GenerateContext } from './opencode'

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
    baseUrl: 'https://api.example.com/v1',
    models: [
      makeModel('deepseek-chat', {
        displayName: 'DeepSeek Chat',
        supportsTools: 'supported',
        contextLimit: 64000,
        maxOutputLimit: 8192,
      }),
    ],
    apiKey: 'sk-test-key',
    providerKey: 'my-gateway',
    ...overrides,
  }
}

describe('mergeOpenCodeIntoText —— 空文件补全', () => {
  it('文件不存在 → 写入完整配置', () => {
    const result = mergeOpenCodeIntoText('', makeContext())
    expect(result.mode).toBe('create')
    const parsed = parseJsonc(result.text) as {
      $schema?: string
      provider: Record<string, unknown>
    }
    expect(parsed.provider['my-gateway']).toBeDefined()
    expect(parsed.$schema).toBe('https://opencode.ai/config.json')
  })

  it('文件是空对象 {} → 同样写完整配置', () => {
    const result = mergeOpenCodeIntoText('{\n  // 空的\n}\n', makeContext())
    expect(result.mode).toBe('create')
    expect(result.warnings).toEqual([])
  })
})

describe('mergeOpenCodeIntoText —— 已有配置并入', () => {
  it('同名 provider 原位替换，其他 provider 与顶层键、注释保留', () => {
    const existing = `{
  // 我的全局配置
  "permission": { "edit": "allow" },
  "provider": {
    // 另一个服务商，别动它
    "other-vendor": { "npm": "@ai-sdk/openai-compatible" },
    "my-gateway": { "npm": "@ai-sdk/openai-compatible", "models": {} }
  },
  "mcp": {}
}`
    const result = mergeOpenCodeIntoText(existing, makeContext())
    expect(result.mode).toBe('splice')

    // 注释保留
    expect(result.text).toContain('// 我的全局配置')
    expect(result.text).toContain('// 另一个服务商，别动它')
    // 其他顶层键保留
    const parsed = parseJsonc(result.text) as Record<string, unknown> & {
      provider: Record<string, Record<string, unknown>>
    }
    expect(parsed.permission).toEqual({ edit: 'allow' })
    expect(parsed.mcp).toEqual({})
    expect(Object.keys(parsed.provider)).toEqual(['other-vendor', 'my-gateway'])
    // 同名键被替换成新内容（模型来自探测结果）
    expect(parsed.provider['my-gateway'].models).toHaveProperty('deepseek-chat')
  })

  it('provider 表里无同名键 → 追加，保留原有键与尾逗号场景合法', () => {
    const existing = `{
  "provider": {
    "other": { "npm": "@ai-sdk/openai-compatible" },
  }
}`
    const result = mergeOpenCodeIntoText(existing, makeContext())
    expect(result.mode).toBe('splice')
    // 原对象已有尾逗号：插入后不允许出现 ",," 或裸逗号导致的解析失败
    expect(result.text).not.toContain(',,')
    const parsed = parseJsonc(result.text) as { provider: Record<string, unknown> }
    expect(Object.keys(parsed.provider)).toEqual(['other', 'my-gateway'])
  })

  it('顶层没有 provider 键 → 插入整个成员', () => {
    const existing = `{
  // 只有 mcp 配置
  "mcp": { "fetch": { "command": "uvx" } }
}`
    const result = mergeOpenCodeIntoText(existing, makeContext())
    expect(result.mode).toBe('splice')
    expect(result.text).toContain('// 只有 mcp 配置')
    const parsed = parseJsonc(result.text) as {
      mcp: unknown
      provider: Record<string, unknown>
    }
    expect(parsed.mcp).toEqual({ fetch: { command: 'uvx' } })
    // 必须是 provider -> my-gateway -> 条目 的两层结构
    expect(parsed.provider['my-gateway']).toBeDefined()
  })

  it('模型 ID 含引号等特殊字符时输出仍可解析', () => {
    const result = mergeOpenCodeIntoText('', makeContext({
      models: [makeModel('we"ird\\id', {})],
    }))
    expect(() => parseJsonc(result.text)).not.toThrow()
  })

  it('v2 文件（providers 容器）：按 v2 键名写入并转换字段', () => {
    // opencode 2 的真实形态（2026-10 迁移）：providers/package/settings/modelID
    const v2File = `{
  "plugins": ["x"],
  "providers": {
    "existing": { "package": "aisdk:@ai-sdk/anthropic", "settings": { "baseURL": "https://a" } }
  }
}`
    const result = mergeOpenCodeIntoText(v2File, makeContext())
    expect(result.mode).toBe('splice')

    const parsed = parseJsonc(result.text) as {
      providers: Record<string, Record<string, unknown>>
      plugins: unknown
    }
    // 写进的是 v2 容器,而不是另起一个 v1 的 provider 键
    expect(parsed.providers).toBeDefined()
    expect(Object.keys(parsed.providers)).toEqual(['existing', 'my-gateway'])
    expect(parsed.plugins).toEqual(['x'])

    const merged = parsed.providers['my-gateway'] as {
      package: string
      settings: Record<string, string>
      models: Record<string, Record<string, unknown>>
    }
    expect(merged.package).toBe('aisdk:@ai-sdk/openai-compatible')
    expect(merged.settings.baseURL).toBe('https://api.example.com/v1')
    expect(merged.settings.apiKey).toBe('sk-test-key')
    const model = merged.models['deepseek-chat'] as Record<string, unknown>
    expect(model.modelID).toBe('deepseek-chat')
    expect((model.capabilities as Record<string, unknown>).tools).toBe(true)
  })
})

describe('基础信息注释（providerComment）', () => {
  it('插入新 provider 时在键上方带注释行,输出仍可解析', () => {
    const existing = '{"mcp": {}}'
    const result = mergeOpenCodeIntoText(existing, makeContext(), {
      providerComment: 'My Gateway · 1 个模型 · ModelScout 2026-10-25',
    })
    expect(result.mode).toBe('splice')
    expect(result.text).toContain('// My Gateway · 1 个模型 · ModelScout 2026-10-25')
    expect(() => parseJsonc(result.text)).not.toThrow()
  })

  it('同名替换时更新已有标记注释而不是堆积', () => {
    const first = mergeOpenCodeIntoText(
      '{"mcp": {}}',
      makeContext(),
      { providerComment: '名字A · 1 个模型 · ModelScout 2026-10-25' },
    )
    const second = mergeOpenCodeIntoText(
      first.text,
      makeContext(),
      { providerComment: '名字B · 2 个模型 · ModelScout 2026-10-26' },
    )
    expect(second.text).toContain('名字B')
    expect(second.text).not.toContain('名字A')
    expect(second.text.match(/ModelScout 2026/g)?.length).toBe(1)
    expect(() => parseJsonc(second.text)).not.toThrow()
  })

  it('新建文件的顶部注释在最外层大括号之前', () => {
    const result = mergeOpenCodeIntoText('', makeContext(), {
      providerComment: 'Info · 1 个模型 · ModelScout',
    })
    expect(result.text.startsWith('// Info')).toBe(true)
    expect(() => parseJsonc(result.text)).not.toThrow()
  })
})

describe('mergeOpenCodeIntoText —— 异常输入', () => {
  it('非法 JSONC 抛 MergeError，不产出文本', () => {
    expect(() => mergeOpenCodeIntoText('{"a": oops}', makeContext())).toThrow(/合法/)
  })

  it('顶层是数组抛 MergeError', () => {
    expect(() => mergeOpenCodeIntoText('[1, 2]', makeContext())).toThrow(/顶层/)
  })
})

describe('upsertOpenCodeProvider / removeOpenCodeProvider（配置管理页）', () => {
  const existing = `{
  // 保留我
  "provider": {
    "keep-me": { "npm": "@ai-sdk/openai-compatible" },
    "edit-me": { "npm": "old", "options": { "baseURL": "https://old/v1" } }
  }
}`

  it('upsert 替换同名 provider，保留注释与其他条目', () => {
    const result = upsertOpenCodeProvider(existing, 'edit-me', {
      npm: 'new',
      options: { baseURL: 'https://new/v1', apiKey: 'sk-x' },
    })
    expect(result.mode).toBe('splice')
    expect(result.text).toContain('// 保留我')
    const parsed = parseJsonc(result.text) as { provider: Record<string, Record<string, unknown>> }
    expect(Object.keys(parsed.provider)).toEqual(['keep-me', 'edit-me'])
    expect((parsed.provider['edit-me'].options as Record<string, unknown>).apiKey).toBe('sk-x')
  })

  it('upsert 不存在的键则追加', () => {
    const result = upsertOpenCodeProvider(existing, 'brand-new', { npm: 'x' })
    const parsed = parseJsonc(result.text) as { provider: Record<string, unknown> }
    expect(Object.keys(parsed.provider)).toEqual(['keep-me', 'edit-me', 'brand-new'])
  })

  it('remove 摘除 provider，不留双逗号，注释保留', () => {
    const result = removeOpenCodeProvider(existing, 'edit-me')
    expect(result).not.toBeNull()
    expect(result!.text).not.toContain(',,')
    expect(result!.text).toContain('// 保留我')
    const parsed = parseJsonc(result!.text) as { provider: Record<string, unknown> }
    expect(Object.keys(parsed.provider)).toEqual(['keep-me'])
  })

  it('remove 中间成员时同样合法', () => {
    const three = '{"provider": {"a": {}, "b": {}, "c": {}}}'
    const result = removeOpenCodeProvider(three, 'b')
    const parsed = parseJsonc(result!.text) as { provider: Record<string, unknown> }
    expect(Object.keys(parsed.provider)).toEqual(['a', 'c'])
  })

  it('remove 不存在的键返回 null', () => {
    expect(removeOpenCodeProvider(existing, 'ghost')).toBeNull()
  })
})
