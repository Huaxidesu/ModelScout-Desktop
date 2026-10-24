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

import { mergeOpenCodeIntoText } from './opencodeMerge'
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
    const parsed = parseJsonc(result.text) as Record<string, unknown>
    expect(parsed.mcp).toEqual({ fetch: { command: 'uvx' } })
    expect(parsed.provider).toBeDefined()
  })

  it('模型 ID 含引号等特殊字符时输出仍可解析', () => {
    const result = mergeOpenCodeIntoText('', makeContext({
      models: [makeModel('we"ird\\id', {})],
    }))
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
