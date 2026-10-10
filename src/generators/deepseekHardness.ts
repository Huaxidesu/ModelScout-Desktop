/**
 * ============================================================================
 * ModelScout —— DeepSeekHardness 配置生成器
 * ============================================================================
 *
 * 注意：重要免责说明
 *
 * DeepSeekHardness 并非本项目开发者维护的软件，其配置文件的确切格式
 * **基于模型训练数据中的公开信息整理**，可能存在以下偏差：
 *   - 字段名拼写（例如 `maxTokens` vs `max_tokens`）
 *   - 配置文件的扩展名（.json / .jsonc / .toml / .yaml）
 *   - 环境变量的注入方式
 *
 * 【交付要求】
 * 使用前请对照 DeepSeekHardness 仓库的最新 README / 示例配置核对。
 * 交付报告已将此列为「待人工确认」第二项。
 *
 * 【本文件的可维护性设计】
 * 全部格式假设集中在 `FORMAT_ASSUMPTIONS` 常量与三个渲染函数中。
 * 若官方格式与假设不符，只需修改这两处。
 *
 * 同时为了让用户能在不确定时快速切换，本生成器**同时输出三种形态**：
 *   1. JSONC（带注释，最可能正确的形态）
 *   2. 严格 JSON
 *   3. TOML（部分此类工具使用 TOML）
 * 用户可以在预览区切换，选择最匹配官方文档的那一个。
 *
 * 【密钥写入方式】
 * 与 OpenCode 生成器保持一致，由 `apiKeyMode` 控制：
 *   - `plain`（默认）：直接写 `apiKey` 明文，复制即可用
 *   - `env`          ：写 `apiKeyEnv` 指向环境变量名，密钥不落文件
 * ============================================================================
 */

import type { ModelInfo } from '@/types/contracts'
import {
  type ApiKeyMode,
  inferEnvVarName,
  inferProviderKey,
  inferProviderName,
} from './naming'

/* ---------------------------------------------------------------------------
 * 格式假设声明（集中在此，便于修正）
 * ------------------------------------------------------------------------- */

/**
 * 本生成器对 DeepSeekHardness 配置格式所做的全部假设。
 *
 * 每一项都标注了「若假设错误，应如何修改」。修改时只需改动
 * 下方对应的渲染函数，不必触碰其他代码。
 */
export const FORMAT_ASSUMPTIONS = {
  /** 假设：顶级配置对象包含 providers 数组 */
  topLevelKey: 'providers',
  /** 假设：每个 provider 用 `baseUrl`（小驼峰） */
  baseUrlKey: 'baseUrl',
  /** 假设：明文密钥字段名 */
  apiKeyKey: 'apiKey',
  /** 假设：每个 provider 用 `apiKeyEnv` 指向环境变量，而非硬编码 Key */
  apiKeyEnvKey: 'apiKeyEnv',
  /** 假设：模型列表字段名 */
  modelsKey: 'models',
  /** 假设：模型的上限字段名 */
  contextKey: 'contextWindow',
  outputKey: 'maxOutputTokens',
  /** 假设：能力开关字段名 */
  toolCallKey: 'supportsToolCall',
  visionKey: 'supportsVision',
  reasoningKey: 'isReasoning',
  /** 假设：默认使用的环境变量名 */
  defaultEnvVar: 'MODELSCOUT_API_KEY',
  /** 假设：默认配置文件扩展名 */
  defaultFileName: 'deepseek-hardness.jsonc',
} as const

/* ---------------------------------------------------------------------------
 * 类型定义
 * ------------------------------------------------------------------------- */

/** DeepSeekHardness 中的单个模型条目 */
export interface HardnessModelEntry {
  /** 模型 ID（在 provider 内唯一） */
  id: string
  /** 显示名称 */
  name: string
  /** 上下文窗口上限 */
  contextWindow: number
  /** 单次输出上限 */
  maxOutputTokens: number
  /** 是否支持工具调用 */
  supportsToolCall: boolean
  /** 是否支持视觉附件 */
  supportsVision: boolean
  /** 是否为推理模型 */
  isReasoning: boolean
}

/** DeepSeekHardness 中的单个 provider 条目 */
export interface HardnessProviderEntry {
  /** provider 标识名 */
  id: string
  /** 显示名称 */
  name: string
  /** API 根地址 */
  baseUrl: string
  /** 明文 API Key；仅 `plain` 模式且密钥非空时出现 */
  apiKey?: string
  /** 指向 API Key 的环境变量名；仅 `env` 模式时出现 */
  apiKeyEnv?: string
  /** 模型列表 */
  models: HardnessModelEntry[]
}

/** 完整的 DeepSeekHardness 配置 */
export interface HardnessConfig {
  /** 配置结构版本（假设） */
  version?: string
  /** provider 列表 */
  providers: HardnessProviderEntry[]
}

/* ---------------------------------------------------------------------------
 * 生成参数
 * ------------------------------------------------------------------------- */

export interface GenerateContext {
  /** Base URL */
  baseUrl: string
  /** 选中的模型列表 */
  models: ModelInfo[]
  /** provider 标识名；缺省时由 Base URL 推断 */
  providerId?: string
  /** provider 显示名；缺省时由标识名推断 */
  providerName?: string
  /** API Key 明文；`apiKeyMode === 'env'` 时不使用 */
  apiKey?: string
  /** 密钥写入方式，默认 `plain` */
  apiKeyMode?: ApiKeyMode
  /** Key 的环境变量名；缺省时由标识名推断 */
  apiKeyEnv?: string
}

/** 解析后的生成参数 */
interface ResolvedContext {
  baseUrl: string
  models: ModelInfo[]
  providerId: string
  providerName: string
  apiKeyMode: ApiKeyMode
  envVarName: string
  apiKeyPlain: string
}

/**
 * 把 `GenerateContext` 补齐成确定值。
 *
 * 与 opencode.ts 的同名函数保持同样的思路：对象构建与文本渲染
 * 共用一套默认值推断，避免「改了一处漏了另一处」。
 */
function resolveContext(context: GenerateContext): ResolvedContext {
  const providerId = (context.providerId ?? '').trim() || inferProviderKey(context.baseUrl)
  const providerName =
    (context.providerName ?? '').trim() || inferProviderName(providerId)
  const apiKeyMode: ApiKeyMode = context.apiKeyMode ?? 'plain'
  const envVarName =
    (context.apiKeyEnv ?? '').trim() ||
    inferEnvVarName(providerId) ||
    FORMAT_ASSUMPTIONS.defaultEnvVar

  return {
    baseUrl: context.baseUrl,
    models: context.models,
    providerId,
    providerName,
    apiKeyMode,
    envVarName,
    apiKeyPlain: (context.apiKey ?? '').trim(),
  }
}

/* ---------------------------------------------------------------------------
 * 主生成函数
 * ------------------------------------------------------------------------- */

/** 未能探测到上下文上限时的回退值（保守默认值） */
const DEFAULT_CONTEXT = 8192
/** 未能探测到输出上限时的回退值（保守默认值） */
const DEFAULT_OUTPUT = 4096

/**
 * 构造 DeepSeekHardness 配置对象。
 *
 * 【安全说明】
 * `plain` 模式会把 API Key 明文写进配置，`env` 模式只写环境变量名。
 * 前者复制即可用，后者适合把配置提交进版本库 —— 由用户按场景选择。
 */
export function buildHardnessConfig(context: GenerateContext): HardnessConfig {
  const ctx = resolveContext(context)

  const models: HardnessModelEntry[] = ctx.models.map((model) => ({
    id: model.id,
    name: model.displayName && model.displayName !== model.id ? model.displayName : model.id,
    contextWindow: model.contextLimit > 0 ? model.contextLimit : DEFAULT_CONTEXT,
    maxOutputTokens: model.maxOutputLimit > 0 ? model.maxOutputLimit : DEFAULT_OUTPUT,
    // 三态中 unknown 一律按 false 处理 —— 在配置文件里，
    // 「不确定但开启」比「不确定而关闭」风险更大（会触发运行时报错）
    supportsToolCall: model.supportsTools === 'supported',
    supportsVision: model.supportsVision === 'supported',
    isReasoning: model.isReasoning,
  }))

  const provider: HardnessProviderEntry = {
    id: ctx.providerId,
    name: ctx.providerName,
    baseUrl: ctx.baseUrl,
    models,
  }

  if (ctx.apiKeyMode === 'env') {
    provider.apiKeyEnv = ctx.envVarName
  } else if (ctx.apiKeyPlain) {
    provider.apiKey = ctx.apiKeyPlain
  }

  return {
    version: '1',
    providers: [provider],
  }
}

/*
 * 说明：这里原本还有 `sourceLabel` / `limitSourceText` 两个函数，
 * 用于把「上限来源」写进每个模型的注释里。随着逐条注释被移除
 * （用户反馈注释太多、淹掉配置本身），它们已无调用方。
 *
 * 上限来源信息本身没有丢 —— 它在界面的结果表格里以「接口声明 /
 * 字典 / 报错解析 / 手动」标签的形式展示，那才是用户真正需要它的地方。
 */

/* ---------------------------------------------------------------------------
 * 三种渲染形态
 * ------------------------------------------------------------------------- */

/**
 * 渲染为带注释的 JSONC 文本（**推荐形态**）。
 *
 * 与 opencode.ts 一致：**不给每个模型写注释** —— 模型一多，注释量就
 * 超过配置本身。探测依据在界面表格里看更清楚。
 */
export function renderHardnessJsonc(context: GenerateContext): string {
  const ctx = resolveContext(context)
  const now = new Date().toISOString()

  const lines: string[] = []

  lines.push('{')
  lines.push('  // ===================================================================')
  lines.push('  // DeepSeekHardness 配置文件')
  lines.push(`  // 由 ModelScout 于 ${now} 自动生成`)
  lines.push('  //')
  lines.push('  // ⚠️ 本配置结构基于公开信息整理，请对照 DeepSeekHardness 官方')
  lines.push('  //    最新文档与示例配置核对字段名后再使用。')
  lines.push('  //')
  if (ctx.apiKeyMode === 'env') {
    lines.push(`  // 密钥以环境变量名引用，使用前请先设置 ${ctx.envVarName}。`)
  } else if (ctx.apiKeyPlain) {
    lines.push('  // ⚠️ 本文件包含明文 API Key，请勿提交到公开仓库或分享给他人。')
  } else {
    lines.push('  // ⚠️ 未填写 API Key，请自行在配置中补上或改用环境变量方式。')
  }
  lines.push('  // ===================================================================')
  lines.push('  "version": "1",')
  lines.push(`  "${FORMAT_ASSUMPTIONS.topLevelKey}": [`)
  lines.push('    {')
  lines.push(`      "id": "${escapeJsonString(ctx.providerId)}",`)
  lines.push(`      "name": "${escapeJsonString(ctx.providerName)}",`)
  lines.push(`      "${FORMAT_ASSUMPTIONS.baseUrlKey}": "${escapeJsonString(ctx.baseUrl)}",`)
  if (ctx.apiKeyMode === 'env') {
    lines.push(
      `      "${FORMAT_ASSUMPTIONS.apiKeyEnvKey}": "${escapeJsonString(ctx.envVarName)}",`,
    )
  } else if (ctx.apiKeyPlain) {
    lines.push(
      `      "${FORMAT_ASSUMPTIONS.apiKeyKey}": "${escapeJsonString(ctx.apiKeyPlain)}",`,
    )
  }
  lines.push(`      "${FORMAT_ASSUMPTIONS.modelsKey}": [`)

  ctx.models.forEach((model, index) => {
    const ctxLimit = model.contextLimit > 0 ? model.contextLimit : DEFAULT_CONTEXT
    const outLimit = model.maxOutputLimit > 0 ? model.maxOutputLimit : DEFAULT_OUTPUT
    const isLast = index === ctx.models.length - 1

    lines.push('        {')
    lines.push(`          "id": "${escapeJsonString(model.id)}",`)
    lines.push(`          "name": "${escapeJsonString(model.displayName || model.id)}",`)
    lines.push(`          "${FORMAT_ASSUMPTIONS.contextKey}": ${ctxLimit},`)
    lines.push(`          "${FORMAT_ASSUMPTIONS.outputKey}": ${outLimit},`)
    lines.push(
      `          "${FORMAT_ASSUMPTIONS.toolCallKey}": ${model.supportsTools === 'supported'},`,
    )
    lines.push(
      `          "${FORMAT_ASSUMPTIONS.visionKey}": ${model.supportsVision === 'supported'},`,
    )
    lines.push(`          "${FORMAT_ASSUMPTIONS.reasoningKey}": ${model.isReasoning}`)
    lines.push(`        }${isLast ? '' : ','}`)
  })

  lines.push('      ]')
  lines.push('    }')
  lines.push('  ]')
  lines.push('}')

  return lines.join('\n')
}

/**
 * 渲染为**严格 JSON**（无注释）。
 */
export function renderHardnessJson(context: GenerateContext): string {
  return JSON.stringify(buildHardnessConfig(context), null, 2)
}

/**
 * 渲染为 TOML 形态。
 *
 * 【为什么也要提供】
 * 部分此类本地推理调度工具使用 TOML。多提供一个形态的成本很低，
 * 却能显著提升「至少有一个能用」的概率。
 *
 * TOML 结构采用 `[[providers]]` + `[[providers.models]]` 的数组表语法。
 */
export function renderHardnessToml(context: GenerateContext): string {
  const ctx = resolveContext(context)
  const now = new Date().toISOString()

  const lines: string[] = []

  lines.push('# =====================================================================')
  lines.push('# DeepSeekHardness 配置文件（TOML 形态）')
  lines.push(`# 由 ModelScout 于 ${now} 自动生成`)
  lines.push('#')
  lines.push('# ⚠️ 本配置结构基于公开信息整理，请对照官方最新文档核对字段名。')
  if (ctx.apiKeyMode === 'env') {
    lines.push(`# 使用前请在环境中设置 ${ctx.envVarName} 环境变量。`)
  } else if (ctx.apiKeyPlain) {
    lines.push('# ⚠️ 本文件包含明文 API Key，请勿提交到公开仓库。')
  }
  lines.push('# =====================================================================')
  lines.push('')
  lines.push('version = "1"')
  lines.push('')
  lines.push('[[providers]]')
  lines.push(`id = ${tomlString(ctx.providerId)}`)
  lines.push(`name = ${tomlString(ctx.providerName)}`)
  lines.push(`${FORMAT_ASSUMPTIONS.baseUrlKey} = ${tomlString(ctx.baseUrl)}`)
  if (ctx.apiKeyMode === 'env') {
    lines.push(`${FORMAT_ASSUMPTIONS.apiKeyEnvKey} = ${tomlString(ctx.envVarName)}`)
  } else if (ctx.apiKeyPlain) {
    lines.push(`${FORMAT_ASSUMPTIONS.apiKeyKey} = ${tomlString(ctx.apiKeyPlain)}`)
  }
  lines.push('')

  ctx.models.forEach((model) => {
    const ctxLimit = model.contextLimit > 0 ? model.contextLimit : DEFAULT_CONTEXT
    const outLimit = model.maxOutputLimit > 0 ? model.maxOutputLimit : DEFAULT_OUTPUT

    lines.push('  [[providers.models]]')
    lines.push(`  id = ${tomlString(model.id)}`)
    lines.push(`  name = ${tomlString(model.displayName || model.id)}`)
    lines.push(`  ${FORMAT_ASSUMPTIONS.contextKey} = ${ctxLimit}`)
    lines.push(`  ${FORMAT_ASSUMPTIONS.outputKey} = ${outLimit}`)
    lines.push(`  ${FORMAT_ASSUMPTIONS.toolCallKey} = ${model.supportsTools === 'supported'}`)
    lines.push(`  ${FORMAT_ASSUMPTIONS.visionKey} = ${model.supportsVision === 'supported'}`)
    lines.push(`  ${FORMAT_ASSUMPTIONS.reasoningKey} = ${model.isReasoning}`)
    lines.push('')
  })

  while (lines.length > 0 && lines[lines.length - 1] === '') {
    lines.pop()
  }

  return lines.join('\n')
}

/* ---------------------------------------------------------------------------
 * 内部工具
 * ------------------------------------------------------------------------- */

/** 转义 JSON 字符串 */
function escapeJsonString(input: string): string {
  return input
    .replace(/\\/g, '\\\\')
    .replace(/"/g, '\\"')
    .replace(/\n/g, '\\n')
    .replace(/\r/g, '\\r')
    .replace(/\t/g, '\\t')
    .replace(/[\u0000-\u001f]/g, (ch) => {
      const code = ch.charCodeAt(0).toString(16).padStart(4, '0')
      return `\\u${code}`
    })
}

/**
 * TOML 字符串字面量。
 *
 * TOML 的字符串转义规则与 JSON 相似但不完全相同：
 *   - 同样用双引号包裹
 *   - 需要转义 `"` 与 `\`
 *   - 反斜杠必须转义（这点与 JSON 一致）
 * 控制字符在 TOML 中的合法转义集比 JSON 小，因此这里直接移除
 * —— 实际配置里不会出现控制字符，这个处理是安全的回退。
 */
function tomlString(input: string): string {
  const escaped = input
    .replace(/\\/g, '\\\\')
    .replace(/"/g, '\\"')
    .replace(/[\u0000-\u001f\u007f]/g, '')
  return `"${escaped}"`
}

/* ---------------------------------------------------------------------------
 * 自检工具（供阶段 5 QA 使用）
 * ------------------------------------------------------------------------- */

/**
 * 校验生成结果中不存在未替换的占位符。
 *
 * 与 opencode.ts 的同名函数保持一致的检查项，
 * 保证「无残留 {{}}」这项检查对两个生成器都生效。
 */
export function validateNoPlaceholders(output: string): string[] {
  const problems: string[] = []

  const mustache = output.match(/\{\{[^}]*\}\}/g)
  if (mustache) {
    problems.push(`存在未替换的 {{}} 占位符：${mustache.slice(0, 5).join(', ')}`)
  }

  const otherTokens: Array<[RegExp, string]> = [
    [/<PLACEHOLDER[^>]*>/gi, 'PLACEHOLDER 标签'],
    [/\$\{[A-Z_]+\}/g, '大写模板变量'],
    [/\bTODO\b/g, 'TODO'],
    [/此处省略/, '省略占位'],
    [/待填写/, '待填写占位'],
    [/同上/, '同上占位'],
  ]

  for (const [re, label] of otherTokens) {
    const found = output.match(re)
    if (found) {
      problems.push(`存在疑似占位符（${label}）：${found.slice(0, 5).join(', ')}`)
    }
  }

  return problems
}
