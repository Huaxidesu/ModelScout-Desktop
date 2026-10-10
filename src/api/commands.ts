/**
 * ============================================================================
 * ModelScout —— 命令调用层（前端唯一 invoke 出口）
 * ============================================================================
 *
 * 【为什么单独建这一层】
 *   1. 阶段 5 QA 要求「逐一核对前端 invoke 命令名 == Rust 注册名」。
 *      把所有调用集中在此，命令名字符串只来自 `COMMAND_NAMES` 常量。
 *   2. 统一错误处理：Rust 端返回的 `CommandError` 是一个对象
 *      （{ kind, message, httpStatus }），不是字符串。
 *      直接 `catch (e) { alert(e) }` 会显示 `[object Object]`。
 *      这里统一把它规约成 `ProbeApiError` 实例。
 *
 * 【Tauri v2 要点（任务书第七节第 1 条）】
 *   invoke 必须从 `@tauri-apps/api/core` 导入，
 *   **不是** v1 的 `@tauri-apps/api/tauri`。
 * ============================================================================
 */

import { invoke } from '@tauri-apps/api/core'
import { COMMAND_NAMES } from '@/types/contracts'
import type {
  ModelListResult,
  PingResult,
  ProbeConfig,
  ProbeRunSummary,
  ProbeSnapshot,
  ProbeErrorKind,
  SaveConfigRequest,
  SaveConfigResult,
} from '@/types/contracts'

/* ---------------------------------------------------------------------------
 * 统一错误类型
 * ------------------------------------------------------------------------- */

/**
 * 前端侧统一的后端错误表示。
 *
 * Rust 端 `CommandError` 序列化后的形状是：
 *   { kind: 'keyInvalid', message: '...', httpStatus: 401 }
 *
 * 但 invoke 在某些情况下（例如命令名拼错、权限被拒）会抛出
 * 一个**纯字符串**。本类统一吸收这两种形态。
 */
export class ProbeApiError extends Error {
  /** 错误分类；无法归类时为 'unknown' */
  readonly kind: ProbeErrorKind
  /** HTTP 状态码，0 表示非 HTTP 层 */
  readonly httpStatus: number

  constructor(kind: ProbeErrorKind, message: string, httpStatus: number) {
    super(message)
    this.name = 'ProbeApiError'
    this.kind = kind
    this.httpStatus = httpStatus
  }

  /** 该错误是否属于「端点级」问题（整轮探测都会失败） */
  get isEndpointLevel(): boolean {
    return this.kind === 'keyInvalid' || this.kind === 'baseUrlInvalid'
  }
}

/**
 * 把 invoke 抛出的任意值规约成 `ProbeApiError`。
 *
 * 这是全前端唯一处理后端错误的地方，组件层只需 `instanceof` 判断即可。
 */
function normalizeError(raw: unknown): ProbeApiError {
  // 形态 1：Rust 的 CommandError 对象
  if (raw && typeof raw === 'object') {
    const obj = raw as Record<string, unknown>
    const kind = typeof obj.kind === 'string' ? (obj.kind as ProbeErrorKind) : 'unknown'
    const message =
      typeof obj.message === 'string' && obj.message.length > 0
        ? obj.message
        : '后端返回了一个未提供说明的错误。'
    const httpStatus =
      typeof obj.httpStatus === 'number'
        ? obj.httpStatus
        : typeof obj.http_status === 'number'
          ? (obj.http_status as number)
          : 0
    return new ProbeApiError(kind, message, httpStatus)
  }

  // 形态 2：纯字符串（命令名错误、权限拒绝、IPC 异常等）
  if (typeof raw === 'string') {
    return new ProbeApiError('unknown', raw, 0)
  }

  // 形态 3：Error 实例
  if (raw instanceof Error) {
    return new ProbeApiError('unknown', raw.message, 0)
  }

  // 形态 4：其他（回退分支）
  return new ProbeApiError('unknown', String(raw), 0)
}

/**
 * 统一包装 invoke：负责错误规约。
 *
 * 所有命令函数都必须经由它调用，不允许直接使用 invoke。
 */
async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(command, args)
  } catch (raw) {
    const error = normalizeError(raw)
    // 在控制台留痕，但**不**包含任何 Key（后端已保证 message 脱敏）
    console.error(`[ModelScout] 命令 ${command} 调用失败：`, error.message)
    throw error
  }
}

/* ---------------------------------------------------------------------------
 * 6 + 1 个命令的封装
 * ------------------------------------------------------------------------- */

/**
 * 【命令 1/7】拉取模型列表（不做能力探测）。
 *
 * 对应 Rust：`commands::list_models`
 * 前端传入的参数名按 Tauri v2 约定为 camelCase，
 * Rust 端的 `config: ProbeConfig` 参数会自动匹配。
 */
export function listModels(config: ProbeConfig): Promise<ModelListResult> {
  return call<ModelListResult>(COMMAND_NAMES.LIST_MODELS, { config })
}

/**
 * 【命令 2/7】连通性与鉴权预检。
 *
 * 对应 Rust：`commands::ping_connection`
 *
 * 注意：该命令**永不抛错**，失败信息在返回值的 `error` 字段里。
 * 因此这里不 try/catch 成异常，而是原样返回给调用方判断 `ok`。
 */
export function pingConnection(
  baseUrl: string,
  apiKey: string,
  timeoutSecs?: number,
): Promise<PingResult> {
  return call<PingResult>(COMMAND_NAMES.PING_CONNECTION, {
    baseUrl,
    apiKey,
    timeoutSecs: timeoutSecs ?? null,
  })
}

/**
 * 【命令 3/7】启动完整探测流水线。
 *
 * 对应 Rust：`commands::start_probe`
 *
 * 该调用会一直挂起到整轮探测结束；期间通过事件推送进度。
 */
export function startProbe(config: ProbeConfig): Promise<ProbeRunSummary> {
  return call<ProbeRunSummary>(COMMAND_NAMES.START_PROBE, { config })
}

/**
 * 【命令 4/7】请求中止当前探测。
 *
 * 对应 Rust：`commands::stop_probe`
 *
 * 中止是协作式的：返回后仍在等待网络的任务会随超时自然结束。
 */
export function stopProbe(): Promise<ProbeSnapshot> {
  return call<ProbeSnapshot>(COMMAND_NAMES.STOP_PROBE)
}

/**
 * 【命令 5/7】查询当前探测快照。
 *
 * 对应 Rust：`commands::get_probe_state`
 */
export function getProbeState(): Promise<ProbeSnapshot> {
  return call<ProbeSnapshot>(COMMAND_NAMES.GET_PROBE_STATE)
}

/**
 * 【命令 6/7】把生成的配置写入磁盘。
 *
 * 对应 Rust：`commands::save_config_file`
 */
export function saveConfigFile(request: SaveConfigRequest): Promise<SaveConfigResult> {
  return call<SaveConfigResult>(COMMAND_NAMES.SAVE_CONFIG_FILE, { request })
}

/**
 * 【辅助】读取内置字典条目数，用于「关于」区域展示数据新鲜度。
 *
 * 该命令未列入 COMMAND_NAMES 常量表（非任务书要求的核心命令），
 * 但同样必须与 lib.rs 的注册名一致。
 */
export function getDictionaryInfo(): Promise<{ entryCount: number }> {
  return call<{ entryCount: number }>('get_dictionary_info')
}
