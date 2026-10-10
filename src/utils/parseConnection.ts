/**
 * ============================================================================
 * ModelScout —— 连接信息识别（从任意文本里抠出 Base URL 与 API Key）
 * ============================================================================
 *
 * 【要解决的问题】
 * 用户拿到端点和密钥的场合，几乎从来不是两个干净的输入框，而是：
 *
 *   - 服务商文档里的一段话：「Base URL: https://xxx/v1，API Key: sk-...」
 *   - 聊天记录里甩过来的两行裸文本
 *   - 一份现成的 opencode.json / config 片段
 *
 * 若要求用户手动挑出两个值再分别粘贴，使用成本过高。
 *
 * 【设计原则：宁可少识别，不可错识别】
 * 这里最危险的失败模式不是「没找到」，而是**「找错了还默默填进去」** ——
 * 用户点「测试连接」拿到 401 或连到别人家端点，排查成本远高于手输一次。
 * 因此：
 *   1. 所有候选都做打分，分数为负的直接丢弃；
 *   2. 落选的候选一并返回，界面可以提示「还看到这些」；
 *   3. 每一条判断都写进 `notes`，让用户能核对依据而不是盲信结果。
 * ============================================================================
 */

/* ---------------------------------------------------------------------------
 * 类型
 * ------------------------------------------------------------------------- */

export interface ExtractResult {
  /** 识别出的 Base URL；未识别到为空字符串 */
  baseUrl: string
  /** 识别出的 API Key；未识别到为空字符串 */
  apiKey: string
  /** 面向用户的说明（识别依据、歧义提示、注意事项） */
  notes: string[]
  /** 全部 URL 候选（含未选中的），按分数降序 */
  urlCandidates: string[]
  /** 全部密钥候选（含未选中的） */
  keyCandidates: string[]
}

/* ---------------------------------------------------------------------------
 * 正则与常量
 * ------------------------------------------------------------------------- */

/**
 * URL 匹配。
 *
 * 字符类里排除掉这些：空白、各类引号、尖括号、圆括号、方括号、
 * 花括号，以及中英文的顿号/逗号/分号/竖线 ——
 * 它们在「URL 出现在句子里」时几乎总是分隔符而不是 URL 的一部分。
 */
const URL_RE = /https?:\/\/[^\s"'`<>()[\]{},;|，；、]+/gi

/** `sk-` 前缀：OpenAI / DeepSeek / Moonshot / 大量聚合网关都用它 */
const KEY_SK_RE = /\bsk-[A-Za-z0-9_-]{12,}/g

/** 带标签的密钥：`apiKey: xxx` / `"api_key": "xxx"` / `密钥：xxx` / `Bearer xxx` */
const KEY_LABELED_RE =
  /(?:api[_\-\s]?key|apikey|access[_\-\s]?token|auth[_\-\s]?token|secret[_\-\s]?key|token|密钥|秘钥|令牌|key)\s*[:=：]\s*["'`]?([A-Za-z0-9_\-.=]{12,})["'`]?/gi

/** `Bearer <token>` */
const KEY_BEARER_RE = /\bBearer\s+([A-Za-z0-9_\-.=]{12,})/gi

/** 纯十六进制长串：有些网关的 key 没有任何前缀 */
const KEY_HEX_RE = /\b[a-fA-F0-9]{32,}\b/g

/**
 * 明显不是 API 端点的主机。
 *
 * 这类链接在「从文档里复制一段话」的场景中极其常见
 * （文档自己会链到 GitHub、官网、参考页），必须先扣分排除，
 * 否则它们很容易盖过真正的端点。
 */
const NON_ENDPOINT_HOSTS = [
  'github.com',
  'gitee.com',
  'opencode.ai',
  'npmjs.com',
  'npmjs.org',
  'google.com',
  'bing.com',
  'baidu.com',
  'stackoverflow.com',
  'zhihu.com',
  'juejin.cn',
  'csdn.net',
  'wikipedia.org',
  'youtube.com',
  'twitter.com',
  'discord.gg',
  'localhost:1420', // 本项目的 dev server，需排除以免误判为端点
]

/** 文件名式的结尾 —— 是「文档链接」而不是「API 端点」 */
const FILE_LIKE_TAIL_RE = /\.(md|markdown|json|jsonc|html?|png|jpe?g|gif|svg|txt|ya?ml|toml|zip|pdf)$/i

/** 常见顶级域名 —— 用于识别「这实际是主机名，不是密钥」 */
const TLD_TAIL_RE =
  /\.(com|cn|org|net|io|ai|dev|app|xyz|top|info|biz|co|me|cc|tv|site|online|tech|cloud|link|pro|store|fun|space|website|host)(\/|$|:)/i

/** JSON 里可能承载 Base URL 的字段名（统一小写比较） */
const URL_FIELD_NAMES = new Set([
  'baseurl',
  'base_url',
  'url',
  'endpoint',
  'api_base',
  'apibase',
  'api_url',
  'apiurl',
  'server',
  'address',
  'host',
])

/** JSON 里可能承载密钥的字段名（统一小写比较） */
const KEY_FIELD_NAMES = new Set([
  'apikey',
  'api_key',
  'key',
  'token',
  'access_token',
  'auth_token',
  'secret',
  'secretkey',
  'secret_key',
  'password',
])

/* ---------------------------------------------------------------------------
 * 主函数
 * ------------------------------------------------------------------------- */

/**
 * 从一段文本中识别 Base URL 与 API Key。
 *
 * 流程：JSON 结构化提取（如果输入是 JSON）→ 正则提取 → 打分筛选。
 * 两条路径的结果会合并去重，因为正则对 JSON 文本同样有效，
 * 而 JSON 解析能额外覆盖「字段名对但值没有明显特征」的情况。
 */
export function extractConnection(raw: string): ExtractResult {
  const text = (raw ?? '').trim()
  const notes: string[] = []

  if (!text) {
    return { baseUrl: '', apiKey: '', notes: ['没有输入任何内容。'], urlCandidates: [], keyCandidates: [] }
  }

  // ---- 1. URL 候选 ----
  const urlSpans: Array<[number, number]> = []
  const rawUrls: string[] = []
  for (const m of text.matchAll(URL_RE)) {
    const start = m.index ?? 0
    urlSpans.push([start, start + m[0].length])
    rawUrls.push(normalizeUrl(m[0]))
  }

  // ---- 2. 密钥候选（带位置，便于排除落在 URL 里的误判）----
  const keyHits: Array<{ value: string; start: number; end: number; weight: number }> = []

  const collect = (re: RegExp, weight: number, group = 0) => {
    for (const m of text.matchAll(re)) {
      const value = (group === 0 ? m[0] : m[group]) ?? ''
      if (!value) continue
      // 计算该捕获组在原文中的大致位置（用于与 URL 区间比对）
      const offsetInMatch = group === 0 ? 0 : m[0].indexOf(value)
      const start = (m.index ?? 0) + Math.max(0, offsetInMatch)
      keyHits.push({ value, start, end: start + value.length, weight })
    }
  }

  collect(KEY_SK_RE, 100)
  collect(KEY_LABELED_RE, 80, 1)
  collect(KEY_BEARER_RE, 70, 1)
  collect(KEY_HEX_RE, 40)

  // ---- 3. JSON 结构化提取（输入是 JSON 时）----
  const jsonFindings = extractFromJson(text)
  for (const v of jsonFindings.baseUrls) {
    const trimmed = normalizeUrl(v)
    if (trimmed && !rawUrls.includes(trimmed)) rawUrls.push(trimmed)
  }
  for (const v of jsonFindings.keys) {
    keyHits.push({ value: v, start: -1, end: -1, weight: 90 })
  }

  // ---- 4. 打分筛选 ----
  const scoredUrls = dedupe(rawUrls)
    .map((url) => ({ url, score: scoreUrl(url) }))
    .sort((a, b) => b.score - a.score)

  const usableUrls = scoredUrls.filter((c) => c.score > 0)
  const baseUrl = usableUrls[0]?.url ?? ''

  const usableKeys = dedupe(
    keyHits
      // 落在 URL 内部的候选一定是误判（例如把域名当成了密钥）
      .filter((hit) => !overlapsAny(hit.start, hit.end, urlSpans))
      .filter((hit) => !looksLikeHostname(hit.value))
      .sort((a, b) => b.weight - a.weight || b.value.length - a.value.length)
      // 去掉「被更高分候选整个包住」的冗余项。
      // 典型例子：`sk-8090…aab` 会同时命中 sk- 规则（权重 100）和
      // 纯十六进制规则（权重 40），后者实际是前者的后缀 ——
      // 不清理的话界面会显示「另有 1 个候选」，白白制造疑虑。
      //
      // 注意：必须用**严格更长**做判据，不能用 `!==`：
      // 同一个值常常被两条规则同时命中（`apiKey: sk-xxx` 同时命中
      // 「带标签」与「sk- 前缀」两条）。若只比较对象身份，
      // 两个候选会互相认为「对方包含我」而**双双被删掉** —— 结果是
      // 明明有密钥却识别不出来。相同值交给 dedupe 去重即可。
      .filter(
        (hit, _i, arr) =>
          !arr.some((other) => other.value.length > hit.value.length && other.value.includes(hit.value)),
      )
      .map((hit) => hit.value),
  )

  const apiKey = usableKeys[0] ?? ''

  // ---- 5. 生成说明 ----
  if (baseUrl) {
    const runnerUp = usableUrls[1]
    notes.push(`Base URL：${baseUrl}${runnerUp ? `（另有候选 ${runnerUp.url}）` : ''}`)
    if (!/\/v\d+/i.test(baseUrl)) {
      notes.push('该地址路径里没有 /v1 之类的版本段，若连接失败可尝试补上 /v1。')
    }
  } else if (scoredUrls.length > 0) {
    notes.push(`看到了 ${scoredUrls.length} 个链接，但都不像 API 端点，已忽略。`)
  } else {
    notes.push('没找到 Base URL —— 请确认文本里包含 https:// 开头的地址。')
  }

  if (apiKey) {
    const others = usableKeys.length - 1
    notes.push(`API Key：${maskSecret(apiKey)}${others > 0 ? `（另有 ${others} 个候选）` : ''}`)
  } else {
    notes.push('没找到 API Key —— 请确认文本里包含密钥本身（通常以 sk- 开头）。')
  }

  return {
    baseUrl,
    apiKey,
    notes,
    urlCandidates: scoredUrls.map((c) => c.url),
    keyCandidates: usableKeys,
  }
}

/* ---------------------------------------------------------------------------
 * URL 打分
 * ------------------------------------------------------------------------- */

/**
 * 具体接口路径后缀 —— 它们不是 Base URL 的一部分。
 *
 * 用户从 curl 示例、SDK 文档里复制时，拿到的常常是**完整接口地址**
 * （`https://api.deepseek.com/v1/chat/completions`），
 * 而 Base URL 只到 `/v1`。多出来的这段会让 AI SDK 拼出
 * `/v1/chat/completions/chat/completions` 这种双份路径。
 */
const ENDPOINT_TAIL_RE =
  /\/(chat\/completions|completions|responses|messages|embeddings|images\/generations|audio\/transcriptions|audio\/translations|models)$/i

/**
 * 规整 URL：去句末标点 → 去尾斜杠 → 去掉具体接口路径 → 再去尾斜杠。
 *
 * 顺序有讲究：句末标点必须在最前面去掉，否则 `.../v1/models。` 里
 * `models` 后面跟的是中文句号，`ENDPOINT_TAIL_RE` 的 `$` 匹配不上。
 */
function normalizeUrl(url: string): string {
  const withoutPunctuation = url.replace(/[.,;:!?、，。；：！？"'）】》]+$/g, '').replace(/\/+$/, '')
  return withoutPunctuation.replace(ENDPOINT_TAIL_RE, '').replace(/\/+$/, '')
}

/** 把 URL 拆成 host 与 path（解析失败时退化为字符串切分） */
function splitUrl(url: string): { host: string; path: string } {
  try {
    const u = new URL(url)
    return { host: u.hostname.toLowerCase(), path: u.pathname.replace(/\/+$/, '').toLowerCase() }
  } catch {
    const rest = url.replace(/^https?:\/\//i, '')
    const slash = rest.indexOf('/')
    return slash < 0
      ? { host: rest.toLowerCase(), path: '' }
      : { host: rest.slice(0, slash).toLowerCase(), path: rest.slice(slash).replace(/\/+$/, '').toLowerCase() }
  }
}

/**
 * 给一个 URL 打「像不像 API 端点」的分。
 *
 * 分数为 0 或负数的候选会被直接丢弃 —— 宁可让用户手输，
 * 也不能把一个文档链接填进 Base URL。
 */
export function scoreUrl(url: string): number {
  const lower = url.toLowerCase()
  const { host, path } = splitUrl(lower)
  let score = 0

  // 主机名里带 api 是强信号
  if (/^api[.\-]/.test(host) || host.startsWith('api') || /[.\-]api[.\-]/.test(host)) {
    score += 3
  }

  // 路径以版本号结尾 —— OpenAI 兼容端点的典型形态
  if (/\/v\d+$/.test(path)) score += 4
  else if (/\/v\d+/.test(path)) score += 2

  // 常见的兼容路径形态
  if (/compatible-mode|\/openai|\/api\/|anthropic|\/paas\/|\/v1\/chat/.test(path)) score += 2

  // 本地 / 内网地址：几乎一定是用户自己的服务
  if (/^(localhost|127\.|0\.0\.0\.0|192\.168\.|10\.|172\.(1[6-9]|2\d|3[01])\.)/.test(host)) {
    score += 3
  }

  // 负信号
  if (FILE_LIKE_TAIL_RE.test(path)) score -= 6
  // 主机名与完整 URL 都比一遍：`localhost:1420` 这类带端口的黑名单项
  // 只在完整 URL 里才看得出来（host 字段是不带端口的）
  if (NON_ENDPOINT_HOSTS.some((h) => host.includes(h) || lower.includes(h))) score -= 8
  // 仅有域名而无路径的，多半是官网而不是端点
  if (path === '') score -= 2

  return score
}

/* ---------------------------------------------------------------------------
 * 密钥判定辅助
 * ------------------------------------------------------------------------- */

/** 判断候选是否落在某个 URL 的字符区间内 */
function overlapsAny(start: number, end: number, spans: Array<[number, number]>): boolean {
  if (start < 0) return false
  return spans.some(([s, e]) => start < e && end > s)
}

/**
 * 判断候选「看起来是个主机名 / 域名」而不是密钥。
 *
 * 为什么需要：密钥正则是宽松的 `[A-Za-z0-9_\-.]{12,}`，
 * 而 `discovery-api.intern-ai.org.cn` 完全符合这个字符集。
 * 若文本里的 URL 没有写协议头（例如用户只粘了域名），
 * 这个域名会先于真正的密钥被选中。
 */
export function looksLikeHostname(value: string): boolean {
  if (TLD_TAIL_RE.test(value)) return true
  // 同时含 `.` 与 `-`、且没有任何密钥常见特征（下划线/大写混合）的，按域名处理
  if (value.includes('.') && value.includes('-') && !/[A-Z]/.test(value) && !value.includes('_')) {
    return true
  }
  return false
}

/** 从 JSON 文本里按字段名捞 Base URL 与密钥（递归，兼容嵌套配置） */
function extractFromJson(text: string): { baseUrls: string[]; keys: string[] } {
  const baseUrls: string[] = []
  const keys: string[] = []

  let parsed: unknown
  try {
    parsed = JSON.parse(text)
  } catch {
    return { baseUrls, keys }
  }

  const walk = (node: unknown): void => {
    if (Array.isArray(node)) {
      node.forEach(walk)
      return
    }
    if (!node || typeof node !== 'object') return

    for (const [key, value] of Object.entries(node as Record<string, unknown>)) {
      const name = key.toLowerCase()
      if (typeof value === 'string') {
        if (URL_FIELD_NAMES.has(name) && /^https?:\/\//i.test(value.trim())) {
          baseUrls.push(value.trim())
        } else if (KEY_FIELD_NAMES.has(name) && value.trim().length >= 8) {
          keys.push(value.trim())
        }
      }
      walk(value)
    }
  }

  walk(parsed)
  return { baseUrls, keys }
}

/* ---------------------------------------------------------------------------
 * 展示辅助
 * ------------------------------------------------------------------------- */

/** 去重但保持首次出现顺序 */
function dedupe(list: string[]): string[] {
  return Array.from(new Set(list))
}

/**
 * 密钥脱敏：只露出头尾，中间打码。
 *
 * 界面上要告诉用户「识别到了哪个 key」，但没必要把完整密钥再显示一遍 ——
 * 截屏、投屏、录屏都可能把它带出去。
 */
export function maskSecret(secret: string): string {
  const s = (secret ?? '').trim()
  if (!s) return ''
  if (s.length <= 8) return '***'
  if (s.length <= 16) return `${s.slice(0, 4)}***${s.slice(-2)}`
  return `${s.slice(0, 6)}…${s.slice(-4)}`
}
