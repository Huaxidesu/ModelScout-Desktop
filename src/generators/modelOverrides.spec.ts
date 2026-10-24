/**
 * 模型参数覆盖（编辑投影 / 写回）单元测试。
 *
 * 核心语义（配置管理「编辑」与一键填写「参数调整」共用的行为契约）：
 *   1. 提取：v1 / v2 / dsh 三种形态都投影成统一行；「未设置」→ null；
 *   2. 写回：**只写动过的字段** —— 未改动的字段不重写，避免把「未设置」
 *      （跟随工具默认判定）覆盖成明确值；
 *   3. v1/v2 的 limit 必须成对（schema 约束）。
 */

import { describe, expect, it } from 'vitest'

import type { ModelInfo } from '@/types/contracts'
import {
  applyDshRows,
  applyOpenCodeRows,
  applyRowsToProbeModels,
  extractDshRows,
  extractOpenCodeRows,
  rowsFromProbeModels,
} from './modelOverrides'

function makeModel(id: string, overrides: Partial<ModelInfo> = {}): ModelInfo {
  return {
    id,
    displayName: id,
    ownedBy: '',
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
    capsSource: '',
    errors: [],
    elapsedMs: 0,
    finalStage: 'done',
    ...overrides,
  }
}

describe('extractOpenCodeRows', () => {
  it('v1:tool_call/attachment/reasoning/limit 全量提取,未设置字段为 null', () => {
    const provider = {
      models: {
        full: { tool_call: true, attachment: false, reasoning: true, limit: { context: 65536, output: 8192 } },
        bare: {},
      },
    }
    const rows = extractOpenCodeRows(provider, 'v1')
    expect(rows[0]).toMatchObject({
      id: 'full', reasoning: true, vision: false, tools: true, contextWindow: 65536, maxOutput: 8192,
    })
    expect(rows[1]).toMatchObject({
      id: 'bare', reasoning: null, vision: null, tools: null, contextWindow: null, maxOutput: null,
    })
  })

  it('v2:capabilities.tools/input 投影,reasoning 恒为 null', () => {
    const provider = {
      models: {
        m: { modelID: 'm', capabilities: { tools: true, input: ['text', 'image'] }, limit: { context: 1, output: 2 } },
      },
    }
    const rows = extractOpenCodeRows(provider, 'v2')
    expect(rows[0]).toMatchObject({ id: 'm', reasoning: null, vision: true, tools: true, contextWindow: 1, maxOutput: 2 })
  })
})

describe('applyOpenCodeRows —— 只写动过的字段', () => {
  const provider = {
    npm: '@ai-sdk/openai-compatible',
    models: {
      m: { tool_call: true, limit: { context: 100, output: 200 } },
    },
  }
  const originals = extractOpenCodeRows(provider, 'v1')

  it('未改动的字段原样保留(包括「未设置」)', () => {
    const rows = JSON.parse(JSON.stringify(originals)) as typeof originals
    rows[0].tools = true // 拨到与现值相同 → 不算改动
    const next = applyOpenCodeRows(provider, rows, originals, 'v1')
    expect((next.models as Record<string, Record<string, unknown>>).m).toEqual({
      tool_call: true,
      limit: { context: 100, output: 200 },
    })
  })

  it('改了 context 而 output 原本缺失时,limit 成对补齐(回退值)', () => {
    const bare = { npm: 'x', models: { m: {} } }
    const orig = extractOpenCodeRows(bare, 'v1')
    const rows = JSON.parse(JSON.stringify(orig)) as typeof orig
    rows[0].contextWindow = 131072
    const next = applyOpenCodeRows(bare, rows, orig, 'v1')
    const entry = (next.models as Record<string, Record<string, unknown>>).m
    expect(entry.limit).toEqual({ context: 131072, output: 4096 })
    // 没动过的 tool_call 等仍是未设置
    expect(entry.tool_call).toBeUndefined()
  })

  it('v2:改 vision → input 列表切换;改 tools → capabilities.tools', () => {
    const v2 = { package: 'aisdk:x', models: { m: { capabilities: { tools: false, input: ['text'] } } } }
    const orig = extractOpenCodeRows(v2, 'v2')
    const rows = JSON.parse(JSON.stringify(orig)) as typeof orig
    rows[0].vision = true
    rows[0].tools = true
    const next = applyOpenCodeRows(v2, rows, orig, 'v2')
    const entry = (next.models as Record<string, { capabilities: Record<string, unknown> }>).m
    expect(entry.capabilities.input).toEqual(['text', 'image'])
    expect(entry.capabilities.tools).toBe(true)
  })
})

describe('extractDshRows / applyDshRows', () => {
  const provider = {
    models: [
      { id: 'a', contextWindow: 8192, maxTokens: 4096 },
      { id: 'b', input: ['text', 'image'], reasoningEfforts: { off: null, high: 'high' } },
    ],
  }

  it('提取:reasoningEfforts 存在即推理;input 含 image 即视觉', () => {
    const rows = extractDshRows(provider)
    expect(rows[0]).toMatchObject({ id: 'a', reasoning: null, vision: null, contextWindow: 8192, maxOutput: 4096 })
    expect(rows[1]).toMatchObject({ id: 'b', reasoning: true, vision: true })
  })

  it('写回:开/关能力 → 加/删字段;未动的字段不重写', () => {
    const originals = extractDshRows(provider)
    const rows = JSON.parse(JSON.stringify(originals)) as typeof originals
    rows[0].reasoning = true
    rows[1].vision = false
    rows[1].reasoning = false
    const next = applyDshRows(provider, rows, originals)
    const models = next.models as Array<Record<string, unknown>>
    expect(models[0].reasoningEfforts).toEqual({ off: null, high: 'high' })
    expect(models[1].input).toBeUndefined()
    expect(models[1].reasoningEfforts).toBeUndefined()
    // 没动过的数值仍在,未设置的数值仍未设置
    expect(models[0].contextWindow).toBe(8192)
    expect(models[1].contextWindow).toBeUndefined()
  })
})

describe('探测数据投影(一键填写)', () => {
  it('rowsFromProbeModels + applyRowsToProbeModels 覆盖回 ModelInfo', () => {
    const models = [
      makeModel('m1', { isReasoning: true, supportsVision: 'supported', contextLimit: 65536 }),
      makeModel('m2'),
    ]
    const rows = rowsFromProbeModels(models)
    expect(rows[0]).toMatchObject({ id: 'm1', reasoning: true, vision: true, tools: false, contextWindow: 65536 })
    expect(rows[1].contextWindow).toBe(8192) // 未知上限 → 回退初值

    rows[1].tools = true
    rows[1].contextWindow = 131072
    const applied = applyRowsToProbeModels(models, rows)
    expect(applied[1].supportsTools).toBe('supported')
    expect(applied[1].contextLimit).toBe(131072)
    // 原数组不被修改
    expect(models[1].supportsTools).toBe('unknown')
  })
})
