/**
 * 模型参数覆盖（配置管理「编辑」与一键填写「参数调整」共用）。
 *
 * 把 provider 里的每个模型条目投影成统一的可编辑行（思考/附件/工具/
 * 上下文/输出上限），编辑后按**文件本身的 schema** 写回原始对象。
 *
 * 关键语义 —— 「未设置」：
 * 配置里没写的字段（例如 v1 模型条目没写 tool_call）代表「跟随工具的
 * 默认判定」，强行写成 true/false 会改变行为。因此编辑行保留三态
 * （null = 未设置），保存时**只写用户动过的字段**（与打开时的原值
 * 逐字段比对），没动过的字段原样保留、新增字段不补写。
 */

import type { ModelInfo } from '@/types/contracts'
import type { OpenCodeV2ExtrasMap } from './opencode'

/** 一个模型的可编辑参数投影 */
export interface ModelEditRow {
  /** 模型条目键名（opencode 的 models key / dsh 的 model.id） */
  id: string
  /** 显示名（可空） */
  name: string
  /** 是否推理（思考）模型；null = 配置未设置 */
  reasoning: boolean | null
  /** 是否支持附件/图片输入；null = 配置未设置 */
  vision: boolean | null
  /** 是否支持工具调用；null = 配置未设置 */
  tools: boolean | null
  /** 上下文窗口；null = 配置未设置 */
  contextWindow: number | null
  /** 单次输出上限；null = 配置未设置 */
  maxOutput: number | null
  /* ---- 以下为 OpenCode v2 独有字段（v1/dsh 场景恒为 null） ---- */
  /** v2 推理强度档位（capabilities.reasoning.effort）；null = 未设置 */
  effort: string[] | null
  /** v2 思考预算下限（capabilities.reasoning.budget.min）；null = 未设置 */
  budgetMin: number | null
  /** v2 思考预算上限（capabilities.reasoning.budget.max）；null = 未设置 */
  budgetMax: number | null
  /** v2 是否支持温度参数（capabilities.temperature）；null = 未设置 */
  temperature: boolean | null
  /** v2 输入侧上限（limit.input）；null = 未设置 */
  inputLimit: number | null
}

/** 未探测到上下文上限时的占位（仅用于探测数据的可编辑初值，不写回配置） */
export const MODEL_EDIT_FALLBACK_CONTEXT = 8192

/** 只接受正的有限数字；其余（含 undefined / null / 0）一律视为「未设置」 */
function num(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) && value > 0 ? value : null
}

function bool(value: unknown): boolean | null {
  return typeof value === 'boolean' ? value : null
}

/** v2 的 input 列表里是否含 image */
function v2InputHasImage(input: unknown): boolean | null {
  if (!Array.isArray(input)) return null
  return input.includes('image')
}

/** v2 capabilities.reasoning.effort → string[] | null */
function v2Effort(reasoning: unknown): string[] | null {
  if (typeof reasoning !== 'object' || reasoning === null) return null
  const effort = (reasoning as Record<string, unknown>).effort
  return Array.isArray(effort) && effort.every((v) => typeof v === 'string') ? (effort as string[]) : null
}

/** v2 capabilities.reasoning.budget 的单个数值字段 */
function v2BudgetValue(reasoning: unknown, key: 'min' | 'max'): number | null {
  if (typeof reasoning !== 'object' || reasoning === null) return null
  const budget = (reasoning as Record<string, unknown>).budget
  if (typeof budget !== 'object' || budget === null) return null
  return num((budget as Record<string, unknown>)[key])
}

// ---- 从配置对象提取 ----

/**
 * 从 OpenCode provider 原始对象提取模型编辑行。
 *
 * @param schema 文件世代：v1（tool_call/attachment/reasoning 布尔）或
 *               v2（capabilities.{tools,input,reasoning}；reasoning 是对象）
 */
export function extractOpenCodeRows(
  provider: Record<string, unknown>,
  schema: 'v1' | 'v2',
): ModelEditRow[] {
  const models = (provider.models ?? {}) as Record<string, Record<string, unknown>>
  return Object.entries(models).map(([key, entry]) => {
    const limit = (entry.limit ?? {}) as Record<string, unknown>
    if (schema === 'v2') {
      const caps = (entry.capabilities ?? {}) as Record<string, unknown>
      const reasoning = caps.reasoning
      return {
        id: key,
        name: typeof entry.name === 'string' ? entry.name : '',
        reasoning:
          typeof reasoning === 'object' && reasoning !== null
            ? bool((reasoning as Record<string, unknown>).supported)
            : null,
        vision: v2InputHasImage(caps.input),
        tools: bool(caps.tools),
        contextWindow: num(limit.context),
        maxOutput: num(limit.output),
        effort: v2Effort(reasoning),
        budgetMin: v2BudgetValue(reasoning, 'min'),
        budgetMax: v2BudgetValue(reasoning, 'max'),
        temperature: bool(caps.temperature),
        inputLimit: num(limit.input),
      }
    }
    return {
      id: key,
      name: typeof entry.name === 'string' ? entry.name : '',
      reasoning: bool(entry.reasoning),
      vision: bool(entry.attachment),
      tools: bool(entry.tool_call),
      contextWindow: num(limit.context),
      maxOutput: num(limit.output),
      effort: null,
      budgetMin: null,
      budgetMax: null,
      temperature: null,
      inputLimit: null,
    }
  })
}

/** 从 dsh provider 原始对象提取模型编辑行 */
export function extractDshRows(provider: Record<string, unknown>): ModelEditRow[] {
  const models = Array.isArray(provider.models) ? (provider.models as Array<Record<string, unknown>>) : []
  return models.map((entry) => ({
    id: typeof entry.id === 'string' ? entry.id : '',
    name: typeof entry.name === 'string' ? entry.name : '',
    reasoning:
      entry.reasoningEfforts === undefined ? null : entry.reasoningEfforts !== false,
    vision: v2InputHasImage(entry.input),
    tools: null, // dsh 的模型条目没有工具开关字段
    contextWindow: num(entry.contextWindow),
    maxOutput: num(entry.maxTokens),
    effort: null,
    budgetMin: null,
    budgetMax: null,
    temperature: null,
    inputLimit: null,
  }))
}

/**
 * 从探测结果（ModelInfo）提取编辑行（一键填写用）。
 *
 * 上下文上限缺失时给回退初值；**输出上限默认留空**（null）。
 *
 * 为什么输出上限不预填：探测到的 `maxOutputLimit` 只是模型「自报」的能力，
 * 不代表网关允许一次请求就用这么多。部分网关（Intern AI 的 tokenplan）
 * 按请求里的 `max_tokens` **预扣**配额，把自报的 1M 原样写进配置后，
 * 每一次请求都会直接 429 quota exceeded —— 而控制台余额看起来还很充足。
 * 因此默认不替用户决定，需要大输出时在参数表里手动填。
 */
export function rowsFromProbeModels(models: ModelInfo[]): ModelEditRow[] {
  return models.map((m) => ({
    id: m.id,
    name: m.displayName && m.displayName !== m.id ? m.displayName : '',
    reasoning: m.isReasoning,
    vision: m.supportsVision === 'supported',
    tools: m.supportsTools === 'supported',
    contextWindow: m.contextLimit > 0 ? m.contextLimit : MODEL_EDIT_FALLBACK_CONTEXT,
    maxOutput: null,
    effort: null,
    budgetMin: null,
    budgetMax: null,
    temperature: null,
    inputLimit: null,
  }))
}

// ---- 写回配置对象 ----

/** 深拷贝一个普通对象（配置条目都是 JSON 形态） */
function clone<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T
}

/**
 * 把编辑结果按 OpenCode 世代写回 provider（返回新对象，不改入参）。
 *
 * 只写「与 original 不同的字段」；`limit` 的 context / output 各自可选，
 * 不强制成对，也不在缺失时补回退值（早期按「必须成对」实现，已纠正）。
 */
export function applyOpenCodeRows(
  provider: Record<string, unknown>,
  rows: ModelEditRow[],
  originals: ModelEditRow[],
  schema: 'v1' | 'v2',
): Record<string, unknown> {
  const next = clone(provider)
  const models = (next.models ?? {}) as Record<string, Record<string, unknown>>

  rows.forEach((row, index) => {
    const before = originals[index] ?? row
    const entry = (models[row.id] ?? {}) as Record<string, unknown>

    if (schema === 'v2') {
      const caps = { ...((entry.capabilities ?? {}) as Record<string, unknown>) }
      if (row.tools !== before.tools && row.tools !== null) {
        caps.tools = row.tools
      }
      if (row.vision !== before.vision && row.vision !== null) {
        caps.input = row.vision ? ['text', 'image'] : ['text']
      }
      if (row.temperature !== before.temperature && row.temperature !== null) {
        caps.temperature = row.temperature
      }

      // reasoning：v2 是对象（supported 必填）；能力开关/强度/预算
      // 任一动过都写，未动过的子字段原样保留
      const reasoningTouched =
        row.reasoning !== before.reasoning ||
        !arraysEqual(row.effort, before.effort) ||
        row.budgetMin !== before.budgetMin ||
        row.budgetMax !== before.budgetMax
      if (reasoningTouched) {
        const reasoning = {
          ...((caps.reasoning ?? {}) as Record<string, unknown>),
        }
        if (row.reasoning !== before.reasoning && row.reasoning !== null) {
          reasoning.supported = row.reasoning
        }
        if (!arraysEqual(row.effort, before.effort)) {
          if (row.effort && row.effort.length > 0) {
            reasoning.effort = [...row.effort]
            reasoning.toggle = true
          } else {
            delete reasoning.effort
            delete reasoning.toggle
          }
        }
        if (row.budgetMin !== before.budgetMin || row.budgetMax !== before.budgetMax) {
          const budget = { ...((reasoning.budget ?? {}) as Record<string, unknown>) }
          if (row.budgetMin !== before.budgetMin) {
            if (row.budgetMin === null) delete budget.min
            else budget.min = row.budgetMin
          }
          if (row.budgetMax !== before.budgetMax) {
            if (row.budgetMax === null) delete budget.max
            else budget.max = row.budgetMax
          }
          if (Object.keys(budget).length > 0) reasoning.budget = budget
          else delete reasoning.budget
        }
        // supported 缺省时补 true（schema 必填；能走到这里说明用户在配置推理）
        if (reasoning.supported === undefined) reasoning.supported = true
        if (reasoning.supported === false && Object.keys(reasoning).length > 1) {
          // supported=false 时其余子字段无意义，一并去掉
          caps.reasoning = { supported: false }
        } else {
          caps.reasoning = reasoning
        }
      }
      entry.capabilities = caps

      // limit：v2 三字段均可选，只写动过的
      const limitTouched =
        row.contextWindow !== before.contextWindow ||
        row.maxOutput !== before.maxOutput ||
        row.inputLimit !== before.inputLimit
      if (limitTouched) {
        const limit = { ...((entry.limit ?? {}) as Record<string, unknown>) }
        if (row.contextWindow !== before.contextWindow && row.contextWindow !== null) {
          limit.context = row.contextWindow
        }
        if (row.maxOutput !== before.maxOutput && row.maxOutput !== null) {
          limit.output = row.maxOutput
        }
        if (row.inputLimit !== before.inputLimit) {
          if (row.inputLimit === null) delete limit.input
          else limit.input = row.inputLimit
        }
        // context / output 各自可选（实测 schema 不要求成对），不再补回退值
        if (Object.keys(limit).length > 0) entry.limit = limit
      }
    } else {
      if (row.tools !== before.tools && row.tools !== null) entry.tool_call = row.tools
      if (row.vision !== before.vision && row.vision !== null) entry.attachment = row.vision
      if (row.reasoning !== before.reasoning && row.reasoning !== null) entry.reasoning = row.reasoning
      const limitTouched =
        row.contextWindow !== before.contextWindow || row.maxOutput !== before.maxOutput
      if (limitTouched) {
        const limit = { ...((entry.limit ?? {}) as Record<string, unknown>) }
        if (row.contextWindow !== before.contextWindow && row.contextWindow !== null) {
          limit.context = row.contextWindow
        }
        if (row.maxOutput !== before.maxOutput && row.maxOutput !== null) {
          limit.output = row.maxOutput
        }
        // 同上：两个字段各自可选，不补回退值
        if (Object.keys(limit).length > 0) entry.limit = limit
      }
    }

    models[row.id] = entry
  })

  next.models = models
  return next
}

/** 两个 string[] | null 是否相等（浅比较元素） */
function arraysEqual(a: string[] | null, b: string[] | null): boolean {
  if (a === b) return true
  if (a === null || b === null) return false
  return a.length === b.length && a.every((v, i) => v === b[i])
}

/**
 * 把编辑行投影成 v2 附加参数表（opencode.ts 的 OpenCodeV2ExtrasMap）。
 *
 * 只收 v2 独有字段；null/空一律不写。供生成器与一键填写在
 * 「从探测结果新建模型条目」的路径上叠加这些参数。
 */
export function v2ExtrasFromRows(rows: ModelEditRow[]): OpenCodeV2ExtrasMap {
  const out: OpenCodeV2ExtrasMap = {}
  for (const row of rows) {
    const extras: Record<string, unknown> = {}
    if (row.effort && row.effort.length > 0) extras.effort = [...row.effort]
    if (row.budgetMin !== null) extras.budgetMin = row.budgetMin
    if (row.budgetMax !== null) extras.budgetMax = row.budgetMax
    if (row.temperature !== null) extras.temperature = row.temperature
    if (row.inputLimit !== null) extras.inputLimit = row.inputLimit
    if (Object.keys(extras).length > 0) out[row.id] = extras
  }
  return out
}

/**
 * 把编辑结果写回 dsh provider（返回新对象）。
 * 同样只写动过的字段；关闭能力 = 移除对应字段（回到 dsh 的继承语义）。
 */
export function applyDshRows(
  provider: Record<string, unknown>,
  rows: ModelEditRow[],
  originals: ModelEditRow[],
): Record<string, unknown> {
  const next = clone(provider)
  const models = Array.isArray(next.models)
    ? (next.models as Array<Record<string, unknown>>)
    : []

  rows.forEach((row, index) => {
    const before = originals[index] ?? row
    const entry = models.find((m) => m.id === row.id)
    if (!entry) return

    if (row.vision !== before.vision && row.vision !== null) {
      if (row.vision) entry.input = ['text', 'image']
      else delete entry.input
    }
    if (row.reasoning !== before.reasoning && row.reasoning !== null) {
      if (row.reasoning) entry.reasoningEfforts = { off: null, high: 'high' }
      else delete entry.reasoningEfforts
    }
    if (row.contextWindow !== before.contextWindow && row.contextWindow !== null) {
      entry.contextWindow = row.contextWindow
    }
    if (row.maxOutput !== before.maxOutput && row.maxOutput !== null) {
      entry.maxTokens = row.maxOutput
    }
  })

  next.models = models
  return next
}

// ---- 写回探测上下文（一键填写用） ----

/**
 * 把编辑行覆盖回 ModelInfo 列表（返回新数组，不改入参）。
 * 一键填写的合并/生成都吃 GenerateContext.models，覆盖后无需改动生成器。
 *
 * `maxOutput` 为 null 表示「不填」——映射成 0（未指定），而不是回退到
 * 探测值，生成器据此省略 `limit.output`。这是「输出上限默认留空」的
 * 落地位置（原因见 rowsFromProbeModels）。
 */
export function applyRowsToProbeModels(models: ModelInfo[], rows: ModelEditRow[]): ModelInfo[] {
  return models.map((model) => {
    const row = rows.find((r) => r.id === model.id)
    if (!row) return model
    return {
      ...model,
      isReasoning: row.reasoning ?? model.isReasoning,
      supportsVision: row.vision === null ? model.supportsVision : row.vision ? 'supported' : 'unsupported',
      supportsTools: row.tools === null ? model.supportsTools : row.tools ? 'supported' : 'unsupported',
      contextLimit: row.contextWindow ?? model.contextLimit,
      maxOutputLimit: row.maxOutput ?? 0,
    }
  })
}
