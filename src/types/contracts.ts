/**
 * 接口契约：前后端对齐的唯一事实来源。
 *
 * 本文件是前端 TypeScript 侧对 Rust 后端数据结构的镜像定义。
 * 每一个 interface 都必须与 `src-tauri/src/models.rs` 中的 #[derive(Serialize)]
 * 结构体一一对应；字段名完全一致（Rust 侧统一使用
 * #[serde(rename_all = "camelCase")]，因此这里全部为小驼峰）。
 *
 * 修改约定：任何字段增删都必须三处同步——
 *   1. src/types/contracts.ts  （本文件）
 *   2. src-tauri/src/models.rs （Rust 结构体）
 *   3. src/api/commands.ts     （若涉及命令签名）
 * 否则交叉核对脚本会失败。
 */

// ---- 基础枚举（字符串字面量联合类型，对应 Rust camelCase 枚举序列化结果） ----

/**
 * 探测错误的分类。
 * 与 Rust 端 `error::ProbeErrorKind` 枚举一一对应。
 *
 * - `keyInvalid`      : HTTP 401 / 403 —— API Key 无效或权限不足
 * - `baseUrlInvalid`  : HTTP 404 / 域名解析失败 —— Base URL 写错
 * - `networkTimeout`  : 连接超时 / 读超时 —— 网络问题
 * - `rateLimited`     : HTTP 429 —— 触发限流，建议降低并发
 * - `serverError`     : HTTP 5xx —— 服务端故障
 * - `modelNotFound`   : 该模型对当前 Key 不可用（404 但 URL 本身正确）
 * - `parseError`      : 响应不是合法 JSON 或结构不符合 OpenAI 规范
 * - `cancelled`       : 用户主动中止
 * - `unknown`         : 未能归类的其他错误
 */
export type ProbeErrorKind =
  | 'keyInvalid'
  | 'baseUrlInvalid'
  | 'networkTimeout'
  | 'rateLimited'
  | 'serverError'
  | 'modelNotFound'
  | 'parseError'
  | 'cancelled'
  | 'unknown'

/**
 * 能力探测的三态结果。
 * 与 Rust 端 `models::TriState` 枚举一一对应。
 *
 * - `supported`    : 明确探测到支持
 * - `unsupported`  : 明确探测到不支持
 * - `unknown`      : 探测失败或无法判定（例如上游直接返回 500）
 */
export type TriState = 'supported' | 'unsupported' | 'unknown'

/**
 * 上下文 / 输出上限的取值来源，用于在前端区分「可信度」。
 * 与 Rust 端 `models::LimitSource` 枚举一一对应。
 *
 * - `declared`    : 接口在 /models 中直接声明，最可信（服务端自报）
 * - `dictionary`  : 命中本地参数字典，可信度高
 * - `errorParse`  : 通过超大 max_tokens 触发报错后解析得出
 * - `fallback`    : 使用回退值（保守默认值）
 * - `unknown`     : 未能获取，前端应展示为「未知」而非 0
 */
export type LimitSource = 'declared' | 'dictionary' | 'errorParse' | 'fallback' | 'unknown'

/**
 * 探测任务整体的状态机。
 * 与 Rust 端 `models::ProbeStatus` 枚举一一对应。
 */
export type ProbeStatus = 'idle' | 'running' | 'cancelling' | 'done' | 'cancelled' | 'failed'

/**
 * 单个模型在探测流水线中的阶段，用于进度面板展示。
 * 与 Rust 端 `models::ProbeStage` 枚举一一对应。
 */
export type ProbeStage =
  | 'pending'
  | 'listing'
  | 'toolProbe'
  | 'visionProbe'
  | 'reasoningProbe'
  | 'limitProbe'
  | 'done'
  | 'failed'

/**
 * API Key 的来源方式。
 * 与 Rust 端 `models::ApiKeySource` 枚举一一对应。
 *
 * - `plain`      : 直接使用 `apiKey` 字段的明文（默认，兼容旧行为）
 * - `env`        : 从 `apiKeyEnvVar` 指定的环境变量读取（由后端读取）
 * - `credential` : 从 `apiKeyCredential` 指定的 Windows 凭据管理器通用凭据读取
 *                  （由后端读取）
 *
 * 后两种方式下 `apiKey` 应留空 —— 明文由后端在发起请求前解析，
 * 前端自始至终不接触它。
 */
export type ApiKeySource = 'plain' | 'env' | 'credential'

// ---- 命令入参 / 出参结构体 ----

/**
 * 传给 `start_probe` 命令的探测配置。
 * 对应 Rust 结构体：`models::ProbeConfig`
 *
 * 注意：`apiKey` 只在 invoke 时向前端 → Rust 单向传递，
 * Rust 端持有它但绝不在任何 log / 事件 payload 中回传或打印。
 *
 * 密钥来源：`apiKeySource` 为 `plain` 时用 `apiKey`；
 * 为 `env` / `credential` 时改用 `apiKeyEnvVar` / `apiKeyCredential`，
 * 由后端读取真正的明文（前端不接触）。
 */
export interface ProbeConfig {
  /** API 根地址，例如 `https://api.openai.com/v1`（末尾斜杠会被规范化去除） */
  baseUrl: string
  /** API Key，Rust 端只用于构造 Authorization 头，不落盘、不打日志 */
  apiKey: string
  /** 并发数，1-10。Rust 端会在 1..=10 范围内 clamp */
  concurrency: number
  /** 单请求超时（秒），Rust 端会在 5..=300 范围内 clamp */
  timeoutSecs: number
  /**
   * 是否执行「超大 max_tokens」探测以反推上下文/输出上限。
   * 关闭可显著提速，但未命中外置字典的模型将拿不到真实上限。
   */
  probeLimits: boolean
  /**
   * 可选的模型 ID 白名单。
   * 为空数组（或 undefined）表示探测 `/models` 返回的全部模型。
   */
  onlyModels?: string[]
  /** API Key 的来源方式，默认 `plain` */
  apiKeySource: ApiKeySource
  /** 当 `apiKeySource = 'env'` 时要读取的环境变量名 */
  apiKeyEnvVar: string
  /** 当 `apiKeySource = 'credential'` 时要读取的 Windows 凭据名 */
  apiKeyCredential: string
}

/**
 * 探测完成后的单个模型全量信息。
 * 对应 Rust 结构体：`models::ModelInfo`
 *
 * 这是结果表格与配置生成器的数据源。
 */
export interface ModelInfo {
  /** 模型 ID，例如 `gpt-4o-2024-08-06`（来自 /models 接口的 id 字段） */
  id: string
  /** 显示名称，取自 /models 的 name 字段；若为空则回退为 id */
  displayName: string
  /** 所属方（/models 的 owned_by 字段），可能为空字符串 */
  ownedBy: string
  /** 创建时间戳（Unix 秒），0 表示上游未提供 */
  created: number

  /** 是否支持工具调用（Function Calling） */
  supportsTools: TriState
  /** 是否支持附件/图片上传（Vision） */
  supportsVision: TriState
  /** 是否支持流式输出。若 /models 未提供则以「探测请求成功」推断 */
  supportsStreaming: TriState

  /** 是否为推理模型（名称正则 或 响应含 reasoning_content） */
  isReasoning: boolean
  /** 判定为推理模型的依据，用于在前端 tooltip 中向用户解释 */
  reasoningEvidence: string

  /** 上下文窗口上限（token 数），0 表示未知 */
  contextLimit: number
  /** 单次最大输出上限（token 数），0 表示未知 */
  maxOutputLimit: number
  /** 上下文上限的来源 */
  contextLimitSource: LimitSource
  /** 输出上限的来源 */
  maxOutputLimitSource: LimitSource

  /**
   * 能力数据的整体来源说明（中文），例如「接口声明（schema 2.4）：上下文、工具调用、推理」。
   *
   * 展示原因：部分网关的 `/models` 声明可能与官方文档不符
   * （例如接口称某模型支持 tools、文档称不支持）。标出来源，
   * 用户才能判断该采信哪一个值。
   */
  capsSource: string

  /** 该模型探测过程中的错误列表（空数组表示全程无错） */
  errors: ProbeError[]
  /** 该模型探测耗时（毫秒） */
  elapsedMs: number
  /** 该模型探测结束时的终态阶段 */
  finalStage: ProbeStage
}

/**
 * 一条探测错误。
 * 对应 Rust 结构体：`models::ProbeError`
 */
export interface ProbeError {
  /** 错误分类 */
  kind: ProbeErrorKind
  /** 面向用户的简短中文说明（Rust 端生成，前端直接展示） */
  message: string
  /** HTTP 状态码，0 表示非 HTTP 层错误 */
  httpStatus: number
  /** 触发该错误的探测环节，例如 `toolProbe` / `limitProbe` */
  stage: ProbeStage
}

/**
 * `start_probe` / `probe_models` / `stop_probe` / `get_probe_state` 的统一返回。
 * 对应 Rust 结构体：`models::ProbeRunSummary`
 */
export interface ProbeRunSummary {
  /** 本次运行的唯一 ID（Unix 毫秒时间戳字符串），用于前端丢弃过期事件 */
  runId: string
  /** 任务整体状态 */
  status: ProbeStatus
  /** 探测成功的模型数量 */
  totalModels: number
  /** 成功获取到全量信息的模型列表 */
  models: ModelInfo[]
  /** 探测过程中的全局错误（例如拉取 /models 列表就失败了） */
  errors: ProbeError[]
  /** 任务总耗时（毫秒） */
  elapsedMs: number
}

/**
 * `get_probe_state` 的返回：包含当前实时状态与已完成模型。
 * 对应 Rust 结构体：`models::ProbeSnapshot`
 */
export interface ProbeSnapshot {
  /** 当前是否有任务在运行 */
  isRunning: boolean
  /** 当前运行 ID；无任务时为空字符串 */
  runId: string
  /** 当前状态 */
  status: ProbeStatus
  /** 已发现（/models 返回）的模型总数 */
  totalModels: number
  /** 已完成探测的模型数量 */
  completedModels: number
  /** 截至此刻已完成探测的模型全量信息（表驱动 UI 可增量渲染） */
  models: ModelInfo[]
}

/**
 * `list_models` 命令的返回：仅拉取模型列表，不做能力探测。
 * 对应 Rust 结构体：`models::ModelListResult`
 */
export interface ModelListResult {
  /** 发现的模型条目（仅 id/displayName/ownedBy/created 有效） */
  models: ModelBrief[]
  /** 拉取过程中的错误（空数组表示成功） */
  errors: ProbeError[]
  /** 拉取耗时（毫秒） */
  elapsedMs: number
}

/**
 * 接口在 `/models` 响应中「自报」的能力（OpenRouter / 聚合网关的扩展 schema）。
 * 对应 Rust 结构体：`models::DeclaredCaps`
 *
 * 语义关键：字段为 `null` = 接口未声明，需要真实请求探测；
 * 有值 = 直接采信声明，跳过对应的探测请求（省额度、更快、更准）。
 */
export interface DeclaredCaps {
  /** 上下文窗口上限（token） */
  contextLimit: number | null
  /** 单次输出上限（token） */
  maxOutputLimit: number | null
  /** 是否支持工具调用 */
  supportsTools: boolean | null
  /** 是否支持图片输入（视觉/附件） */
  supportsVision: boolean | null
  /**
   * 是否为推理模型（`supported_parameters` 中存在 `reasoning`）。
   *
   * 注意：这比「响应里出现 reasoning_content」更可靠——部分网关
   * 对所有模型都会返回 `reasoning_content` 字段（可能为空），
   * 靠响应体判断会把非推理模型误判为推理模型。
   */
  supportsReasoning: boolean | null
  /** 接口是否标记为就绪 */
  isReady: boolean | null
  /** 扩展 schema 版本，仅用于排障展示 */
  schemaVersion: string
}

/**
 * 轻量模型条目，仅列表信息。
 * 对应 Rust 结构体：`models::ModelBrief`
 */
export interface ModelBrief {
  id: string
  displayName: string
  ownedBy: string
  created: number
  /** 接口自报的能力；为空表示未声明，需真实探测 */
  declared: DeclaredCaps
}

/**
 * `ping_connection` 命令的返回：网络连通性与错误分类预检。
 * 对应 Rust 结构体：`models::PingResult`
 */
export interface PingResult {
  /** 是否连通且鉴权通过 */
  ok: boolean
  /** HTTP 状态码，0 表示未拿到响应 */
  httpStatus: number
  /** 分类后的错误；ok 为 true 时该字段为 null */
  error: ProbeError | null
  /** 往返耗时（毫秒） */
  elapsedMs: number
  /** 探测到的模型数量（ok 为 true 时有效） */
  modelCount: number
  /** 实际请求的 URL（已脱敏，不含 Key） */
  resolvedUrl: string
}

/**
 * `save_config_file` 命令的入参。
 * 对应 Rust 结构体：`models::SaveConfigRequest`
 *
 * 注意：安全约束 —— `fileName` 不得包含路径分隔符（`/` `\`）或 `..`，
 * Rust 端会做校验并拒绝非法文件名，防止路径穿越。
 */
export interface SaveConfigRequest {
  /** 建议的文件名（不含目录），例如 `opencode.json` */
  fileName: string
  /** 文件正文 */
  content: string
}

/**
 * `save_config_file` 命令的返回。
 * 对应 Rust 结构体：`models::SaveConfigResult`
 *
 * 文件会写入应用数据目录下的 `exports/` 子目录；
 * 同名文件不会被覆盖，而是追加 `-1` `-2` 等后缀。
 */
export interface SaveConfigResult {
  /** 是否保存成功 */
  saved: boolean
  /** 实际写入的绝对路径；取消保存时为空字符串 */
  path: string
}

/**
 * `inspect_tool_config` 命令的返回。
 * 对应 Rust 结构体：`models::ToolConfigFile`
 *
 * 「一键填写」用它读取目标工具（OpenCode / dsh）的现有配置文件，
 * 生成左右对比预览。文件不存在不是错误（`exists=false` + 空内容），
 * 前端据此走「空文件补全」分支。
 */
export interface ToolConfigFile {
  /** 解析后的目标文件绝对路径（已做 `~` 展开与默认路径推断） */
  path: string
  /** 文件是否存在 */
  exists: boolean
  /** 文件内容（UTF-8）；不存在时为空字符串 */
  content: string
}

/**
 * `write_tool_config` 命令的返回。
 * 对应 Rust 结构体：`models::ToolConfigWriteResult`
 *
 * 用户在一键填写对比预览里确认后，把合并结果写回目标文件；
 * 覆盖已有文件前会创建 `<原名>.modelscout.bak` 备份。
 */
export interface ToolConfigWriteResult {
  /** 实际写入的绝对路径 */
  path: string
  /** 写入前创建的备份文件路径；原文件不存在（未备份）时为 null */
  backupPath: string | null
  /** 写入的字节数 */
  bytesWritten: number
}

// ---- 事件 payload 结构体 ----
/**
 * 事件 `probe://progress` 的 payload。
 * 对应 Rust 结构体：`models::ProbeProgress`
 *
 * 说明：Rust 端采用事件名与 payload 一同注入的设计，
 * payload 内自带 `runId`，前端据此丢弃过期运行的事件。
 */
export interface ProbeProgress {
  /** 所属运行 ID；前端应比对当前 runId，不一致则忽略 */
  runId: string
  /** 当前正在处理的模型 ID */
  modelId: string
  /** 该模型当前所处阶段 */
  stage: ProbeStage
  /** 该阶段的人类可读描述（中文） */
  stageLabel: string
  /** 已完成的模型数 */
  completed: number
  /** 总模型数 */
  total: number
  /** 整体完成百分比（0-100，Rust 端计算好，前端直接用于进度条） */
  percent: number
  /** 该阶段耗时（毫秒），未完成阶段为 0 */
  elapsedMs: number
}

/**
 * 事件 `probe://log` 的 payload。
 * 对应 Rust 结构体：`models::ProbeLog`
 */
export interface ProbeLog {
  /** 所属运行 ID */
  runId: string
  /** 日志级别 */
  level: 'debug' | 'info' | 'warn' | 'error'
  /** 关联的模型 ID；全局日志为空字符串 */
  modelId: string
  /** 日志正文（中文）。Rust 端保证正文中不含完整 API Key */
  message: string
  /** 事件产生的 Unix 毫秒时间戳 */
  timestamp: number
}

/**
 * 事件 `probe://model-done` 的 payload：单个模型探测完成。
 * 对应 Rust 结构体：`models::ProbeModelDone`
 */
export interface ProbeModelDone {
  /** 所属运行 ID */
  runId: string
  /** 完成探测的模型全量信息 */
  model: ModelInfo
  /** 已完成数量 */
  completed: number
  /** 总数 */
  total: number
}

/**
 * 事件 `probe://finished` 的 payload：整轮探测结束（正常完成或被中止）。
 * 对应 Rust 结构体：`models::ProbeFinished`
 */
export interface ProbeFinished {
  /** 所属运行 ID */
  runId: string
  /** 终态：`done` | `cancelled` | `failed` */
  status: ProbeStatus
  /** 成功完成的模型数 */
  completed: number
  /** 总数 */
  total: number
  /** 总耗时（毫秒） */
  elapsedMs: number
  /** 全局错误列表 */
  errors: ProbeError[]
}

/**
 * 事件 `probe://error` 的 payload：导致整轮任务终止的致命错误。
 * 对应 Rust 结构体：`models::ProbeFatal`
 */
export interface ProbeFatal {
  /** 所属运行 ID */
  runId: string
  /** 分类后的错误 */
  error: ProbeError
  /** 致命错误导致任务在哪个阶段终止 */
  stage: ProbeStage
}

// ---- 事件名常量（唯一集中定义，Rust 端 emit 的字符串必须与此完全一致） ----

/**
 * 全部 Tauri 事件名常量。
 * 需与 `src-tauri/src/probe.rs` 中的 emit 字面量逐一核对保持一致。
 */
export const EVENT_NAMES = {
  /** 进度更新（逐阶段） */
  PROGRESS: 'probe://progress',
  /** 日志行 */
  LOG: 'probe://log',
  /** 单个模型探测完成 */
  MODEL_DONE: 'probe://model-done',
  /** 整轮探测结束 */
  FINISHED: 'probe://finished',
  /** 致命错误 */
  ERROR: 'probe://error',
  /* ---- Key 管家（Beta） ---- */
  /** 批量检测进度（逐 key、逐阶段） */
  KEY_BUTLER_PROGRESS: 'key-butler://progress',
  /** 单条 Key 检测完成 */
  KEY_BUTLER_ENTRY_DONE: 'key-butler://entry-done',
  /** 整批检测结束 */
  KEY_BUTLER_FINISHED: 'key-butler://finished',
} as const

/** 事件名联合类型 */
export type EventName = (typeof EVENT_NAMES)[keyof typeof EVENT_NAMES]

// ---- 命令名常量（唯一集中定义，Rust 端 generate_handler 注册名必须与此一致） ----

/**
 * 全部 Tauri 命令名常量。
 * 需与 `src-tauri/src/lib.rs` 中 `generate_handler![]` 的注册名逐一核对保持一致。
 */
export const COMMAND_NAMES = {
  /** 仅拉取模型列表 */
  LIST_MODELS: 'list_models',
  /** 连通性 + 鉴权预检 */
  PING_CONNECTION: 'ping_connection',
  /** 启动完整探测流水线（异步，进度走事件） */
  START_PROBE: 'start_probe',
  /** 请求中止当前探测 */
  STOP_PROBE: 'stop_probe',
  /** 查询当前探测快照 */
  GET_PROBE_STATE: 'get_probe_state',
  /** 导出配置到用户选择的位置（供「保存文件」使用） */
  SAVE_CONFIG_FILE: 'save_config_file',
  /** 一键填写：读取目标工具（opencode/dsh）的现有配置文件 */
  INSPECT_TOOL_CONFIG: 'inspect_tool_config',
  /** 一键填写：用户确认后把合并结果写入目标配置文件 */
  WRITE_TOOL_CONFIG: 'write_tool_config',
  /* ---- Key 管家（Beta）：全部带 key_butler_ 前缀，避免与探测命令冲突 ---- */
  /** 批量文本 → 提取 Key → 前缀识别 → 入列（去重） */
  KEY_BUTLER_IMPORT: 'key_butler_import',
  /** 单/批量检测（有效性 → 余额 → 分类 → 快照），进度走事件 */
  KEY_BUTLER_CHECK: 'key_butler_check',
  /** 请求中止当前批量检测 */
  KEY_BUTLER_CHECK_CANCEL: 'key_butler_check_cancel',
  /** 返回脱敏列表 */
  KEY_BUTLER_LIST: 'key_butler_list',
  /** 返回单个完整 Key（仅用于一键复制） */
  KEY_BUTLER_REVEAL: 'key_butler_reveal',
  /** 删除一条 Key */
  KEY_BUTLER_DELETE: 'key_butler_delete',
  /** 修改备注 */
  KEY_BUTLER_UPDATE_NOTE: 'key_butler_update_note',
  /** 修改 Base URL 与余额接口配置 */
  KEY_BUTLER_SET_CONFIG: 'key_butler_set_config',
  /** 一次修改多条 Key（平台 / Base URL / 余额接口 / 备注） */
  KEY_BUTLER_UPDATE_MANY: 'key_butler_update_many',
  /** 启用加密保存（设主密码并加密现有列表） */
  KEY_BUTLER_VAULT_ENABLE: 'key_butler_vault_enable',
  /** 启用本地保存（无密码模式：随机数据密钥 + Windows DPAPI，磁盘上仍是密文） */
  KEY_BUTLER_VAULT_ENABLE_SYSTEM: 'key_butler_vault_enable_system',
  /** 用主密码解锁已有密文库 */
  KEY_BUTLER_VAULT_UNLOCK: 'key_butler_vault_unlock',
  /** 关闭加密保存（清空本地密文） */
  KEY_BUTLER_VAULT_DISABLE: 'key_butler_vault_disable',
  /** 查询加密保存状态 */
  KEY_BUTLER_VAULT_STATUS: 'key_butler_vault_status',
  /** 查询某 Key 的余额快照序列 */
  KEY_BUTLER_SNAPSHOTS: 'key_butler_snapshots',
} as const

/** 命令名联合类型 */
export type CommandName = (typeof COMMAND_NAMES)[keyof typeof COMMAND_NAMES]

// ---- Key 管家（Beta）契约 ----
// 与 `src-tauri/src/key_butler/models.rs` 逐字段一一对应（camelCase）。
// 契约三处同步约定同样适用于本模块：改字段必须同改
// `key_butler/models.rs` + 本文件 + `docs/CONTRACTS.md`。
//
// 安全约定：`KeyEntry` 不含 Key 明文，只有 `maskedKey`（格式
// `sk-ab***cdef`）。完整明文只在 `key_butler_reveal` 的返回值里出现一次，
// 仅供一键复制，不落日志、不落事件。

/**
 * 一条 Key 的四态状态。
 * 与 Rust 端 `key_butler::models::KeyStatus` 一一对应。
 *
 * - `active`       : Key 有效且余额充足
 * - `insufficient` : Key 有效但余额 ≤ 阈值
 * - `invalid`      : 401 / 403，Key 已失效
 * - `error`        : 网络 / 429 / 5xx / 解析失败，有效性未知
 *
 * 注意：未检测过的 Key 也用 `error` 承载，前端以 `lastCheckedAt == null`
 * 区分并显示「未检测」，且不计入四态汇总。
 */
export type KeyStatus = 'active' | 'insufficient' | 'invalid' | 'error'

/**
 * 余额接口的适配器类型。
 * 与 Rust 端 `key_butler::models::BalanceEndpointKind` 一一对应。
 *
 * - `builtin-oneapi`   : one-api / new-api 系中转站（billing/subscription + usage）
 * - `builtin-deepseek` : DeepSeek 官方 `GET {host}/user/balance`
 * - `none`             : 该平台没有公开余额接口（官方 OpenAI / Anthropic）
 * - `custom`           : 用户自定义
 */
export type BalanceEndpointKind =
  | 'builtin-oneapi'
  | 'builtin-deepseek'
  | 'none'
  | 'custom'

/** 自定义余额接口的 HTTP 方法。与 Rust 端 `HttpMethod` 一一对应。 */
export type HttpMethod = 'GET' | 'POST'

/**
 * 批量检测中单条 Key 所处的阶段。
 * 与 Rust 端 `key_butler::models::CheckStage` 一一对应。
 */
export type CheckStage = 'pending' | 'checking' | 'balance' | 'done'

/**
 * 自定义余额接口配置（`balanceEndpoint === 'custom'` 时生效）。
 * 对应 Rust 结构体：`key_butler::models::BalanceEndpoint`
 */
export interface BalanceEndpoint {
  /** URL 模板，支持 `{base}` 占位（`{base}` = 该 Key 的 Base URL） */
  urlTemplate: string
  /** HTTP 方法 */
  method: HttpMethod
  /** 额外请求头 */
  extraHeaders: Record<string, string>
  /** 总额度取值路径（JSONPath 式，如 `hard_limit_usd`） */
  totalPath: string | null
  /** 已用额度取值路径（如 `total_usage`） */
  usedPath: string | null
  /** 直接余额取值路径（服务端直接返回余额字段时使用） */
  balancePath: string | null
  /** 单位除数：默认 1；one-api 的 `total_usage` 以美分计时为 100 */
  unitDivisor: number
}

/**
 * 一条被管理的 Key（不含明文）。
 * 对应 Rust 结构体：`key_butler::models::KeyEntry`
 */
export interface KeyEntry {
  /** 唯一 id（uuid v4） */
  id: string
  /** 脱敏展示串，格式 `sk-ab***cdef`（前 5 后 4） */
  maskedKey: string
  /** 平台标识：anthropic / openai / groq / google / xai / huggingface / replicate / unknown */
  platform: string
  /** 用户备注 */
  note: string
  /** 导入时间（Unix 毫秒） */
  createdAt: number
  /** 最近一次检测完成时间（Unix 毫秒）；`null` = 从未检测 */
  lastCheckedAt: number | null
  /** 四态状态 */
  status: KeyStatus
  /** Key 是否有效；`null` = 未能判定。`status==='error' && keyValid===true` 表示「Key 有效但余额查询失败」 */
  keyValid: boolean | null
  /** 余额数值；`null` = 未知 */
  balance: number | null
  /** 余额是否可信 */
  balanceKnown: boolean
  /** 币种（如 USD / CNY）；取不到为 `null` */
  currency: string | null
  /** 错误分类（复用探测模块的 9 类字符串值）；无错误为 `null` */
  errorCategory: string | null
  /** 该 Key 使用的余额接口适配器 */
  balanceEndpoint: BalanceEndpointKind
  /** 自定义余额接口配置（`balanceEndpoint === 'custom'` 时生效） */
  customEndpoint: BalanceEndpoint | null
  /** 该 Key 的 Base URL（有效性检测与余额查询都需要）；`null` = 未设置 */
  baseUrl: string | null
}

/**
 * 一条余额快照。不含 Key 明文，可明文落盘。
 * 对应 Rust 结构体：`key_butler::models::Snapshot`
 */
export interface Snapshot {
  /** 关联的 Key id */
  keyId: string
  /** 快照时间（Unix 毫秒） */
  ts: number
  /** 当时的余额；`null` = 未知 */
  balance: number | null
  /** 当时的状态字符串（`KeyStatus` 的序列化值） */
  status: string
}

/**
 * 批量导入时的平台判定方式。
 * 与 Rust 端 `key_butler::models::KeyImportMode` 一一对应。
 *
 * - `auto`   : 按 Key 形态自动识别平台（默认）
 * - `manual` : 整批统一指定厂商（`KeyImportRequest.platform` 必填）
 */
export type KeyImportMode = 'auto' | 'manual'

/**
 * `key_butler_import` 的入参。
 * 对应 Rust 结构体：`key_butler::models::KeyImportRequest`
 */
export interface KeyImportRequest {
  /** 待导入的整段文本（提取规则在后端，前端不参与切分） */
  text: string
  /** 平台判定方式；不传 = `auto` */
  mode?: KeyImportMode
  /** 手动模式下的平台标识（如 `deepseek`） */
  platform?: string | null
  /** 手动模式下的 Base URL 覆盖；空 = 用该平台默认值 */
  baseUrl?: string | null
  /** 手动模式下的余额适配器覆盖；不传 = 用该平台默认值 */
  balanceEndpoint?: BalanceEndpointKind | null
}

/**
 * `key_butler_import` 的返回。
 * 对应 Rust 结构体：`key_butler::models::KeyImportResult`
 */
export interface KeyImportResult {
  /** 导入后的全量脱敏列表 */
  entries: KeyEntry[]
  /** 本次新增条数 */
  added: number
  /** 因重复被跳过的条数 */
  duplicates: number
  /** 文本里没能识别成 Key 的行数 */
  ignored: number
}

/**
 * `key_butler_check` 的入参。
 * 对应 Rust 结构体：`key_butler::models::KeyCheckRequest`
 */
export interface KeyCheckRequest {
  /** 要检测的 Key id；空数组 = 全部 */
  ids: string[]
  /** 并发数（1-10，后端 clamp） */
  concurrency?: number | null
  /** 单请求超时（秒，5-300，后端 clamp） */
  timeoutSecs?: number | null
  /** 余额不足阈值：余额 ≤ 该值判为 `insufficient` */
  lowBalanceThreshold?: number | null
}

/**
 * `key_butler_check` 的返回：整批检测汇总。
 * 对应 Rust 结构体：`key_butler::models::KeyCheckSummary`
 */
export interface KeyCheckSummary {
  /** 本次运行 id（Unix 毫秒字符串） */
  runId: string
  /** 实际检测条数 */
  checked: number
  /** 四态计数 */
  active: number
  insufficient: number
  invalid: number
  error: number
  /** 总耗时（毫秒） */
  elapsedMs: number
}

/**
 * 本地保存（vault）状态。
 * 对应 Rust 结构体：`key_butler::models::VaultStatus`
 */
export interface VaultStatus {
  /** 磁盘上是否已存在密文库（此前启用过） */
  enabled: boolean
  /** 当前会话是否已解锁 */
  unlocked: boolean
  /** 当前内存中的 Key 条数 */
  entryCount: number
  /**
   * 是否使用「无密码保存」：数据密钥由 Windows DPAPI 绑定当前账户，启动时自动解开。
   * 磁盘上依然是 AES-256-GCM 密文，不是明文。
   *
   * `enabled && !unlocked && systemKey` 表示「这份数据属于别的 Windows 账户
   * 或另一台电脑」，需要与「等待输入主密码」区分开。
   */
  systemKey: boolean
}

/**
 * `key_butler_set_config` 的入参。
 * 对应 Rust 结构体：`key_butler::models::KeyConfigPatch`
 */
export interface KeyConfigPatch {
  /** 新的 Base URL；`undefined` 表示不改，`null` 表示清空 */
  baseUrl?: string | null
  /** 新的余额接口适配器 */
  balanceEndpoint?: BalanceEndpointKind
  /** 新的自定义余额接口配置 */
  customEndpoint?: BalanceEndpoint | null
}

/**
 * `key_butler_update_many` 的入参：一次改多条 Key。
 * 对应 Rust 结构体：`key_butler::models::KeyBatchPatch`
 *
 * 语义与 `KeyConfigPatch` 一致：`undefined` 表示该项不动。
 */
export interface KeyBatchPatch {
  /** 新的平台标识 */
  platform?: string | null
  /** 新的 Base URL；空字符串表示清空 */
  baseUrl?: string | null
  /** 新的余额接口适配器 */
  balanceEndpoint?: BalanceEndpointKind | null
  /** 新的备注 */
  note?: string | null
  /**
   * 改平台时是否同时套用该平台默认的 Base URL 与余额适配器。
   * 默认 `false`（只改标签）；显式的 `baseUrl` / `balanceEndpoint` 始终优先。
   */
  applyPlatformDefaults?: boolean
}

/**
 * 事件 `key-butler://progress` 的 payload。
 * 对应 Rust 结构体：`key_butler::models::KeyCheckProgress`
 */
export interface KeyCheckProgress {
  runId: string
  keyId: string
  maskedKey: string
  stage: CheckStage
  stageLabel: string
  completed: number
  total: number
  percent: number
}

/**
 * 事件 `key-butler://entry-done` 的 payload。
 * 对应 Rust 结构体：`key_butler::models::KeyCheckEntryDone`
 */
export interface KeyCheckEntryDone {
  runId: string
  entry: KeyEntry
  completed: number
  total: number
}

/**
 * 事件 `key-butler://finished` 的 payload。
 * 对应 Rust 结构体：`key_butler::models::KeyCheckFinished`
 */
export interface KeyCheckFinished {
  runId: string
  status: string
  checked: number
  total: number
  elapsedMs: number
  summary: KeyCheckSummary
}
