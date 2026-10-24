/**
 * 事件订阅层：前端唯一的事件出口。
 *
 * 所有 listen 集中在此，事件名字符串只来自 `EVENT_NAMES` 常量：
 *   - 组件中不出现硬编码的事件名字面量；
 *   - 核对时只需比对 contracts.ts 的 EVENT_NAMES 与 events.rs 的常量。
 *
 * Tauri v2 的 listen 从 `@tauri-apps/api/event` 导入（与 v1 的 API 签名不同）；
 * 每个 listen 返回 Promise<UnlistenFn>，组件卸载时必须调用，否则造成监听器泄漏。
 */

import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { EVENT_NAMES } from '@/types/contracts'
import type {
  KeyCheckEntryDone,
  KeyCheckFinished,
  KeyCheckProgress,
  ProbeProgress,
  ProbeLog,
  ProbeModelDone,
  ProbeFinished,
  ProbeFatal,
} from '@/types/contracts'

/** 所有订阅函数统一返回这个类型，便于集中管理取消逻辑 */
export type Unlisten = UnlistenFn

/**
 * 订阅「进度更新」事件。
 *
 * @param handler 收到进度 payload 时调用
 * @returns 取消订阅函数（Promise 解析后得到）
 */
export function onProbeProgress(
  handler: (payload: ProbeProgress) => void,
): Promise<Unlisten> {
  return listen<ProbeProgress>(EVENT_NAMES.PROGRESS, (event) => {
    handler(event.payload)
  })
}

/**
 * 订阅「日志」事件。
 */
export function onProbeLog(handler: (payload: ProbeLog) => void): Promise<Unlisten> {
  return listen<ProbeLog>(EVENT_NAMES.LOG, (event) => {
    handler(event.payload)
  })
}

/**
 * 订阅「单模型完成」事件。
 */
export function onProbeModelDone(
  handler: (payload: ProbeModelDone) => void,
): Promise<Unlisten> {
  return listen<ProbeModelDone>(EVENT_NAMES.MODEL_DONE, (event) => {
    handler(event.payload)
  })
}

/**
 * 订阅「整轮结束」事件。
 */
export function onProbeFinished(
  handler: (payload: ProbeFinished) => void,
): Promise<Unlisten> {
  return listen<ProbeFinished>(EVENT_NAMES.FINISHED, (event) => {
    handler(event.payload)
  })
}

/**
 * 订阅「致命错误」事件。
 */
export function onProbeError(handler: (payload: ProbeFatal) => void): Promise<Unlisten> {
  return listen<ProbeFatal>(EVENT_NAMES.ERROR, (event) => {
    handler(event.payload)
  })
}

/**
 * 一次性订阅全部 5 个事件，返回统一的取消函数。
 *
 * 这是组件层推荐的用法：一行注册、一行清理，
 * 不会因为漏掉某个事件而造成内存泄漏。
 *
 * @param handlers 各事件的处理函数（可以只提供关心的那几个）
 * @returns 取消全部订阅的异步函数
 */
export async function subscribeAll(handlers: {
  onProgress?: (payload: ProbeProgress) => void
  onLog?: (payload: ProbeLog) => void
  onModelDone?: (payload: ProbeModelDone) => void
  onFinished?: (payload: ProbeFinished) => void
  onError?: (payload: ProbeFatal) => void
}): Promise<() => Promise<void>> {
  const unlisteners: Unlisten[] = []

  if (handlers.onProgress) {
    unlisteners.push(await onProbeProgress(handlers.onProgress))
  }
  if (handlers.onLog) {
    unlisteners.push(await onProbeLog(handlers.onLog))
  }
  if (handlers.onModelDone) {
    unlisteners.push(await onProbeModelDone(handlers.onModelDone))
  }
  if (handlers.onFinished) {
    unlisteners.push(await onProbeFinished(handlers.onFinished))
  }
  if (handlers.onError) {
    unlisteners.push(await onProbeError(handlers.onError))
  }

  // 返回一个幂等的取消函数
  return async () => {
    for (const unlisten of unlisteners) {
      try {
        unlisten()
      } catch {
        // 取消订阅失败不应影响卸载流程，静默忽略
      }
    }
    unlisteners.length = 0
  }
}

// ---- Key 管家（Beta）事件 ----
// 命名与用法与探测模块一致：事件名只来自 `EVENT_NAMES`，
// 组件卸载时必须调用返回的取消函数。

/** 订阅「批量检测进度」事件。 */
export function onKeyButlerProgress(
  handler: (payload: KeyCheckProgress) => void,
): Promise<Unlisten> {
  return listen<KeyCheckProgress>(EVENT_NAMES.KEY_BUTLER_PROGRESS, (event) => {
    handler(event.payload)
  })
}

/** 订阅「单条 Key 检测完成」事件。 */
export function onKeyButlerEntryDone(
  handler: (payload: KeyCheckEntryDone) => void,
): Promise<Unlisten> {
  return listen<KeyCheckEntryDone>(EVENT_NAMES.KEY_BUTLER_ENTRY_DONE, (event) => {
    handler(event.payload)
  })
}

/** 订阅「整批检测结束」事件。 */
export function onKeyButlerFinished(
  handler: (payload: KeyCheckFinished) => void,
): Promise<Unlisten> {
  return listen<KeyCheckFinished>(EVENT_NAMES.KEY_BUTLER_FINISHED, (event) => {
    handler(event.payload)
  })
}

/**
 * 一次性订阅 Key 管家的全部 3 个事件，返回统一的取消函数。
 *
 * 与探测模块的 `subscribeAll` 同一形态：一行注册、一行清理。
 */
export async function subscribeKeyButler(handlers: {
  onProgress?: (payload: KeyCheckProgress) => void
  onEntryDone?: (payload: KeyCheckEntryDone) => void
  onFinished?: (payload: KeyCheckFinished) => void
}): Promise<() => Promise<void>> {
  const unlisteners: Unlisten[] = []

  if (handlers.onProgress) {
    unlisteners.push(await onKeyButlerProgress(handlers.onProgress))
  }
  if (handlers.onEntryDone) {
    unlisteners.push(await onKeyButlerEntryDone(handlers.onEntryDone))
  }
  if (handlers.onFinished) {
    unlisteners.push(await onKeyButlerFinished(handlers.onFinished))
  }

  return async () => {
    for (const unlisten of unlisteners) {
      try {
        unlisten()
      } catch {
        // 取消订阅失败不应影响卸载流程，静默忽略
      }
    }
    unlisteners.length = 0
  }
}
