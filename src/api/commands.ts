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
  KeyBatchPatch,
  KeyCheckRequest,
  KeyCheckSummary,
  KeyConfigPatch,
  KeyEntry,
  KeyImportRequest,
  KeyImportResult,
  ModelListResult,
  PingResult,
  ProbeConfig,
  ProbeRunSummary,
  ProbeSnapshot,
  ProbeErrorKind,
  SaveConfigRequest,
  SaveConfigResult,
  Snapshot,
  VaultStatus,
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
 *
 * 入参与 `start_probe` 一致，直接传整个 `ProbeConfig`：
 * 密钥来源（明文 / 环境变量 / 凭据管理器）由后端统一解析，
 * 预检也必须走同一条路径，否则「测试连接」能过、真正探测却拿不到 Key。
 */
export function pingConnection(config: ProbeConfig): Promise<PingResult> {
  return call<PingResult>(COMMAND_NAMES.PING_CONNECTION, { config })
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

/* ---------------------------------------------------------------------------
 * Key 管家（Beta）—— 13 个命令
 * ---------------------------------------------------------------------------
 * 与探测模块共用同一个 `call<T>()` 出口，因此错误同样是规约后的
 * `ProbeApiError`，组件层可以用同一套 `instanceof` 判断处理。
 *
 * 命名沿用文件里既有的 `动词+名词` 风格；命令名一律取自 `COMMAND_NAMES`，
 * 不允许在调用点写字面量。
 * ------------------------------------------------------------------------- */

/**
 * 批量文本 → 提取 Key → 前缀识别 → 入列（去重）。
 *
 * 提取在**后端**完成（与 `utils/parseConnection.ts` 同一套正则思路），
 * 前端只负责把整段文本原样传过去。
 */
export function keyButlerImport(request: KeyImportRequest): Promise<KeyImportResult> {
  return call<KeyImportResult>(COMMAND_NAMES.KEY_BUTLER_IMPORT, { request })
}

/**
 * 单/批量检测。
 *
 * 会挂起到整批检测结束；期间进度与逐条结果通过 3 个 `key-butler://*` 事件推送。
 */
export function keyButlerCheck(request: KeyCheckRequest): Promise<KeyCheckSummary> {
  return call<KeyCheckSummary>(COMMAND_NAMES.KEY_BUTLER_CHECK, { request })
}

/** 请求中止当前批量检测（协作式，返回中止那一刻的汇总）。 */
export function keyButlerCheckCancel(): Promise<KeyCheckSummary> {
  return call<KeyCheckSummary>(COMMAND_NAMES.KEY_BUTLER_CHECK_CANCEL)
}

/** 返回脱敏列表（`KeyEntry` 不含明文）。 */
export function keyButlerList(): Promise<KeyEntry[]> {
  return call<KeyEntry[]>(COMMAND_NAMES.KEY_BUTLER_LIST)
}

/**
 * 返回单个完整 Key —— **仅供一键复制**。
 *
 * 调用方拿到后应立即复制并把变量置空，不要存进 store 或写进日志。
 */
export function keyButlerReveal(id: string): Promise<string> {
  return call<string>(COMMAND_NAMES.KEY_BUTLER_REVEAL, { id })
}

/** 删除一条 Key（连同它的余额快照），返回删除后的全量列表。 */
export function keyButlerDelete(id: string): Promise<KeyEntry[]> {
  return call<KeyEntry[]>(COMMAND_NAMES.KEY_BUTLER_DELETE, { id })
}

/** 修改备注，返回更新后的全量列表。 */
export function keyButlerUpdateNote(id: string, note: string): Promise<KeyEntry[]> {
  return call<KeyEntry[]>(COMMAND_NAMES.KEY_BUTLER_UPDATE_NOTE, { id, note })
}

/** 修改 Base URL 与余额接口配置，返回更新后的全量列表。 */
export function keyButlerSetConfig(id: string, patch: KeyConfigPatch): Promise<KeyEntry[]> {
  return call<KeyEntry[]>(COMMAND_NAMES.KEY_BUTLER_SET_CONFIG, { id, patch })
}

/** 一次修改多条 Key（平台 / Base URL / 余额接口 / 备注），返回更新后的全量列表。 */
export function keyButlerUpdateMany(ids: string[], patch: KeyBatchPatch): Promise<KeyEntry[]> {
  return call<KeyEntry[]>(COMMAND_NAMES.KEY_BUTLER_UPDATE_MANY, { ids, patch })
}

/** 启用加密保存（设置主密码并加密当前列表）。 */
export function keyButlerVaultEnable(password: string): Promise<VaultStatus> {
  return call<VaultStatus>(COMMAND_NAMES.KEY_BUTLER_VAULT_ENABLE, { password })
}

/**
 * 启用本地保存（无密码模式）。
 *
 * 用户不需要输入任何东西，但磁盘上仍然是 AES-256-GCM 密文 ——
 * 「无密码」靠的是把随机数据密钥用 Windows DPAPI 绑到当前账户上。
 */
export function keyButlerVaultEnableSystem(): Promise<VaultStatus> {
  return call<VaultStatus>(COMMAND_NAMES.KEY_BUTLER_VAULT_ENABLE_SYSTEM)
}

/** 用主密码解锁已有密文库。 */
export function keyButlerVaultUnlock(password: string): Promise<VaultStatus> {
  return call<VaultStatus>(COMMAND_NAMES.KEY_BUTLER_VAULT_UNLOCK, { password })
}

/** 关闭加密保存（删除本地密文，内存表保留）。 */
export function keyButlerVaultDisable(): Promise<VaultStatus> {
  return call<VaultStatus>(COMMAND_NAMES.KEY_BUTLER_VAULT_DISABLE)
}

/** 查询加密保存状态（是否已启用 / 当前是否已解锁）。 */
export function keyButlerVaultStatus(): Promise<VaultStatus> {
  return call<VaultStatus>(COMMAND_NAMES.KEY_BUTLER_VAULT_STATUS)
}

/** 查询某条 Key 的余额快照序列（按时间升序）。 */
export function keyButlerSnapshots(keyId: string): Promise<Snapshot[]> {
  return call<Snapshot[]>(COMMAND_NAMES.KEY_BUTLER_SNAPSHOTS, { keyId })
}
