/**
 * 环境变量引用联动（配置管理「环境变量密钥」子模块用）。
 *
 * 从本机扫描出的 provider 清单里收集 `{env:XXX}` 引用，得到
 * 「变量名 → 引用它的 provider 列表」；再对照机器上实际存在的变量，
 * 就能标出「配置引用了但机器上还没有」的变量（联动创建的入口）。
 */

/** 一个引用来源：哪个工具的哪个 provider */
export interface EnvRefSource {
  tool: 'opencode' | 'dsh'
  providerKey: string
}

/** 可提取 env 引用的最小 provider 形态（来自 configScan 的条目） */
export interface EnvRefLikeOpencode {
  key: string
  apiKey: { mode: 'plain' | 'env' | 'none'; envVar?: string }
}

export interface EnvRefLikeDsh {
  key: string
  apiKeyEnv: string
}

/**
 * 收集全部 env 引用。
 *
 * OpenCode 看 `apiKey.mode === 'env'` 的 `envVar`；dsh 看非空 `apiKeyEnv`。
 * 同一变量被多个 provider 引用时全部保留（顺序 = 传入顺序）。
 */
export function collectEnvVarRefs(
  opencodeEntries: EnvRefLikeOpencode[],
  dshEntries: EnvRefLikeDsh[],
): Map<string, EnvRefSource[]> {
  const refs = new Map<string, EnvRefSource[]>()
  const push = (name: string, source: EnvRefSource) => {
    const trimmed = name.trim()
    if (!trimmed) return
    const list = refs.get(trimmed) ?? []
    list.push(source)
    refs.set(trimmed, list)
  }
  for (const entry of opencodeEntries) {
    if (entry.apiKey.mode === 'env' && entry.apiKey.envVar) {
      push(entry.apiKey.envVar, { tool: 'opencode', providerKey: entry.key })
    }
  }
  for (const entry of dshEntries) {
    if (entry.apiKeyEnv) push(entry.apiKeyEnv, { tool: 'dsh', providerKey: entry.key })
  }
  return refs
}

/**
 * 求出「配置引用了、但机器上还没有」的变量名（区分大小写精确匹配）。
 *
 * 返回按名字排序的数组；这些变量需要创建后配置才能工作。
 */
export function missingEnvVars(
  refs: Map<string, unknown>,
  existingNames: string[],
): string[] {
  const existing = new Set(existingNames)
  return [...refs.keys()].filter((name) => !existing.has(name)).sort()
}
