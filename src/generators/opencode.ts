/**
 * OpenCode 配置生成器。
 *
 * 格式依据：OpenCode 官方 JSON Schema（https://opencode.ai/config.json）。
 * 关键结构（已按 schema 逐字段核对）：
 *
 *   provider.<key> = {
 *     npm?:     string              // 加载哪个 AI SDK provider 包
 *     name?:    string              // 显示名
 *     options?: { apiKey?, baseURL?, ... }
 *     models?:  { [模型 ID]: ModelEntry }
 *   }
 *
 *   ModelEntry = {
 *     id?:          string
 *     name?:        string
 *     tool_call?:   boolean         // 顶层字段，不是 capabilities.toolcall
 *     attachment?:  boolean         // 顶层字段
 *     reasoning?:   boolean         // 顶层字段
 *     limit?:       { context: number, output: number }   // 二者都必填
 *   }
 *
 * 两个 schema 约束（本文件据此实现）：
 * 1. 模型条目 `additionalProperties: false`，嵌套的
 *    `capabilities: { toolcall, ... }` 是非法键，会使整份配置校验失败；
 *    能力开关必须以 `tool_call` / `attachment` / `reasoning` 平铺在条目上。
 * 2. `limit` 对象同样 `additionalProperties: false`，且 `context` / `output`
 *    均为必填，不能只写其中一个。
 *
 * 安全说明：`options.apiKey` 支持明文（默认）或 `{env:VAR}` 引用
 * （密钥留在环境变量里），由 `GenerateContext.apiKeyMode` 控制。
 */

import type { ModelInfo } from '@/types/contracts'
import {
  type ApiKeyMode,
  inferEnvVarName,
  inferProviderKey,
  inferProviderName,
  resolveApiKeyField,
} from './naming'

// ---- 类型定义 ----

/**
 * OpenCode 配置文件中的单个 provider 条目。
 * 字段与官方 schema 的 `ProviderConfig` 对齐。
 */
export interface OpenCodeProvider {
  /** npm 包名：OpenCode 通过它加载对应的 provider SDK */
  npm: string
  /** provider 的显示名称 */
  name: string
  /** provider 级配置 */
  options: {
    /** API Key。明文或 `{env:VAR}` 引用 */
    apiKey?: string
    /** API 根地址 */
    baseURL: string
  }
  /** 模型清单：key 是模型 ID，value 是该模型的配置 */
  models: Record<string, OpenCodeModelEntry>
}

/**
 * OpenCode 中的单个模型条目。
 *
 * 能力开关是顶层字段（`tool_call` / `attachment` / `reasoning`），
 * 不是嵌套对象，原因见文件头注释。
 */
export interface OpenCodeModelEntry {
  /** 模型 ID（与它在 `models` 里的 key 相同，显式写出来便于阅读） */
  id: string
  /** 显示名称 */
  name: string
  /** 是否支持工具调用（Function Calling） */
  tool_call?: boolean
  /** 是否支持附件 / 图片上传 */
  attachment?: boolean
  /** 是否为推理模型 */
  reasoning?: boolean
  /** 上下文与输出上限（opencode 用于 token 预算计算；二者必须同时存在） */
  limit?: {
    context: number
    output: number
  }
}

/** 完整的 OpenCode 配置对象 */
export interface OpenCodeConfig {
  /** 配置结构版本 */
  $schema?: string
  /** provider 字典 */
  provider: Record<string, OpenCodeProvider>
}

// ---- 生成参数 ----

export interface GenerateContext {
  /** Base URL（用户填写的原始值） */
  baseUrl: string
  /** 选中的模型列表 */
  models: ModelInfo[]
  /** provider 在配置中的键名；缺省时由 Base URL 推断 */
  providerKey?: string
  /** provider 的显示名称；缺省时由键名推断 */
  providerName?: string
  /** API Key 明文；`apiKeyMode === 'env'` 时不使用 */
  apiKey?: string
  /** 密钥写入方式，默认 `plain`（明文，复制即可用） */
  apiKeyMode?: ApiKeyMode
  /** `env` 模式下使用的环境变量名；缺省时由键名推断 */
  apiKeyEnv?: string
}

/** 解析后的生成参数（把可选字段全部填成确定值） */
export interface ResolvedContext {
  baseUrl: string
  models: ModelInfo[]
  providerKey: string
  providerName: string
  apiKeyField: string
  apiKeyMode: ApiKeyMode
  envVarName: string
  pkg: string
}

/**
 * 把 `GenerateContext` 补齐成确定值。
 *
 * 单独抽出是为了让「对象构建」与「JSONC 文本渲染」两条路径
 * 共用同一套默认值推断，避免两处各写一遍默认值逻辑导致不一致。
 */
export function resolveContext(context: GenerateContext): ResolvedContext {
  const providerKey = (context.providerKey ?? '').trim() || inferProviderKey(context.baseUrl)
  const providerName =
    (context.providerName ?? '').trim() || inferProviderName(providerKey)
  const apiKeyMode: ApiKeyMode = context.apiKeyMode ?? 'plain'
  const envVarName = (context.apiKeyEnv ?? '').trim() || inferEnvVarName(providerKey)

  return {
    baseUrl: context.baseUrl,
    models: context.models,
    providerKey,
    providerName,
    apiKeyField: resolveApiKeyField(apiKeyMode, context.apiKey ?? '', envVarName),
    apiKeyMode,
    envVarName,
    pkg: inferProviderPackage(context.baseUrl),
  }
}

// ---- 辅助函数 ----

/**
 * 从 Base URL 推测 provider 所使用的 SDK 包名。
 *
 * OpenCode 通过 `npm` 字段加载对应的 AI SDK provider。绝大多数
 * OpenAI 兼容端点都可以用 `@ai-sdk/openai-compatible`，
 * 因此这里统一使用它作为默认值 —— 它正是为「非 OpenAI 官方的
 * OpenAI 兼容端点」设计的。
 *
 * 例外：官方端点使用对应的官方包可以获得更好的兼容性。
 */
export function inferProviderPackage(baseUrl: string): string {
  const lower = baseUrl.toLowerCase()

  // Anthropic 官方端点
  if (lower.includes('api.anthropic.com')) {
    return '@ai-sdk/anthropic'
  }
  // Google Gemini 官方端点
  if (lower.includes('generativelanguage.googleapis.com')) {
    return '@ai-sdk/google'
  }
  // 其余（含 OpenAI 官方）统一走兼容包，它在 OpenAI 端点上同样工作正常
  return '@ai-sdk/openai-compatible'
}

/**
 * 生成单个模型条目。
 *
 * 能力三态为 `unknown` 时**不写入**对应字段，让 OpenCode 用它自己的默认判定，
 * 而不是被本函数错误的 `false` 误导。
 */
export function buildModelEntry(model: ModelInfo): OpenCodeModelEntry {
  const entry: OpenCodeModelEntry = {
    id: model.id,
    name: model.displayName && model.displayName !== model.id ? model.displayName : model.id,
  }

  // 三态 → 顶层布尔字段（unknown 直接省略）
  if (model.supportsTools === 'supported') {
    entry.tool_call = true
  } else if (model.supportsTools === 'unsupported') {
    entry.tool_call = false
  }

  if (model.supportsVision === 'supported') {
    entry.attachment = true
  } else if (model.supportsVision === 'unsupported') {
    entry.attachment = false
  }

  if (model.isReasoning) {
    entry.reasoning = true
  }

  // 只有在上限已知时才写入 limit，避免写入凭空的数字。
  // schema 要求 context 与 output 同时存在，因此要么都写、要么都不写。
  if (model.contextLimit > 0 || model.maxOutputLimit > 0) {
    entry.limit = {
      context: model.contextLimit > 0 ? model.contextLimit : DEFAULT_CONTEXT,
      output: model.maxOutputLimit > 0 ? model.maxOutputLimit : DEFAULT_OUTPUT,
    }
  }

  return entry
}

/** 未能探测到上下文上限时使用的回退值（8K，几乎所有模型都支持） */
const DEFAULT_CONTEXT = 8192
/** 未能探测到输出上限时使用的回退值（4K，几乎无风险） */
const DEFAULT_OUTPUT = 4096

// ---- 主生成函数 ----

/**
 * 构造 OpenCode 配置对象。
 *
 * 返回对象而非字符串：可用官方 schema 做结构化校验（见 opencode.spec.ts）；
 * 后续若需写文件，直接 JSON.stringify 即可。
 */
export function buildOpenCodeConfig(context: GenerateContext): OpenCodeConfig {
  const ctx = resolveContext(context)

  const models: Record<string, OpenCodeModelEntry> = {}
  for (const model of ctx.models) {
    models[model.id] = buildModelEntry(model)
  }

  const options: OpenCodeProvider['options'] = { baseURL: ctx.baseUrl }
  // 密钥为空时不写该字段：写空字符串会让 opencode 报鉴权失败，
  // 不写则允许它回退到环境变量等其它鉴权来源。
  if (ctx.apiKeyField) {
    options.apiKey = ctx.apiKeyField
  }

  const provider: Record<string, OpenCodeProvider> = {
    [ctx.providerKey]: {
      npm: ctx.pkg,
      name: ctx.providerName,
      options,
      models,
    },
  }

  return {
    $schema: 'https://opencode.ai/config.json',
    provider,
  }
}

/**
 * 渲染 provider 对象的内部键值（不含外层的 `"provider": {` / `}`）。
 *
 * 单独抽出：完整配置与片段两种形态只有缩进基准和首尾包装不同，
 * 中间部分完全一致，共享避免两处实现漂移。
 *
 * @param keyIndent provider 键名那一行左侧的空格数
 */
function providerBodyLines(ctx: ResolvedContext, keyIndent: number): string[] {
  const k = keyIndent
  const pad = (n: number) => ' '.repeat(n)
  const lines: string[] = []

  lines.push(`${pad(k)}"${escapeJsonString(ctx.providerKey)}": {`)
  lines.push(`${pad(k + 2)}"npm": "${escapeJsonString(ctx.pkg)}",`)
  lines.push(`${pad(k + 2)}"name": "${escapeJsonString(ctx.providerName)}",`)
  lines.push(`${pad(k + 2)}"options": {`)
  if (ctx.apiKeyField) {
    lines.push(`${pad(k + 4)}"apiKey": "${escapeJsonString(ctx.apiKeyField)}",`)
  }
  lines.push(`${pad(k + 4)}"baseURL": "${escapeJsonString(ctx.baseUrl)}"`)
  lines.push(`${pad(k + 2)}},`)
  lines.push('')
  lines.push(`${pad(k + 2)}"models": {`)

  ctx.models.forEach((model, index) => {
    const entry = buildModelEntry(model)
    const isLast = index === ctx.models.length - 1
    // 注意：键名同样必须转义。模型 ID 来自上游 `/models` 响应，
    // 里面出现 `"` 或 `\` 时若不转义，整份配置就成了非法 JSON。
    lines.push(
      `${pad(k + 4)}"${escapeJsonString(model.id)}": ${stringifyEntry(entry, k + 4)}${isLast ? '' : ','}`,
    )
  })

  lines.push(`${pad(k + 2)}}`)
  lines.push(`${pad(k)}}`)

  return lines
}

/**
 * 把配置渲染为带注释的 JSONC 文本（完整配置）。
 *
 * 手动拼字符串而非 JSON.stringify 的原因：用户需要读懂生成的配置。
 * JSON.stringify 只能输出压缩产物或难以阅读的巨块；手动拼接可以
 * 在文件头写明用途、密钥位置与注意事项，并按 provider → 模型分组加空行。
 *
 * 模型区不加逐条注释：模型一多，注释量会超过配置本身；
 * 探测依据在界面的结果表格里查看更清楚，配置文件保持干净。
 *
 * 输出 JSONC 而非严格 JSON：官方示例普遍使用带注释的 JSONC；
 * 需要严格 JSON 时使用 `renderOpenCodeJson()`。
 */
export function renderOpenCodeJsonc(context: GenerateContext): string {
  const ctx = resolveContext(context)
  const now = new Date().toISOString()

  const lines: string[] = []

  lines.push('{')
  lines.push('  // ===================================================================')
  lines.push('  // OpenCode 配置文件')
  lines.push(`  // 由 ModelScout 于 ${now} 自动生成`)
  lines.push('  //')
  if (ctx.apiKeyMode === 'env') {
    lines.push('  // 密钥以 {env:VAR} 形式引用，使用前请先设置环境变量：')
    lines.push(`  //   ${ctx.envVarName}`)
  } else if (ctx.apiKeyField) {
    lines.push('  // ⚠️ 本文件包含明文 API Key，请勿提交到公开仓库或分享给他人。')
  } else {
    lines.push('  // ⚠️ 未填写 API Key，opencode 将回退到环境变量或其它鉴权来源。')
  }
  lines.push('  // ===================================================================')
  lines.push('  "$schema": "https://opencode.ai/config.json",')
  lines.push('')
  lines.push('  "provider": {')
  lines.push(...providerBodyLines(ctx, 4))
  lines.push('  }')
  lines.push('}')

  return lines.join('\n')
}

/**
 * 渲染为可粘贴的片段：只有 `"provider": { ... }` 一段，没有根大括号。
 *
 * 用户通常已有一份 opencode.json（含 mcp / permission / plugin 等），
 * 需要的是把 provider 合并进去而不是替换整份文件；完整配置自带
 * `{` … `}` 与 `$schema`，直接粘贴会多出一层括号导致解析失败。
 * 片段形态消除这类错误：粘到哪一层就是哪一层。
 *
 * 末尾逗号的处理：片段不带尾逗号——若它恰为最后一项，尾逗号是语法错误；
 * 是否补逗号取决于粘贴位置，因此把提示放在片段末尾，
 * 即用户实际需要判断的位置。
 */
export function renderOpenCodeProviderFragment(context: GenerateContext): string {
  const ctx = resolveContext(context)

  const lines: string[] = []

  lines.push('// ===================================================================')
  lines.push('// OpenCode 配置片段（provider 部分）')
  lines.push('// 由 ModelScout 自动生成 —— 粘贴到你已有 opencode.json 的顶层对象里')
  lines.push('//')
  if (ctx.apiKeyMode === 'env') {
    lines.push(`// 密钥以 {env:VAR} 引用，使用前请先设置环境变量 ${ctx.envVarName}`)
  } else if (ctx.apiKeyField) {
    lines.push('// ⚠️ 本片段包含明文 API Key，请勿分享或提交到公开仓库')
  } else {
    lines.push('// ⚠️ 未填写 API Key，opencode 将回退到环境变量或其它鉴权来源')
  }
  lines.push('// ===================================================================')
  lines.push('')
  lines.push('"provider": {')
  lines.push(...providerBodyLines(ctx, 2))
  lines.push('}')
  lines.push('')
  lines.push('// ↑ 如果这段不是配置文件里的最后一项，请在上一行的 } 后面补一个逗号。')

  return lines.join('\n')
}

/**
 * 把配置渲染为严格 JSON（无注释）。
 *
 * 用于需要 JSON.parse 的场景（CI、脚本、工具链校验）。
 */
export function renderOpenCodeJson(context: GenerateContext): string {
  return JSON.stringify(buildOpenCodeConfig(context), null, 2)
}

// ---- 内部工具 ----

/**
 * 转义 JSON 字符串中的特殊字符。
 *
 * 必须处理：反斜杠、双引号、控制字符。
 * 不处理这些会导致生成的配置文件无法被解析 —— 这是配置生成器最常见的 bug。
 */
export function escapeJsonString(input: string): string {
  return input
    .replace(/\\/g, '\\\\')
    .replace(/"/g, '\\"')
    .replace(/\n/g, '\\n')
    .replace(/\r/g, '\\r')
    .replace(/\t/g, '\\t')
    // 其他控制字符统一转义为 \uXXXX
    .replace(/[\u0000-\u001f]/g, (ch) => {
      const code = ch.charCodeAt(0).toString(16).padStart(4, '0')
      return `\\u${code}`
    })
}

/**
 * 把对象序列化成带缩进的 JSON 片段，缩进基准可指定。
 *
 * 直接 JSON.stringify 的第二参数固定为 2 空格缩进，
 * 无法让它以「已经缩进到某个层级」的状态输出。
 * 因此这里 stringify 后再对每行做额外缩进。
 */
function stringifyEntry(value: unknown, indentSpaces: number): string {
  const raw = JSON.stringify(value, null, 2)
  const pad = ' '.repeat(indentSpaces)
  return raw
    .split('\n')
    .map((line, index) => (index === 0 ? line : pad + line))
    .join('\n')
}

// ---- 自检工具 ----

/**
 * 校验生成结果中不存在未替换的占位符。
 *
 * 验收要求：模板中的占位符全部被正确替换，无 `{{}}` 残留；
 * 该函数把该要求落实为可执行检查。
 */
export function validateNoPlaceholders(output: string): string[] {
  const problems: string[] = []
  // 检测 {{...}} 形式
  const mustache = output.match(/\{\{[^}]*\}\}/g)
  if (mustache) {
    problems.push(`存在未替换的 {{}} 占位符：${mustache.slice(0, 5).join(', ')}`)
  }
  // 检测常见的其他占位符形式
  const otherTokens = [
    /<PLACEHOLDER[^>]*>/gi,
    /\$\{[A-Z_]+\}/g, // 仅检测大写下划线的模板变量，避免误伤正常内容
    /\bTODO\b/g,
    /待填写/,
    /此处省略/,
  ]
  for (const re of otherTokens) {
    const found = output.match(re)
    if (found) {
      problems.push(`存在疑似占位符：${found.slice(0, 5).join(', ')}`)
    }
  }
  return problems
}
