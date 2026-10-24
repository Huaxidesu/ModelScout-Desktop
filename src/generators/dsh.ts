/**
 * DeepSeek Harness（dsh）配置生成器 —— 按 dsh 的真实格式生成。
 *
 * 格式依据（2026-10-25 核对）：
 *   - 文件：`$DSH_HOME/profiles/<profile>/cordis.patch.yml`（DSH_HOME 默认
 *     `~/.dsh`；`dsh web` 的 profile 为 `web`）。dsh 每次请求前重读该文件。
 *   - 结构：顶层是「插件补丁数组」，自定义 provider 挂在
 *     `- id: llm-pi-ai` 条目的 `config.providers.<routeKey>` 下。
 *   - provider 字段（PiAiProviderProfile，见官方 config-catalog）：
 *       apiKeyEnv   凭据引用（环境变量名）——补丁文件不支持明文 Key
 *       displayName 配置界面的显示名（缺省用 routeKey）
 *       api         openai-completions | openai-responses | anthropic-messages
 *       baseURL     API 根地址
 *       compat      线上兼容开关（见下方 compat 注释）
 *       models      模型列表（数组）
 *   - 模型字段（PiAiModelProfile）：
 *       id / name / contextWindow / maxTokens /
 *       input（[text, image]，缺省继承 [text]）/
 *       reasoningEfforts（声明推理档位；不声明 = 无档位菜单）
 *
 * compat 开关的取舍：pi-ai 对不认识的地址按 OpenAI 官方端点发送请求
 * （developer role + max_completion_tokens），多数 OpenAI 兼容网关会拒绝。
 * 因此非 OpenAI 官方端点默认写入官方文档推荐的
 * `compat: { supportsDeveloperRole: false, maxTokensField: max_tokens }`；
 * OpenAI 官方端点则省略（让 pi-ai 自行识别）。
 *
 * 与 deepseekHardness.ts 的关系：那是最早按公开信息猜测的模板（JSON/TOML），
 * 保留作参考；本文件是按真实 schema 生成的版本，「一键填写」以此为准。
 */

import { load as yamlLoad, dump as yamlDump } from 'js-yaml'

import type { ModelInfo } from '@/types/contracts'
import { type ApiKeyMode, inferEnvVarName, inferProviderKey, inferProviderName } from './naming'

/** dsh 补丁文件里承载自定义 provider 的插件 id */
export const DSH_LLM_PLUGIN_ID = 'llm-pi-ai'

/** 未能探测到上下文上限时的回退值（保守默认，避免 dsh 按 1M 默认值发超限请求） */
const DEFAULT_CONTEXT_WINDOW = 8192
/** 未能探测到输出上限时的回退值 */
const DEFAULT_MAX_TOKENS = 4096

// ---- 类型定义（对应 PiAiModelProfile / PiAiProviderProfile 的子集） ----

/** dsh 里的一条模型配置 */
export interface DshModelProfile {
  id: string
  /** 选择器显示名；缺省用 id */
  name?: string
  /** 上下文窗口（请求+响应合计） */
  contextWindow: number
  /** 单次输出上限 */
  maxTokens: number
  /** 输入模态；缺省继承 [text] */
  input?: Array<'text' | 'image'>
  /** 推理档位；key 是档位名，value 是线上 spelling（off 可为空） */
  reasoningEfforts?: Record<string, string | null>
}

/** dsh 里的一条 provider 路由配置 */
export interface DshProviderProfile {
  /** 凭据引用（环境变量名）。dsh 不接受补丁文件里写明文 Key */
  apiKeyEnv: string
  /** 显示名 */
  displayName: string
  /** 线上协议 */
  api: 'openai-completions' | 'openai-responses' | 'anthropic-messages'
  /** API 根地址 */
  baseURL: string
  /** 线上兼容开关（仅网关端点写入） */
  compat?: {
    supportsDeveloperRole: false
    maxTokensField: 'max_tokens'
  }
  /** 模型列表 */
  models: DshModelProfile[]
}

/** 生成参数（与 opencode 生成器同构，可由预览区直接传入） */
export interface GenerateContext {
  /** Base URL */
  baseUrl: string
  /** 选中的模型列表 */
  models: ModelInfo[]
  /** provider 路由键名（dsh 要求小写）；缺省由 Base URL 推断 */
  providerKey?: string
  /** provider 显示名；缺省由键名推断 */
  providerName?: string
  /** API Key 明文。dsh 补丁不支持明文，此字段仅用于决定提示文案 */
  apiKey?: string
  /** 密钥写入方式（dsh 始终用环境变量引用，此字段用于提示） */
  apiKeyMode?: ApiKeyMode
  /** 环境变量名；缺省由键名推断 */
  apiKeyEnv?: string
}

/** 解析后的生成参数 */
interface ResolvedContext {
  baseUrl: string
  models: ModelInfo[]
  providerKey: string
  providerName: string
  envVarName: string
  hasPlainKey: boolean
}

function resolveContext(context: GenerateContext): ResolvedContext {
  const providerKey = (context.providerKey ?? '').trim() || inferProviderKey(context.baseUrl)
  const providerName =
    (context.providerName ?? '').trim() || inferProviderName(providerKey)
  const envVarName = (context.apiKeyEnv ?? '').trim() || inferEnvVarName(providerKey)
  return {
    baseUrl: context.baseUrl,
    models: context.models,
    providerKey,
    providerName,
    envVarName,
    hasPlainKey: context.apiKeyMode !== 'env' && Boolean((context.apiKey ?? '').trim()),
  }
}

// ---- 对象构建 ----

/** 从 Base URL 推断 dsh 的线上协议 */
export function inferDshApi(baseUrl: string): DshProviderProfile['api'] {
  const lower = baseUrl.toLowerCase()
  if (lower.includes('api.anthropic.com')) return 'anthropic-messages'
  // ModelScout 只对接 OpenAI 兼容端点，其余一律走 chat completions 协议
  return 'openai-completions'
}

/** 是否为 OpenAI 官方端点（pi-ai 能自行识别，无需 compat 开关） */
function isOpenAiOfficial(baseUrl: string): boolean {
  return baseUrl.toLowerCase().includes('api.openai.com')
}

/** 构造单条模型配置 */
export function buildDshModelProfile(model: ModelInfo): DshModelProfile {
  const name =
    model.displayName && model.displayName !== model.id ? model.displayName : undefined
  // 未知上限写保守回退值：省略时 dsh 会按路由默认值（1M/128K）预算，
  // 对探测不到上限的网关模型反而更容易发出超限请求
  const contextWindow = model.contextLimit > 0 ? model.contextLimit : DEFAULT_CONTEXT_WINDOW
  const maxTokens = model.maxOutputLimit > 0 ? model.maxOutputLimit : DEFAULT_MAX_TOKENS
  // 与官方文档示例一致：off（不发参数）+ high。
  // 网关若不认 reasoning_effort，可删掉本段或加 compat 开关。
  const reasoningEfforts = model.isReasoning ? { off: null, high: 'high' } : undefined

  // 键序（id → name → 上限 → 能力）与 renderDshPatchYml 的手写渲染保持一致，
  // 让 js-yaml dump（合并路径）与手写渲染（新建路径）产出的字段顺序相同
  return {
    id: model.id,
    ...(name !== undefined ? { name } : {}),
    contextWindow,
    maxTokens,
    ...(model.supportsVision === 'supported' ? { input: ['text', 'image'] as Array<'text' | 'image'> } : {}),
    ...(reasoningEfforts !== undefined ? { reasoningEfforts } : {}),
  }
}

/** 构造完整的 provider 路由配置对象 */
export function buildDshProviderProfile(context: GenerateContext): DshProviderProfile {
  const ctx = resolveContext(context)
  const api = inferDshApi(ctx.baseUrl)

  const profile: DshProviderProfile = {
    apiKeyEnv: ctx.envVarName,
    displayName: ctx.providerName,
    api,
    baseURL: ctx.baseUrl,
    models: ctx.models.map(buildDshModelProfile),
  }

  if (api === 'openai-completions' && !isOpenAiOfficial(ctx.baseUrl)) {
    profile.compat = { supportsDeveloperRole: false, maxTokensField: 'max_tokens' }
  }
  return profile
}

// ---- 文本渲染（新建文件 / 片段预览用） ----

/**
 * 渲染全新补丁文件（文件不存在时「一键填写」直接写入的内容）。
 *
 * 手写渲染而非 yamlDump：头部要写用途、密钥与路径说明，保持注释可读。
 */
export function renderDshPatchYml(context: GenerateContext): string {
  const ctx = resolveContext(context)
  const now = new Date().toISOString()
  const lines: string[] = []

  lines.push('# =====================================================================')
  lines.push('# DeepSeek Harness (dsh) 配置补丁')
  lines.push(`# 由 ModelScout 于 ${now} 自动生成`)
  lines.push('#')
  lines.push('# 位置：$DSH_HOME/profiles/<profile>/cordis.patch.yml（默认 ~/.dsh/profiles/web/）')
  lines.push('# dsh 每次请求前都会重读本文件，保存即生效，无需重启。')
  lines.push('#')
  lines.push('# 密钥：dsh 不支持在补丁里写明文 Key，这里用 apiKeyEnv 引用环境变量。')
  lines.push(`# 使用前请设置环境变量 ${ctx.envVarName}，`)
  lines.push('# 或在 dsh 的 Settings → Models 页面为该 provider 保存凭据。')
  lines.push('# =====================================================================')
  lines.push('')
  lines.push(...renderPluginEntryLines(context))

  return `${lines.join('\n')}\n`
}

/**
 * 渲染 `- id: llm-pi-ai` 插件条目（含 config.providers 与 provider 主体）。
 */
function renderPluginEntryLines(context: GenerateContext): string[] {
  const ctx = resolveContext(context)
  const lines: string[] = []
  lines.push(`- id: ${DSH_LLM_PLUGIN_ID}`)
  lines.push('  config:')
  lines.push('    providers:')
  lines.push(`      ${yamlScalar(ctx.providerKey)}:`)
  // provider 字段以「字段名顶格」的相对缩进产出，这里统一加 8 空格挂到键下
  for (const line of providerFieldLines(context)) {
    lines.push(line === '' ? '' : `        ${line}`)
  }
  return lines
}

/**
 * provider 字段行（**相对缩进**：字段名顶格，嵌套内容向右 2 空格递进）。
 *
 * 由两个调用方各自决定挂载缩进：完整文件挂在 `      <key>:` 之下（+8），
 * 片段挂在 `  <key>:` 之下（+4）。
 */
function providerFieldLines(context: GenerateContext): string[] {
  const profile = buildDshProviderProfile(context)
  const lines: string[] = []

  lines.push(`apiKeyEnv: ${yamlScalar(profile.apiKeyEnv)}`)
  lines.push(`displayName: ${yamlScalar(profile.displayName)}`)
  lines.push(`api: ${profile.api}`)
  lines.push(`baseURL: ${yamlScalar(profile.baseURL)}`)

  if (profile.compat) {
    lines.push('# 中转/自建网关按下面两个开关发请求更稳（OpenAI 官方端点可删掉 compat 段）')
    lines.push('compat:')
    lines.push('  supportsDeveloperRole: false')
    lines.push('  maxTokensField: max_tokens')
  }

  lines.push('models:')
  for (const model of profile.models) {
    lines.push(`  - id: ${yamlScalar(model.id)}`)
    if (model.name !== undefined) {
      lines.push(`    name: ${yamlScalar(model.name)}`)
    }
    lines.push(`    contextWindow: ${model.contextWindow}`)
    lines.push(`    maxTokens: ${model.maxTokens}`)
    if (model.input) {
      lines.push('    input:')
      lines.push('      - text')
      lines.push('      - image')
    }
    if (model.reasoningEfforts) {
      lines.push('    reasoningEfforts:')
      lines.push('      off:')
      lines.push('      high: high')
    }
  }

  return lines
}

/**
 * 渲染可粘贴的片段：只有 `providers.<key>:` 子树。
 *
 * 用途：粘到已有 cordis.patch.yml 里 `- id: llm-pi-ai` 条目的
 * `config.providers:` 下面（同级会多一个 provider 键）。
 */
export function renderDshProviderFragmentYml(context: GenerateContext): string {
  const ctx = resolveContext(context)
  const lines: string[] = []

  lines.push('# =====================================================================')
  lines.push('# DeepSeek Harness (dsh) provider 片段')
  lines.push('# 由 ModelScout 自动生成 —— 粘贴到已有 cordis.patch.yml 里')
  lines.push('# `- id: llm-pi-ai` 条目的 config.providers: 下面（同级再加一个键）')
  lines.push('#')
  lines.push(`# 密钥通过环境变量 ${ctx.envVarName} 提供（dsh 补丁不支持明文 Key）`)
  lines.push('# =====================================================================')
  lines.push('')
  lines.push(`providers:`)
  lines.push(`  ${yamlScalar(ctx.providerKey)}:`)
  // 字段行的相对缩进以「provider 字段名顶格」为基准，挂到片段里要 +4 空格
  for (const line of providerFieldLines(context)) {
    lines.push(line === '' ? '' : `    ${line}`)
  }

  return `${lines.join('\n')}\n`
}

/**
 * YAML 标量序列化：能安全用 plain 风格就用 plain，否则双引号转义。
 *
 * 保守判断——只放行字母数字与少量安全符号开头、且不含
 * `: `、` #`、前后空白等 YAML 陷阱的值。
 */
export function yamlScalar(value: string): string {
  if (/^[A-Za-z0-9_][A-Za-z0-9_./:@+~-]*$/.test(value)) {
    return value
  }
  // JSON 字符串是合法的 YAML 双引号标量
  return JSON.stringify(value)
}

// ---- 合并（一键填写：已有配置 → 并入 provider） ----

/** 合并结果 */
export interface DshMergeResult {
  /** create = 全新文件；merge = 并入已有补丁 */
  mode: 'create' | 'merge'
  /** 合并后的完整文件文本 */
  text: string
  /** 需要向用户明示的注意事项 */
  warnings: string[]
}

/** 无法合并时抛出；message 面向用户（中文） */
export class DshMergeError extends Error {
  constructor(message: string) {
    super(message)
    this.name = 'DshMergeError'
  }
}

/**
 * 把生成的 provider 并入已有（或空缺的）cordis.patch.yml。
 *
 * dsh 的合并语义（官方文档：「Cordis config override 会整体替换条目
 * 配置，编辑时要保留其他 provider 与字段」）：
 *   - 找到 `- id: llm-pi-ai` 条目 → 在其 config.providers 下替换同名键、
 *     保留其他 provider；条目不存在则追加一个；
 *   - 其他插件条目原样保留。
 *
 * @param existingText 磁盘上的原文件内容；空字符串表示文件不存在
 */
export function mergeDshIntoText(
  existingText: string,
  context: GenerateContext,
  options: { providerComment?: string } = {},
): DshMergeResult {
  if (existingText.trim() === '') {
    const body = renderDshPatchYml(context)
    const text = options.providerComment
      ? insertYamlComment(body, resolveContext(context).providerKey, options.providerComment)
      : body
    return { mode: 'create', text, warnings: [] }
  }

  let doc: unknown
  try {
    doc = yamlLoad(existingText)
  } catch (err) {
    const message = err instanceof Error ? err.message.split('\n')[0] : String(err)
    // js-yaml v5 对「没有文档内容」（只有注释/纯空白）会抛
    // "expected a document, but the input is empty"，单独给出更准确的提示
    if (/input is empty/.test(message)) {
      throw new DshMergeError(
        '现有文件里没有可解析的 YAML 内容（可能只有注释）。请手动把片段粘贴进去，或清空后重试。',
      )
    }
    throw new DshMergeError(`现有文件不是合法的 YAML（${message}），无法自动合并。`)
  }

  if (doc === null || doc === undefined) {
    throw new DshMergeError(
      '现有文件里没有可解析的 YAML 内容（可能只有注释）。请手动把片段粘贴进去，或清空后重试。',
    )
  }
  if (!Array.isArray(doc)) {
    throw new DshMergeError(
      'cordis.patch.yml 的顶层应是插件条目数组（`- id: ...` 列表），当前文件顶层不是数组，无法自动合并。',
    )
  }

  const ctx = resolveContext(context)
  const profile = buildDshProviderProfile(context)
  const result = mergeDshProfileIntoText(existingText, ctx.providerKey, {
    ...profile,
  })
  if (options.providerComment) {
    return {
      ...result,
      text: insertYamlComment(result.text, ctx.providerKey, options.providerComment),
    }
  }
  return result
}

/**
 * 在 YAML 文本中 provider 键所在行上方插入一行 `# 注释`（同缩进）。
 *
 * 键已是标记注释（含 ModelScout）的上一行会被原位替换,避免重复填写堆积;
 * 找不到键行时原样返回(不冒险)。
 */
export function insertYamlComment(text: string, providerKey: string, comment: string): string {
  const escaped = providerKey.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
  const re = new RegExp(`^(\\s+)(${escaped}|"${escaped}"|'${escaped}'):(\\s|$)`, 'm')
  const match = re.exec(text)
  if (!match || match.index === undefined) return text
  const lineStart = match.index
  const indent = match[1]
  const commentLine = `${indent}# ${comment}`

  if (lineStart > 0) {
    const prevEnd = lineStart - 1
    const prevStart = text.lastIndexOf('\n', Math.max(prevEnd - 1, 0)) + 1
    const prevLine = text.slice(prevStart, prevEnd)
    if (/^\s*#.*ModelScout/.test(prevLine)) {
      return text.slice(0, prevStart) + commentLine + text.slice(lineStart)
    }
  }
  return text.slice(0, lineStart) + commentLine + '\n' + text.slice(lineStart)
}

/**
 * 把一个 provider 对象（原始数据，不必是本生成器构造的）并入补丁文件。
 *
 * 配置管理页「绑定密钥 / 改动回写」用：在解析出的现有 provider 上
 * 修改字段后原样写回，保留解析时见到的其余字段。
 */
export function mergeDshProfileIntoText(
  existingText: string,
  providerKey: string,
  profile: Record<string, unknown>,
): DshMergeResult {
  if (existingText.trim() === '') {
    throw new DshMergeError('目标文件为空；请先用「一键填写」生成完整补丁后再做增量修改。')
  }

  let doc: unknown
  try {
    doc = yamlLoad(existingText)
  } catch (err) {
    const message = err instanceof Error ? err.message.split('\n')[0] : String(err)
    if (/input is empty/.test(message)) {
      throw new DshMergeError('现有文件里没有可解析的 YAML 内容（可能只有注释），无法改写。')
    }
    throw new DshMergeError(`现有文件不是合法的 YAML（${message}），无法改写。`)
  }
  if (doc === null || doc === undefined) {
    throw new DshMergeError('现有文件里没有可解析的 YAML 内容，无法改写。')
  }
  if (!Array.isArray(doc)) {
    throw new DshMergeError('cordis.patch.yml 的顶层应是插件条目数组，当前不是，无法改写。')
  }

  applyDshProvider(doc, providerKey, profile)

  const merged = yamlDump(doc, { indent: 2, lineWidth: -1, noRefs: true })
  return {
    mode: 'merge',
    text: `${merged}\n`,
    warnings: [
      '改写会重新排版 YAML 并丢失文件里的注释；其他插件条目与其他 provider 保留。',
    ],
  }
}

/**
 * 从补丁文件里删除一个 provider。
 *
 * @returns 该 provider 不存在时返回 null
 */
export function removeDshProvider(
  existingText: string,
  providerKey: string,
): DshMergeResult | null {
  let doc: unknown
  try {
    doc = yamlLoad(existingText)
  } catch (err) {
    throw new DshMergeError(
      `现有文件不是合法的 YAML（${err instanceof Error ? err.message.split('\n')[0] : String(err)}），无法改写。`,
    )
  }
  if (!Array.isArray(doc)) {
    throw new DshMergeError('cordis.patch.yml 的顶层应是插件条目数组，当前不是，无法改写。')
  }

  const entry = doc.find(
    (item) =>
      typeof item === 'object' && item !== null && !Array.isArray(item) &&
      (item as Record<string, unknown>).id === DSH_LLM_PLUGIN_ID,
  ) as Record<string, unknown> | undefined
  if (!entry) return null

  const config = entry.config as Record<string, unknown> | undefined
  const providers = config?.providers as Record<string, unknown> | undefined
  if (!providers || !(providerKey in providers)) return null

  delete providers[providerKey]
  const merged = yamlDump(doc, { indent: 2, lineWidth: -1, noRefs: true })
  return {
    mode: 'merge',
    text: `${merged}\n`,
    warnings: ['删除会重新排版 YAML 并丢失注释；其他条目保留。'],
  }
}

/** 在补丁文档数组里定位/创建 llm-pi-ai 条目并写入 provider（同名替换） */
function applyDshProvider(
  doc: unknown[],
  providerKey: string,
  profile: Record<string, unknown>,
): void {
  const entryIndex = doc.findIndex(
    (item) =>
      typeof item === 'object' && item !== null && !Array.isArray(item) &&
      (item as Record<string, unknown>).id === DSH_LLM_PLUGIN_ID,
  )

  if (entryIndex < 0) {
    doc.push({
      id: DSH_LLM_PLUGIN_ID,
      config: { providers: { [providerKey]: profile } },
    })
    return
  }

  const raw = doc[entryIndex] as Record<string, unknown>
  const config =
    typeof raw.config === 'object' && raw.config !== null && !Array.isArray(raw.config)
      ? (raw.config as Record<string, unknown>)
      : raw.config === undefined
        ? {}
        : null
  if (config === null) {
    throw new DshMergeError(`插件条目 ${DSH_LLM_PLUGIN_ID} 的 config 不是对象，无法写入。`)
  }
  const providers =
    typeof config.providers === 'object' && config.providers !== null && !Array.isArray(config.providers)
      ? (config.providers as Record<string, unknown>)
      : config.providers === undefined
        ? {}
        : null
  if (providers === null) {
    throw new DshMergeError('config.providers 不是对象（映射），无法写入。')
  }
  raw.config = { ...config, providers: { ...providers, [providerKey]: profile } }
}
