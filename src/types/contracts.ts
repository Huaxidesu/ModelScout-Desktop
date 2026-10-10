/**
 * ============================================================================
 * ModelScout —— 阶段 1：接口契约（前后端对齐的唯一事实来源）
 * ============================================================================
 *
 * 【本文件地位】
 * 本文件是前端 TypeScript 侧对 Rust 后端数据结构的镜像定义。
 * 每一个 interface 都必须与 `src-tauri/src/models.rs` 中的 #[derive(Serialize)]
 * 结构体一一对应；字段名必须完全一致（Rust 侧统一使用
 * #[serde(rename_all = "camelCase")]，因此这里全部为小驼峰）。
 *
 * 【修改约定】
 * 任何字段增删都必须「三处同步」：
 *   1. src/types/contracts.ts  （本文件）
 *   2. src-tauri/src/models.rs （Rust 结构体）
 *   3. src/api/commands.ts     （若涉及命令签名）
 * 否则阶段 5 QA 的交叉核对会失败。
 * ============================================================================
 */

/* ---------------------------------------------------------------------------
 * 一、基础枚举（字符串字面量联合类型，与 Rust 的 #[serde(rename_all = "camelCase")]
 *     枚举序列化结果对应）
 * ------------------------------------------------------------------------- */

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
 * - `declared`    : **接口在 /models 中直接声明**，最可信（服务端自己说的）
 * - `dictionary`  : 命中本地参数字典，可信度高
 * - `errorParse`  : 通过超大 max_tokens 触发报错后解析得出
 * - `fallback`    : 使用保守兜底值
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

/* ---------------------------------------------------------------------------
 * 二、命令入参 / 出参结构体
 * ------------------------------------------------------------------------- */

/**
 * 传给 `start_probe` 命令的探测配置。
 * 对应 Rust 结构体：`models::ProbeConfig`
 *
 * 注意：`apiKey` 只在 invoke 时向前端 → Rust 单向传递，
 * Rust 端持有它但**绝不**在任何 log / 事件 payload 中回传或打印。
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
   * **为什么要展示**：实测发现部分网关的 `/models` 声明可能与官方文档不符
   * （例如接口称某模型支持 tools、文档称不支持）。把来源标出来，
   * 用户才能判断该采信谁，而不是盲信一个值。
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
 * 语义很关键：字段为 `null` = 接口**未声明**，需要真实请求探测；
 * 有值 = **直接采信声明**，跳过对应的探测请求（省额度、更快、更准）。
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
   * 是否为**推理模型**（`supported_parameters` 中存在 `reasoning`）。
   *
   * 注意：这比「响应里出现 reasoning_content」更可靠 —— 实测发现部分网关
   * 对**所有**模型都会返回 `reasoning_content` 字段（可能为空），
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
 * ⚠️ 安全约束：`fileName` 不得包含路径分隔符（`/` `\`）或 `..`，
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

/* ---------------------------------------------------------------------------
 * 三、事件 payload 结构体
 * ------------------------------------------------------------------------- */
/**
 * 事件 `probe://progress` 的 payload。
 * 对应 Rust 结构体：`models::ProbeProgress`
 *
 * 说明：Rust 端采用**事件名与 payload 一同注入**的设计，
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

/* ---------------------------------------------------------------------------
 * 四、事件名常量（唯一集中定义，Rust 端 emit 的字符串必须与此完全一致）
 * ------------------------------------------------------------------------- */

/**
 * 全部 Tauri 事件名常量。
 * 阶段 5 QA 会逐一核对本表与 `src-tauri/src/probe.rs` 中的 emit 字面量。
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
} as const

/** 事件名联合类型 */
export type EventName = (typeof EVENT_NAMES)[keyof typeof EVENT_NAMES]

/* ---------------------------------------------------------------------------
 * 五、命令名常量（唯一集中定义，Rust 端 generate_handler 注册名必须与此一致）
 * ------------------------------------------------------------------------- */

/**
 * 全部 Tauri 命令名常量。
 * 阶段 5 QA 会逐一核对本表与 `src-tauri/src/lib.rs` 中
 * `generate_handler![]` 的注册名。
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
} as const

/** 命令名联合类型 */
export type CommandName = (typeof COMMAND_NAMES)[keyof typeof COMMAND_NAMES]
