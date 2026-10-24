/**
 * 探测流程组合式函数：把「UI 事件 → 命令调用 → 事件订阅 → store 更新」
 * 这条链路收拢到一处，组件只负责调用 `start()` / `stop()` / `ping()` 并渲染状态。
 *
 * 生命周期：事件订阅在组件挂载时建立、卸载时释放；
 * 本函数暴露 `teardown()`，由 App.vue 在 onUnmounted 中调用。
 */

import { onUnmounted, ref } from 'vue'
import { createDiscreteApi, darkTheme } from 'naive-ui'

import {
  getProbeState,
  listModels,
  pingConnection,
  startProbe,
  stopProbe,
  ProbeApiError,
} from '@/api/commands'
import { subscribeAll } from '@/api/events'
import { useProbeStore } from '@/stores/probe'
import type { ModelBrief, PingResult } from '@/types/contracts'

// ---- 组件外可用的消息提示实例 ----

/**
 * 使用 createDiscreteApi 而不是 useMessage 的原因：
 * `useMessage()` 只能在 NMessageProvider 的组件树内部、setup 期间调用；
 * 而 useProbe() 可在任意时机被调用，且需要脱离组件上下文
 * （事件回调、异步流程）弹出提示。
 *
 * `createDiscreteApi(['message'])` 创建脱离组件树的独立渲染上下文，
 * 是官方为组件外调用 message 提供的方案。代价是不受 NConfigProvider
 * 主题影响，因此构造时显式传入与 App.vue 一致的暗色主题保证视觉统一。
 */
const { message } = createDiscreteApi(['message'], {
  configProviderProps: {
    theme: darkTheme,
  },
})

// ---- 错误分类的中文标签与颜色（供 UI 复用） ----

/** 错误分类 → 中文标签。与 Rust 的 `ProbeErrorKind::label()` 保持一致。 */
export const ERROR_KIND_LABELS: Record<string, string> = {
  keyInvalid: 'Key 无效',
  baseUrlInvalid: 'Base URL 错误',
  networkTimeout: '网络问题',
  rateLimited: '触发限流',
  serverError: '服务端故障',
  modelNotFound: '模型不可用',
  parseError: '响应解析失败',
  cancelled: '用户中止',
  unknown: '未知错误',
}

/** 错误分类 → Naive UI 标签颜色类型。 */
export const ERROR_KIND_TYPES: Record<string, 'error' | 'warning' | 'info'> = {
  keyInvalid: 'error',
  baseUrlInvalid: 'error',
  networkTimeout: 'warning',
  rateLimited: 'warning',
  serverError: 'error',
  modelNotFound: 'info',
  parseError: 'error',
  cancelled: 'info',
  unknown: 'warning',
}

/** 三态结果 → 中文标签 */
export const TRI_STATE_LABELS: Record<string, string> = {
  supported: '支持',
  unsupported: '不支持',
  unknown: '未知',
}

/** 上限来源 → 中文标签（在表格里以小徽标展示可信度） */
export const LIMIT_SOURCE_LABELS: Record<string, string> = {
  declared: '接口声明',
  dictionary: '字典',
  errorParse: '报错解析',
  fallback: '手动/兜底',
  unknown: '未知',
}

// ---- 主组合式函数 ----

export function useProbe() {
  const store = useProbeStore()

  /** 预检结果（由 ping() 填充） */
  const pingResult = ref<PingResult | null>(null)
  /** 预检进行中 */
  const pinging = ref(false)
  /** 已发现的模型清单（由 previewModels() 填充） */
  const discovered = ref<ModelBrief[]>([])
  /** 清单拉取进行中 */
  const loadingList = ref(false)
  /** 中止请求已发出但尚未收到 finished 事件 */
  const stopping = ref(false)

  /** 取消事件订阅的函数；由 setupSubscriptions 赋值 */
  let teardown: (() => Promise<void>) | null = null

  /* ------------------------- 事件订阅 ------------------------- */

  /**
   * 建立全部事件订阅。应在组件挂载时调用一次。
   */
  async function setupSubscriptions(): Promise<void> {
    // 避免重复订阅
    if (teardown) return

    teardown = await subscribeAll({
      onProgress: (payload) => store.applyProgress(payload),
      onLog: (payload) => store.applyLog(payload),
      onModelDone: (payload) => store.applyModelDone(payload),
      onFinished: (payload) => {
        store.applyFinished(payload)
        stopping.value = false
        if (payload.status === 'done') {
          message.success(
            `探测完成：${payload.completed}/${payload.total} 个模型，耗时 ${formatDuration(payload.elapsedMs)}`,
          )
        } else if (payload.status === 'cancelled') {
          message.warning(`探测已中止：已完成 ${payload.completed}/${payload.total}`)
        }
      },
      onError: (payload) => {
        store.applyFatal(payload.error)
        stopping.value = false
      },
    })
  }

  /** 释放事件订阅。App.vue 会在 onUnmounted 中调用。 */
  async function teardownSubscriptions(): Promise<void> {
    if (teardown) {
      await teardown()
      teardown = null
    }
  }

  // 组件卸载时自动清理，杜绝监听器泄漏
  onUnmounted(() => {
    void teardownSubscriptions()
  })

  /* ------------------------- 预检 ------------------------- */

  /**
   * 连通性预检。
   *
   * 此处推荐将其作为「第一步」：在花时间跑完整探测前，
   * 先确认 Key 与 Base URL 是否正确。
   */
  async function ping(): Promise<void> {
    if (configInvalid()) return
    pinging.value = true
    pingResult.value = null
    try {
      // 直接传整个配置：密钥来源（明文 / 环境变量 / 凭据管理器）
      // 由后端统一解析，保证「测试连接」与「开始探测」走同一条取密钥路径。
      const result = await pingConnection(store.config)
      pingResult.value = result
      if (result.ok) {
        message.success(`连接正常，发现 ${result.modelCount} 个模型。`)
        await previewModels()
      } else if (result.error) {
        message.error(result.error.message)
      }
    } catch (err) {
      const msgText = err instanceof ProbeApiError ? err.message : String(err)
      message.error(msgText)
    } finally {
      pinging.value = false
    }
  }

  /* ------------------------- 拉取模型清单 ------------------------- */

  /** 拉取模型清单（用于让用户缩小探测范围）。 */
  async function previewModels(): Promise<void> {
    if (configInvalid()) return
    loadingList.value = true
    try {
      const result = await listModels(store.config)
      discovered.value = result.models
    } catch (err) {
      const msgText = err instanceof ProbeApiError ? err.message : String(err)
      message.error(msgText)
      discovered.value = []
    } finally {
      loadingList.value = false
    }
  }

  /* ------------------------- 启动探测 ------------------------- */

  /**
   * 启动完整探测。
   *
   * 不使用 `store.beginRun()` 预置状态：运行 ID 由后端生成，前端无法提前知道。
   * 先清空结果并把状态置为 running，然后让 `startProbe` 的返回与事件共同填充数据。
   */
  async function start(): Promise<void> {
    if (store.isRunning) {
      message.warning('已有探测任务在运行中。')
      return
    }
    if (configInvalid()) return

    if (store.config.onlyModels && store.config.onlyModels.length === 0) {
      // 空数组表示全量，这里显式删掉该字段避免后端误解
      delete store.config.onlyModels
    }

    // 重置 UI 状态（保留配置）
    store.resetRun()
    store.hasRun = true
    store.status = 'running'
    store.startedAt = Date.now()
    stopping.value = false

    try {
      const summary = await startProbe(store.config)
      // 回退覆盖：即使事件全部丢失，结果也是完整的
      store.applySummary(summary)

      if (summary.errors.length > 0) {
        const first = summary.errors[0]
        message.error(`探测结束但存在错误：${first.message}`)
      }
    } catch (err) {
      const apiErr = err instanceof ProbeApiError ? err : null
      const msgText = apiErr ? apiErr.message : String(err)

      store.status = 'failed'
      store.finishedAt = Date.now()
      store.globalErrors = apiErr
        ? [{ kind: apiErr.kind, message: apiErr.message, httpStatus: apiErr.httpStatus, stage: 'pending' }]
        : [
            {
              kind: 'unknown',
              message: msgText,
              httpStatus: 0,
              stage: 'pending',
            },
          ]
      message.error(msgText)
    }
  }

  /* ------------------------- 中止探测 ------------------------- */

  /**
   * 请求中止当前探测。
   *
   * 中止是协作式的：这里立即把 UI 切到 cancelling，
   * 真正的终态由后端的 `probe://finished` 事件确认。
   */
  async function stop(): Promise<void> {
    if (!store.isRunning) {
      message.info('当前没有正在运行的探测任务。')
      return
    }
    stopping.value = true
    try {
      const snapshot = await stopProbe()
      store.applySnapshot(snapshot)
      store.status = 'cancelling'
      message.info('已请求中止，正在等待在途请求结束…')
    } catch (err) {
      stopping.value = false
      const msgText = err instanceof ProbeApiError ? err.message : String(err)
      message.error(msgText)
    }
  }

  /* ------------------------- 状态重同步 ------------------------- */

  /**
   * 从后端拉取快照并覆盖本地状态。
   *
   * 场景：用户重新加载了前端（开发时按 F5），或事件因故丢失。
   */
  async function sync(): Promise<void> {
    try {
      const snapshot = await getProbeState()
      store.applySnapshot(snapshot)
    } catch (err) {
      const msgText = err instanceof ProbeApiError ? err.message : String(err)
      message.error(msgText)
    }
  }

  /* ------------------------- 内部工具 ------------------------- */

  /** 校验配置；不合法时弹出提示并返回 true。 */
  function configInvalid(): boolean {
    const cfg = store.config

    if (!cfg.baseUrl.trim()) {
      message.error('请填写 Base URL。')
      return true
    }

    // 密钥按来源校验：明文要求 Key 非空，另两种来源要求「名字」非空。
    switch (cfg.apiKeySource) {
      case 'env':
        if (!cfg.apiKeyEnvVar.trim()) {
          message.error('请填写 API Key 所在的环境变量名。')
          return true
        }
        break
      case 'credential':
        if (!cfg.apiKeyCredential.trim()) {
          message.error('请填写 Windows 凭据管理器中的凭据名。')
          return true
        }
        break
      default:
        if (!cfg.apiKey.trim()) {
          message.error('请填写 API Key。')
          return true
        }
    }

    return false
  }

  return {
    store,
    pingResult,
    pinging,
    discovered,
    loadingList,
    stopping,
    setupSubscriptions,
    teardownSubscriptions,
    ping,
    previewModels,
    start,
    stop,
    sync,
  }
}

// ---- 通用格式化工具（供多个组件复用） ----

/**
 * 把毫秒格式化成人类可读时长。
 *
 * 例：1234 → "1.2s"，65000 → "1分5秒"
 */
export function formatDuration(ms: number): string {
  if (!Number.isFinite(ms) || ms < 0) return '—'
  if (ms < 1000) return `${Math.round(ms)}ms`
  const seconds = ms / 1000
  if (seconds < 60) return `${seconds.toFixed(1)}s`
  const minutes = Math.floor(seconds / 60)
  const rest = Math.round(seconds - minutes * 60)
  return `${minutes}分${rest}秒`
}

/**
 * 把 token 数格式化成紧凑写法。
 *
 * 例：128000 → "128K"，1048576 → "1M"，0 → "未知"
 */
export function formatTokens(value: number): string {
  if (!value || value <= 0) return '未知'
  if (value >= 1_000_000) {
    const millions = value / 1_000_000
    // 整数则不带小数位，否则保留一位
    return Number.isInteger(millions) ? `${millions}M` : `${millions.toFixed(1)}M`
  }
  if (value >= 1000) {
    const thousands = value / 1000
    return Number.isInteger(thousands) ? `${thousands}K` : `${thousands.toFixed(1)}K`
  }
  return String(value)
}
