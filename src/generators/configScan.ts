/**
 * 配置扫描解析：把本机 OpenCode / dsh 配置文件解析成 provider 清单。
 *
 * 配置管理页（「一键识别」）的纯逻辑层：输入文件文本，输出结构化的
 * provider 条目；不碰文件系统（读取由 scan_tool_configs 命令完成），
 * 因此可以纯函数式测试。
 *
 * 容错原则：单个 provider 解析失败只跳过它自己（记入 errors），
 * 不让整份文件失效 —— 用户配置里混着任何奇怪的东西都不该挡住识别。
 */

import { load as yamlLoad } from 'js-yaml'

import { parseJsonc } from '@/utils/jsonc'
import { DSH_LLM_PLUGIN_ID } from './dsh'

// ---- OpenCode ----

/**
 * OpenCode 配置的两个 schema 世代。
 *
 * v2（2026-10 起）重写了键名：顶层 `providers`、provider 的 `package`
 * （带 `aisdk:` 前缀）与 `settings`、模型的 `modelID` 与
 * `capabilities.{tools,input,output}`。v2 读取 v1 键并在内存归一化，
 * **但同一文件里 v2 原生值优先** —— 往 v2 文件里写 v1 键会被无视，
 * 因此读和写都必须跟着文件本身的世代走。
 */
export type OpenCodeSchema = 'v1' | 'v2'

/** OpenCode 配置里识别出的一个 provider */
export interface OpenCodeScanEntry {
  /** provider 键名（路由名） */
  key: string
  /** 该文件使用的 schema 世代 */
  schema: OpenCodeSchema
  /** 显示名 */
  name: string
  /** SDK 包名（v1 npm / v2 package，v2 带 aisdk: 前缀） */
  npm: string
  /** API 根地址 */
  baseURL: string
  /** 密钥状态 */
  apiKey:
    | { mode: 'plain'; value: string }
    | { mode: 'env'; envVar: string }
    | { mode: 'none' }
  /** 模型 id 列表 */
  modelIds: string[]
}

/**
 * 从 provider 容器对象（v1 的 `provider` 或 v2 的 `providers`）解析条目。
 */
function parseOpenCodeContainer(
  container: Record<string, unknown>,
  schema: OpenCodeSchema,
  errors: string[],
): OpenCodeScanEntry[] {
  const entries: OpenCodeScanEntry[] = []
  for (const [key, raw] of Object.entries(container)) {
    if (typeof raw !== 'object' || raw === null || Array.isArray(raw)) {
      errors.push(`provider "${key}" 不是对象，已跳过。`)
      continue
    }
    const provider = raw as Record<string, unknown>
    // v1 的端点配置在 options；v2 在 settings（另有平级 headers/body）
    const holder =
      (typeof provider.options === 'object' && provider.options !== null ? provider.options : null) ??
      (typeof provider.settings === 'object' && provider.settings !== null ? provider.settings : null)
    const options = (holder ?? {}) as Record<string, unknown>

    const apiKeyRaw = typeof options.apiKey === 'string' ? options.apiKey.trim() : ''
    const envRef = parseEnvRef(apiKeyRaw)
    const apiKey: OpenCodeScanEntry['apiKey'] = apiKeyRaw
      ? envRef
        ? { mode: 'env', envVar: envRef }
        : { mode: 'plain', value: apiKeyRaw }
      : { mode: 'none' }

    const models =
      typeof provider.models === 'object' && provider.models !== null
        ? Object.keys(provider.models as Record<string, unknown>)
        : []

    entries.push({
      key,
      schema,
      name: typeof provider.name === 'string' && provider.name ? provider.name : key,
      npm:
        typeof provider.package === 'string' && provider.package
          ? provider.package
          : typeof provider.npm === 'string'
            ? provider.npm
            : '',
      baseURL: typeof options.baseURL === 'string' ? options.baseURL : '',
      apiKey,
      modelIds: models,
    })
  }
  return entries
}

/**
 * 解析 opencode.json / opencode.jsonc 的 provider 清单（v1/v2 双 schema）。
 *
 * 两个世代都读：v2 的 `providers` 与 v1 的 `provider`（v2 迁移期允许混用）。
 */
export function parseOpenCodeProviders(content: string): ScanParseResult<OpenCodeScanEntry> {
  const errors: string[] = []
  let parsed: unknown
  try {
    parsed = parseJsonc(content)
  } catch (err) {
    return {
      entries: [],
      errors: [`文件不是合法 JSON/JSONC：${err instanceof Error ? err.message : String(err)}`],
    }
  }

  const root = parsed as Record<string, unknown> | null
  const entries: OpenCodeScanEntry[] = []
  let found = false

  if (root && typeof root.providers === 'object' && root.providers !== null && !Array.isArray(root.providers)) {
    found = true
    entries.push(...parseOpenCodeContainer(root.providers as Record<string, unknown>, 'v2', errors))
  }
  if (root && typeof root.provider === 'object' && root.provider !== null && !Array.isArray(root.provider)) {
    found = true
    entries.push(...parseOpenCodeContainer(root.provider as Record<string, unknown>, 'v1', errors))
  }
  if (!found) {
    return { entries: [], errors: ['顶层没有 provider(s) 对象。'] }
  }

  return { entries, errors }
}

/** dsh 补丁里识别出的一个 provider */
export interface DshScanEntry {
  /** 路由键名 */
  key: string
  /** 显示名（displayName，缺省用 key） */
  name: string
  /** API 根地址 */
  baseURL: string
  /** 线上协议 */
  api: string
  /** 密钥引用的环境变量名 */
  apiKeyEnv: string
  /** 模型 id 列表 */
  modelIds: string[]
}

/** 解析结果：条目 + 解析过程中被跳过的问题说明 */
export interface ScanParseResult<T> {
  entries: T[]
  errors: string[]
}

/** 从 `{env:VAR}` 引用里取变量名；不是该形态返回 null */
export function parseEnvRef(value: string): string | null {
  const match = /^\{env:(\w+)\}$/.exec(value.trim())
  return match ? match[1] : null
}

/**
 * 取 OpenCode 配置里某个 provider 的**原始对象**（未加工，保留全部字段）。
 *
 * 双 schema：优先在 v2 的 `providers` 里找，其次 v1 的 `provider`
 * （v2 原生值优先的语义与 opencode 自身一致）。返回带世代标记，
 * 调用方（改写回写）据此决定写入哪个容器。
 * 不存在返回 null；文件解析失败抛 SyntaxError。
 */
export function rawOpenCodeProvider(
  content: string,
  key: string,
): { schema: OpenCodeSchema; value: Record<string, unknown> } | null {
  const parsed = parseJsonc(content) as Record<string, unknown> | null
  if (!parsed) return null

  const containers: Array<{ schema: OpenCodeSchema; container: unknown }> = [
    { schema: 'v2', container: parsed.providers },
    { schema: 'v1', container: parsed.provider },
  ]
  for (const { schema, container } of containers) {
    if (typeof container !== 'object' || container === null) continue
    const raw = (container as Record<string, unknown>)[key]
    if (typeof raw === 'object' && raw !== null && !Array.isArray(raw)) {
      return { schema, value: { ...(raw as Record<string, unknown>) } }
    }
  }
  return null
}

/**
 * 取 dsh 补丁里某个 provider 的原始对象（深拷贝）。
 * 不存在返回 null；文件解析失败抛 YAMLException。
 */
export function rawDshProvider(content: string, key: string): Record<string, unknown> | null {
  const doc = yamlLoad(content) as unknown[]
  if (!Array.isArray(doc)) return null
  for (const item of doc) {
    if (typeof item !== 'object' || item === null) continue
    const entry = item as Record<string, unknown>
    if (entry.id !== DSH_LLM_PLUGIN_ID) continue
    const config = entry.config as Record<string, unknown> | undefined
    const providers = config?.providers as Record<string, unknown> | undefined
    const raw = providers?.[key]
    if (typeof raw === 'object' && raw !== null && !Array.isArray(raw)) {
      return JSON.parse(JSON.stringify(raw)) as Record<string, unknown>
    }
  }
  return null
}

/**
 * 由 provider 键名推导建议的环境变量名（绑定密钥 / dsh apiKeyEnv 用）。
 */
export function suggestEnvVarName(providerKey: string): string {
  const cleaned = providerKey.replace(/[^A-Za-z0-9]+/g, '_').replace(/^_+|_+$/g, '')
  return `${cleaned.toUpperCase() || 'PROVIDER'}_API_KEY`
}

/**
 * 解析 dsh cordis.patch.yml 的 provider 清单。
 *
 * 只认 `- id: llm-pi-ai` 条目的 `config.providers`；其他插件条目不在
 * 本功能的管辖范围（那是 dsh 其他插件的配置）。
 */
export function parseDshProviders(content: string): ScanParseResult<DshScanEntry> {
  const errors: string[] = []
  let doc: unknown
  try {
    doc = yamlLoad(content)
  } catch (err) {
    return {
      entries: [],
      errors: [`文件不是合法 YAML：${err instanceof Error ? err.message.split('\n')[0] : String(err)}`],
    }
  }
  if (doc === null || doc === undefined) {
    return { entries: [], errors: ['文件里没有 YAML 内容。'] }
  }
  if (!Array.isArray(doc)) {
    return { entries: [], errors: ['顶层不是插件条目数组。'] }
  }

  const entries: DshScanEntry[] = []
  for (const item of doc) {
    if (typeof item !== 'object' || item === null) continue
    const entry = item as Record<string, unknown>
    if (entry.id !== DSH_LLM_PLUGIN_ID) continue

    const config =
      typeof entry.config === 'object' && entry.config !== null
        ? (entry.config as Record<string, unknown>)
        : null
    const providers = config?.providers
    if (typeof providers !== 'object' || providers === null) {
      errors.push(`${DSH_LLM_PLUGIN_ID} 条目里没有 config.providers，已跳过。`)
      continue
    }
    for (const [key, raw] of Object.entries(providers as Record<string, unknown>)) {
      if (typeof raw !== 'object' || raw === null || Array.isArray(raw)) {
        errors.push(`provider "${key}" 不是对象，已跳过。`)
        continue
      }
      const provider = raw as Record<string, unknown>
      const models = Array.isArray(provider.models)
        ? provider.models
            .map((m) =>
              typeof m === 'object' && m !== null && 'id' in m
                ? String((m as Record<string, unknown>).id)
                : null,
            )
            .filter((v): v is string => v !== null)
        : []

      entries.push({
        key,
        name:
          typeof provider.displayName === 'string' && provider.displayName
            ? provider.displayName
            : key,
        baseURL: typeof provider.baseURL === 'string' ? provider.baseURL : '',
        api: typeof provider.api === 'string' ? provider.api : '',
        apiKeyEnv: typeof provider.apiKeyEnv === 'string' ? provider.apiKeyEnv : '',
        modelIds: models,
      })
    }
  }

  return { entries, errors }
}

/**
 * 识别一份 OpenCode 配置文本的 schema 世代。
 *
 * 规则与 opencodeMerge 的容器选择一致：顶层有 `providers`（复数）按 v2
 * 处理（v2 对 v1 键只做内存归一化，同文件冲突时 v2 原生值胜出）；只有
 * `provider`（单数）才是 v1；两者都没有（空对象/别的配置）或解析失败
 * 返回 null —— 由调用方决定回退（通常是生成器选择的版本）。
 */
export function detectOpenCodeVersion(text: string): OpenCodeSchema | null {
  let parsed: unknown
  try {
    parsed = parseJsonc(text)
  } catch {
    return null
  }
  if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) return null
  const root = parsed as Record<string, unknown>
  if (typeof root.providers === 'object' && root.providers !== null) return 'v2'
  if (typeof root.provider === 'object' && root.provider !== null) return 'v1'
  return null
}
