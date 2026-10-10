/**
 * ============================================================================
 * ModelScout —— provider 命名推断（两个生成器共用）
 * ============================================================================
 *
 * 【为什么单独抽出来】
 *   provider 的「键名 / 显示名 / 环境变量名」三件事，OpenCode 与
 *   DeepSeekHardness 两个生成器都要用，预览界面的输入框默认值也要用。
 *   写在任何一个生成器里都会造成「改了 A 忘了 B」，所以集中在此。
 *
 * 【设计原则：推断只是默认值，不是结论】
 *   推断结果一律会填进输入框，用户可随时改。
 *   推断失败时**绝不抛异常**，而是回退到一个安全值 —— 生成配置这件事
 *   不应该因为一个 URL 长得奇怪就整个失败。
 * ============================================================================
 */

/* ---------------------------------------------------------------------------
 * 公共后缀表
 * ------------------------------------------------------------------------- */

/**
 * 需要剥离两段的公共后缀（`example.co.uk` 的主体是 `example`）。
 *
 * 只列常见的双段后缀；未列出的按单段后缀处理。
 * 这不是一份完整的 Public Suffix List —— 对「推断一个默认名字」
 * 这个用途来说，覆盖常见情况即可，多剥离的风险只是名字略短。
 */
const MULTI_PART_SUFFIXES = new Set([
  'com.cn',
  'org.cn',
  'net.cn',
  'gov.cn',
  'edu.cn',
  'co.uk',
  'org.uk',
  'ac.uk',
  'gov.uk',
  'co.jp',
  'ne.jp',
  'or.jp',
  'com.au',
  'net.au',
  'org.au',
  'com.hk',
  'org.hk',
  'com.tw',
  'org.tw',
  'com.sg',
  'co.kr',
  'or.kr',
  'com.br',
  'com.mx',
  'co.in',
])

/**
 * 通用子域前缀 —— 它们描述的是「这个域名的用途」，不是「它是谁」。
 *
 * 例如 `api.openai.com` 的主体是 `openai` 而不是 `api`。
 */
const GENERIC_LABELS = new Set([
  'api',
  'www',
  'v1',
  'v2',
  'gateway',
  'gw',
  'llm',
  'proxy',
  'openai',
  'chat',
  'serving',
  'inference',
])

/* ---------------------------------------------------------------------------
 * provider key
 * ------------------------------------------------------------------------- */

/** 判断是否为 IPv4 / IPv6 字面量（含方括号形式） */
function isIpLiteral(host: string): boolean {
  if (host.startsWith('[') && host.endsWith(']')) return true // IPv6
  if (/^\d{1,3}(\.\d{1,3}){3}$/.test(host)) return true // IPv4
  return false
}

/** 把任意字符串规整成合法的 provider key（小写、只含 a-z0-9-_.） */
export function sanitizeProviderKey(raw: string): string {
  const cleaned = raw
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9\-_.]+/g, '-')
    .replace(/-{2,}/g, '-')
    .replace(/^[-_.]+|[-_.]+$/g, '')
  return cleaned
}

/**
 * 从 Base URL 推断 provider 键名。
 *
 * 规则（按优先级）：
 *   1. 局域网 / 本机地址（IP、localhost）→ `local`
 *      —— 这类端点没有可辨识的「厂商名」，用 `local` 最诚实
 *   2. 否则取域名主体：先剥离公共后缀，再跳过 `api` / `www` 之类的通用前缀，
 *      取剩下的最后一段
 *
 * 实例：
 *   `https://api.openai.com/v1`                  → `openai`
 *   `https://discovery-api.intern-ai.org.cn/v1`  → `intern-ai`
 *   `http://192.168.66.202:8765/v1`              → `local`
 *   `http://localhost:11434/v1`                  → `local`
 *   （无法解析时）                                → `modelscout`
 */
export function inferProviderKey(baseUrl: string): string {
  const host = extractHost(baseUrl)
  if (!host) return 'modelscout'

  const lowerHost = host.toLowerCase()
  if (lowerHost === 'localhost' || isIpLiteral(lowerHost)) return 'local'

  const labels = lowerHost.split('.').filter(Boolean)
  // 单段主机名（`com`、`myserver`）不是可辨识的域名，推不出有意义的品牌名
  if (labels.length < 2) return 'modelscout'

  // 1) 剥离公共后缀
  let rest = labels
  if (labels.length >= 3 && MULTI_PART_SUFFIXES.has(labels.slice(-2).join('.'))) {
    rest = labels.slice(0, -2)
  } else {
    rest = labels.slice(0, -1)
  }

  // 2) 跳过通用前缀（至少保留一段）
  while (rest.length > 1 && GENERIC_LABELS.has(rest[0])) {
    rest = rest.slice(1)
  }

  // punycode 形式（`xn--...`）是中文等非 ASCII 域名的编码结果，
  // 拿它当 provider 名字毫无可读性，不如回退到通用值。
  // 注意：必须在 sanitize **之前**判断 —— sanitize 会把 `xn--` 的
  // 双连字符合并成单连字符，那时就认不出来了。
  const rawLabel = rest[rest.length - 1] ?? ''
  if (!rawLabel || rawLabel.startsWith('xn--')) return 'modelscout'

  return sanitizeProviderKey(rawLabel) || 'modelscout'
}

/**
 * 从 URL 中取出 hostname。
 *
 * 不直接依赖 `new URL()` —— 用户填的可能是 `192.168.66.202:8765/v1`
 * 这种没有协议头的裸地址，`new URL()` 会抛异常。
 * 这里先尝试标准解析，失败则回退到手动切分。
 */
function extractHost(baseUrl: string): string {
  const trimmed = (baseUrl ?? '').trim()
  if (!trimmed) return ''

  // 带协议头的情况交给标准解析器
  if (/^[a-z][a-z0-9+.-]*:\/\//i.test(trimmed)) {
    try {
      return new URL(trimmed).hostname
    } catch {
      /* 交由下方的手动解析处理 */
    }
  }

  // 裸地址：去掉可能的协议残留、路径、查询串、端口
  return trimmed
    .replace(/^[a-z][a-z0-9+.-]*:\/\//i, '')
    .split(/[/?#]/)[0]
    .split('@')
    .pop()!
    .replace(/:\d+$/, '')
}

/* ---------------------------------------------------------------------------
 * provider 显示名
 * ------------------------------------------------------------------------- */

/**
 * 已知品牌的正确写法。
 *
 * 纯 `title case` 会把 `deepseek` 变成 `Deepseek`、`openai` 变成 `Openai`，
 * 一眼就能看出是机器生成的。这张表让常见品牌保持厂商自己的写法。
 */
const KNOWN_BRANDS: Record<string, string> = {
  ai: 'AI',
  api: 'API',
  openai: 'OpenAI',
  deepseek: 'DeepSeek',
  anthropic: 'Anthropic',
  claude: 'Claude',
  gemini: 'Gemini',
  glm: 'GLM',
  qwen: 'Qwen',
  kimi: 'Kimi',
  minimax: 'MiniMax',
  moonshot: 'Moonshot',
  intern: 'Intern',
  llm: 'LLM',
  gpt: 'GPT',
  ollama: 'Ollama',
  vllm: 'vLLM',
  lmstudio: 'LM Studio',
  siliconflow: 'SiliconFlow',
  openrouter: 'OpenRouter',
  together: 'Together',
  groq: 'Groq',
  mistral: 'Mistral',
  azure: 'Azure',
  bedrock: 'Bedrock',
  vertex: 'Vertex',
  local: 'Local',
  gateway: 'Gateway',
  proxy: 'Proxy',
  modelscout: 'ModelScout',
}

/**
 * 把 provider key 转成好看的显示名。
 *
 * 实例：
 *   `deepseek-local`  → `DeepSeek Local`
 *   `intern-ai`       → `Intern AI`
 *   `openai`          → `OpenAI`
 *   `my_gateway_v2`   → `My Gateway V2`
 */
export function inferProviderName(providerKey: string): string {
  const words = (providerKey ?? '')
    .split(/[-_.\s]+/)
    .map((w) => w.trim())
    .filter(Boolean)

  if (words.length === 0) return 'ModelScout'

  return words
    .map((word) => {
      const lower = word.toLowerCase()
      const known = KNOWN_BRANDS[lower]
      if (known) return known
      return lower.charAt(0).toUpperCase() + lower.slice(1)
    })
    .join(' ')
}

/* ---------------------------------------------------------------------------
 * 环境变量名
 * ------------------------------------------------------------------------- */

/**
 * 由 provider key 推出环境变量名。
 *
 * 实例：
 *   `deepseek-local` → `DEEPSEEK_LOCAL_API_KEY`
 *   `openai`         → `OPENAI_API_KEY`   ← 与官方惯例一致，可直接复用已有变量
 */
export function inferEnvVarName(providerKey: string): string {
  const base = sanitizeProviderKey(providerKey)
    .replace(/[^a-z0-9]+/gi, '_')
    .replace(/^_+|_+$/g, '')
    .toUpperCase()

  return base ? `${base}_API_KEY` : 'MODELSCOUT_API_KEY'
}

/* ---------------------------------------------------------------------------
 * 密钥写入方式
 * ------------------------------------------------------------------------- */

/**
 * 密钥在生成配置中的写法。
 *
 * - `plain` : 直接写明文。**复制即可用**，但配置文件本身就成了敏感文件。
 * - `env`   : 写成 opencode 的 `{env:VAR}` 引用，密钥留在环境变量里。
 *
 * 默认 `plain` —— 用户要的是「一键复制就能跑」，这是最直接的形态；
 * 界面上同时提供 `env` 供需要把配置提交进版本库的场景使用。
 */
export type ApiKeyMode = 'plain' | 'env'

/** 计算最终写进配置的密钥字段值 */
export function resolveApiKeyField(
  mode: ApiKeyMode,
  apiKey: string,
  envVarName: string,
): string {
  if (mode === 'env') {
    return `{env:${sanitizeEnvVarName(envVarName)}}`
  }
  return (apiKey ?? '').trim()
}

/** 规整环境变量名（只允许 A-Z0-9_） */
export function sanitizeEnvVarName(raw: string): string {
  const cleaned = (raw ?? '')
    .trim()
    .toUpperCase()
    .replace(/[^A-Z0-9_]+/g, '_')
    .replace(/^_+|_+$/g, '')
  return cleaned || 'MODELSCOUT_API_KEY'
}

/**
 * 密钥是否算「已填写」。
 *
 * 判断依据刻意宽松：只要非空就算填了。
 * 此处不校验格式 —— 各家网关的 key 前缀格式各异（`sk-` / `glm-` / 纯十六进制），
 * 在这里做格式校验只会误伤。
 */
export function hasApiKey(apiKey: string | undefined): boolean {
  return Boolean(apiKey && apiKey.trim().length > 0)
}
