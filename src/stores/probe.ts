/**
 * ============================================================================
 * ModelScout —— 全局状态管理（Pinia）
 * ============================================================================
 *
 * 【职责】
 * 集中存放探测相关的全部可变状态，让组件只关心渲染：
 *   - 连接配置（含 API Key，**只在前端内存中存续，不落盘**）
 *   - 运行状态机（idle / running / cancelling / done / cancelled / failed）
 *   - 进度与日志缓冲
 *   - 探测结果模型列表与勾选状态
 *   - 错误分类汇总
 *
 * 【为什么用 Pinia 而不是简单的 reactive 单例】
 *   - 阶段 4 的配置生成器需要读探测结果，组件层级较深
 *   - 便于在阶段 5 做单元验证（store 可以脱离组件测试）
 * ============================================================================
 */

import { defineStore } from 'pinia'
import { computed, reactive, ref } from 'vue'

import type {
  ModelInfo,
  ProbeConfig,
  ProbeError,
  ProbeErrorKind,
  ProbeLog,
  ProbeModelDone,
  ProbeProgress,
  ProbeRunSummary,
  ProbeSnapshot,
  ProbeStage,
  ProbeStatus,
} from '@/types/contracts'

/* ---------------------------------------------------------------------------
 * 常量
 * ------------------------------------------------------------------------- */

/** 单条日志缓冲上限，防止长时间运行导致内存无限增长 */
const MAX_LOG_LINES = 600

/** 默认探测配置 */
export function defaultConfig(): ProbeConfig {
  return {
    baseUrl: 'https://api.openai.com/v1',
    apiKey: '',
    concurrency: 3, // 任务书第五节要求：默认并发 3
    timeoutSecs: 30, // 任务书第五节要求：30s 超时
    probeLimits: true,
    onlyModels: [],
  }
}

/* ---------------------------------------------------------------------------
 * Store 定义
 * ------------------------------------------------------------------------- */

export const useProbeStore = defineStore('probe', () => {
  /* ---------------------------- 配置 ---------------------------- */

  /**
   * 探测配置。
   *
   * ⚠️ 安全说明：`apiKey` 只存在于渲染进程内存中，不写 localStorage、
   * 不写文件。窗口关闭即丢失，这是刻意的设计（决策日志 D-09）。
   */
  const config = reactive<ProbeConfig>(defaultConfig())

  /* ---------------------------- 运行状态 ---------------------------- */

  /** 当前状态机状态 */
  const status = ref<ProbeStatus>('idle')
  /** 当前运行 ID（用于丢弃过期事件） */
  const runId = ref('')
  /** 是否已启动过至少一轮探测（用于决定是否显示结果区） */
  const hasRun = ref(false)

  /* ---------------------------- 进度 ---------------------------- */

  /** 总模型数 */
  const total = ref(0)
  /** 已完成模型数 */
  const completed = ref(0)
  /** 整体百分比 */
  const percent = ref(0)
  /** 每个模型当前所处阶段，key 是模型 ID */
  const stageByModel = reactive<Record<string, ProbeStage>>({})

  /* ---------------------------- 结果 ---------------------------- */

  /** 探测完成的模型列表（表驱动 UI 的数据源） */
  const models = ref<ModelInfo[]>([])
  /** 被用户勾选的模型 ID 集合 */
  const selectedIds = ref<string[]>([])

  /* ---------------------------- 日志与错误 ---------------------------- */

  /** 日志缓冲（最新的在最后） */
  const logs = ref<ProbeLog[]>([])
  /** 全局错误（来自 finished 事件或致命错误事件） */
  const globalErrors = ref<ProbeError[]>([])
  /** 探测起止时间戳（毫秒） */
  const startedAt = ref(0)
  const finishedAt = ref(0)

  /* ---------------------------- 计算属性 ---------------------------- */

  /** 是否正在运行（含正在收敛的中止状态） */
  const isRunning = computed(
    () => status.value === 'running' || status.value === 'cancelling',
  )

  /** 探测耗时（毫秒）。运行中按当前时间算，结束后固定。 */
  const elapsedMs = computed(() => {
    if (startedAt.value === 0) return 0
    const end = finishedAt.value > 0 ? finishedAt.value : Date.now()
    return end - startedAt.value
  })

  /** 被勾选的模型对象列表 */
  const selectedModels = computed(() =>
    models.value.filter((m) => selectedIds.value.includes(m.id)),
  )

  /**
   * 全部错误（含每个模型内部的错误），按分类汇总计数。
   * 用于「错误分类」汇总卡片。
   */
  const errorSummary = computed(() => {
    const counter = new Map<ProbeErrorKind, number>()

    const bump = (kind: ProbeErrorKind) => {
      counter.set(kind, (counter.get(kind) ?? 0) + 1)
    }

    for (const err of globalErrors.value) {
      // 用户主动中止不算「错误」，不纳入统计
      if (err.kind !== 'cancelled') bump(err.kind)
    }
    for (const model of models.value) {
      for (const err of model.errors) {
        if (err.kind !== 'cancelled') bump(err.kind)
      }
    }

    return Array.from(counter.entries())
      .map(([kind, count]) => ({ kind, count }))
      .sort((a, b) => b.count - a.count)
  })

  /** 能力统计：支持工具调用 / 支持视觉 / 推理模型的数量 */
  const capabilityStats = computed(() => {
    let tools = 0
    let vision = 0
    let reasoning = 0
    for (const model of models.value) {
      if (model.supportsTools === 'supported') tools += 1
      if (model.supportsVision === 'supported') vision += 1
      if (model.isReasoning) reasoning += 1
    }
    return { tools, vision, reasoning, total: models.value.length }
  })

  /* ---------------------------- 动作 ---------------------------- */

  /** 重置为一次全新的运行。保留配置，清空所有结果。 */
  function resetRun(): void {
    status.value = 'idle'
    runId.value = ''
    total.value = 0
    completed.value = 0
    percent.value = 0
    models.value = []
    selectedIds.value = []
    logs.value = []
    globalErrors.value = []
    startedAt.value = 0
    finishedAt.value = 0
    for (const key of Object.keys(stageByModel)) {
      delete stageByModel[key]
    }
  }

  /** 开始一次新运行：重置结果并置为 running。 */
  function beginRun(newRunId: string, totalModels: number): void {
    resetRun()
    hasRun.value = true
    runId.value = newRunId
    total.value = totalModels
    status.value = 'running'
    startedAt.value = Date.now()
  }

  /**
   * 应用一条进度事件。
   *
   * `runId` 不匹配的事件会被丢弃 —— 这是处理「用户中止后立刻重开」
   * 场景的关键：旧运行的迟到事件不应污染新运行的进度。
   */
  function applyProgress(payload: ProbeProgress): void {
    if (payload.runId && payload.runId !== currentRunIdOrAccept(payload.runId)) {
      return
    }
    stageByModel[payload.modelId] = payload.stage
    completed.value = payload.completed
    total.value = payload.total
    percent.value = payload.percent
  }

  /**
   * 内部使用：若当前 runId 为空（例如 startProbe 的返回还没到），
   * 则接受第一个到的事件并把它作为当前运行 ID。
   * 这解决了「事件比 invoke 返回先到」的竞态。
   */
  function currentRunIdOrAccept(incoming: string): string {
    if (!runId.value) {
      runId.value = incoming
    }
    return runId.value
  }

  /** 应用一条日志事件。 */
  function applyLog(payload: ProbeLog): void {
    if (runId.value && payload.runId && payload.runId !== runId.value) {
      return
    }
    logs.value.push(payload)
    if (logs.value.length > MAX_LOG_LINES) {
      // 保留最新的 MAX_LOG_LINES 条
      logs.value.splice(0, logs.value.length - MAX_LOG_LINES)
    }
  }

  /** 应用一条「单模型完成」事件。 */
  function applyModelDone(payload: ProbeModelDone): void {
    if (runId.value && payload.runId && payload.runId !== runId.value) {
      return
    }
    const index = models.value.findIndex((m) => m.id === payload.model.id)
    if (index >= 0) {
      models.value[index] = payload.model
    } else {
      models.value.push(payload.model)
      // 新完成的模型默认被勾选：绝大多数用户希望「全部生成」
      if (!selectedIds.value.includes(payload.model.id)) {
        selectedIds.value.push(payload.model.id)
      }
    }
    completed.value = payload.completed
    total.value = payload.total
  }

  /** 应用一次「整轮结束」（含被中止的情况）。 */
  function applyFinished(payload: {
    status: ProbeStatus
    completed: number
    total: number
    elapsedMs: number
    errors: ProbeError[]
  }): void {
    status.value = payload.status
    completed.value = payload.completed
    total.value = payload.total
    globalErrors.value = payload.errors
    finishedAt.value = Date.now()
    percent.value = payload.status === 'done' ? 100 : percent.value
  }

  /**
   * 用一次完整运行的结果覆盖 store。
   *
   * 这是**事件丢失时的兜底路径**：`startProbe` 的返回值里带着完整结果，
   * 即使用户在探测过程中关闭又打开了 DevTools 导致事件丢失，
   * 最终也能拿到正确数据。
   */
  function applySummary(summary: ProbeRunSummary): void {
    runId.value = summary.runId || runId.value
    status.value = summary.status

    // 防御：后端返回**空数组**、而事件路径已经累积出结果时，不要用空数组覆盖。
    //
    // 为什么需要：曾经出现过后端漏收集结果、返回 `models: []` + `status: 'failed'`
    // 的 bug（见 D-37），而 `probe://model-done` 事件其实是对的 ——
    // 此时盲目覆盖会把好不容易拿到的数据清空，界面显示「0/0 暂无探测结果」。
    // 反之若事件丢了、后端有数据，这个分支同样会用后端的正确值覆盖。
    if (summary.models.length > 0 || models.value.length === 0) {
      models.value = summary.models
      total.value = summary.models.length
      completed.value = summary.models.length
    }
    globalErrors.value = summary.errors
    if (summary.status === 'done') {
      percent.value = 100
    }
    if (startedAt.value === 0) {
      startedAt.value = Date.now() - summary.elapsedMs
    }
    finishedAt.value = Date.now()

    // 默认全选尚未被选中的新模型
    for (const model of summary.models) {
      if (!selectedIds.value.includes(model.id)) {
        selectedIds.value.push(model.id)
      }
    }

    pruneSelection()
  }

  /**
   * 丢弃指向「已不存在模型」的勾选 id。
   *
   * 为什么需要：`selectedIds` 是独立于 `models` 的数组。若只往里加、从不清理，
   * 就会累积**幽灵 id**。而界面上「生成配置（N）」「已选 N / M」用的都是
   * `selectedIds.length` —— 于是显示的数字会比实际可生成的模型还多。
   *
   * 这正是 D-37 截图里那个自相矛盾细节的来源：结果表显示「0/0 暂无探测结果」，
   * 按钮却写着「生成配置 (10)」。
   */
  function pruneSelection(): void {
    const alive = new Set(models.value.map((m) => m.id))
    selectedIds.value = selectedIds.value.filter((id) => alive.has(id))
  }

  /** 用快照覆盖 store（用于重同步）。 */
  function applySnapshot(snapshot: ProbeSnapshot): void {
    if (snapshot.runId) {
      runId.value = snapshot.runId
    }
    status.value = snapshot.status
    total.value = snapshot.totalModels
    completed.value = snapshot.completedModels
    models.value = snapshot.models
    pruneSelection()
    if (snapshot.status === 'idle') {
      hasRun.value = snapshot.models.length > 0
    }
  }

  /** 记录一条致命错误（来自 probe://error 事件）。 */
  function applyFatal(err: ProbeError): void {
    globalErrors.value.push(err)
    status.value = 'failed'
    finishedAt.value = Date.now()
  }

  /* ---------------------------- 勾选管理 ---------------------------- */

  /** 切换单个模型的勾选状态。 */
  function toggleModel(id: string): void {
    const index = selectedIds.value.indexOf(id)
    if (index >= 0) {
      selectedIds.value.splice(index, 1)
    } else {
      selectedIds.value.push(id)
    }
  }

  /** 全选。 */
  function selectAll(): void {
    selectedIds.value = models.value.map((m) => m.id)
  }

  /** 全不选。 */
  function selectNone(): void {
    selectedIds.value = []
  }

  /** 反选。 */
  function invertSelection(): void {
    const current = new Set(selectedIds.value)
    selectedIds.value = models.value.map((m) => m.id).filter((id) => !current.has(id))
  }

  /**
   * 按条件批量选择：只选支持工具调用的 / 只选推理模型 / 只选支持视觉的。
   *
   * 这是很实用的筛选器 —— 例如接 OpenCode 时通常只想要支持工具调用的模型。
   */
  function selectByPredicate(predicate: (model: ModelInfo) => boolean): void {
    selectedIds.value = models.value.filter(predicate).map((m) => m.id)
  }

  /**
   * 手动纠正某个模型的字段。
   *
   * 【为什么需要】
   * 上限探测依赖服务端在报错里回显数值，实测中总有服务商不给。
   * 与其让用户去改 JSON，不如在表格里允许直接编辑 ——
   * 这是「诚实标注 + 可修正」的落地（决策日志 D-10）。
   */
  function patchModel(id: string, patch: Partial<ModelInfo>): void {
    const index = models.value.findIndex((m) => m.id === id)
    if (index < 0) return
    models.value[index] = { ...models.value[index], ...patch }
    // 手动修改的字段标记为 fallback 来源，让 UI 显示「手动」标签
    if (patch.contextLimit !== undefined) {
      models.value[index].contextLimitSource = 'fallback'
    }
    if (patch.maxOutputLimit !== undefined) {
      models.value[index].maxOutputLimitSource = 'fallback'
    }
  }

  return {
    // 配置
    config,
    // 状态
    status,
    runId,
    hasRun,
    total,
    completed,
    percent,
    stageByModel,
    models,
    selectedIds,
    logs,
    globalErrors,
    startedAt,
    finishedAt,
    // 计算属性
    isRunning,
    elapsedMs,
    selectedModels,
    errorSummary,
    capabilityStats,
    // 动作
    resetRun,
    beginRun,
    applyProgress,
    applyLog,
    applyModelDone,
    applyFinished,
    applySummary,
    applySnapshot,
    applyFatal,
    toggleModel,
    selectAll,
    selectNone,
    invertSelection,
    selectByPredicate,
    patchModel,
  }
})
