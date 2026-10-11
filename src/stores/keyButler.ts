/**
 * ============================================================================
 * ModelScout —— Key 管家（Beta）全局状态
 * ============================================================================
 *
 * 【职责】
 * 集中存放 Key 管家的全部可变状态：Key 列表、加密保存状态、检测进度、
 * 过滤条件。组件只负责渲染与调用 action。
 *
 * 【与探测模块 store 的关系】
 * 完全独立。`stores/probe.ts` 管探测，本文件管 Key 管家，
 * 两者不共享任何状态（模块隔离原则）。
 *
 * 【性能要点】
 * 批量检测会高频推送进度事件（每条 Key 两个阶段）。这里对进度做
 * 100ms 时间窗合并，避免每个事件都触发一次组件重渲染；
 * 逐条结果（`applyEntryDone`）则就地替换，不做整体重建。
 * ============================================================================
 */

import { defineStore } from 'pinia'
import { computed, ref } from 'vue'

import {
  keyButlerCheck,
  keyButlerCheckCancel,
  keyButlerDelete,
  keyButlerImport,
  keyButlerList,
  keyButlerReveal,
  keyButlerSetConfig,
  keyButlerSnapshots,
  keyButlerUpdateNote,
  keyButlerVaultDisable,
  keyButlerVaultEnable,
  keyButlerVaultStatus,
  keyButlerVaultUnlock,
} from '@/api/commands'
import type {
  KeyCheckEntryDone,
  KeyCheckFinished,
  KeyCheckProgress,
  KeyCheckSummary,
  KeyConfigPatch,
  KeyEntry,
  KeyStatus,
  Snapshot,
  VaultStatus,
} from '@/types/contracts'

/* ---------------------------------------------------------------------------
 * 常量与工具
 * ------------------------------------------------------------------------- */

/**
 * 平台标识 → 界面显示名。
 *
 * 未收录的平台直接显示原始标识 —— 前缀表是用户可扩充的，
 * 界面不该因为出现新平台就显示空白。
 */
export const PLATFORM_LABELS: Record<string, string> = {
  openai: 'OpenAI',
  anthropic: 'Anthropic',
  groq: 'Groq',
  google: 'Google',
  xai: 'xAI',
  huggingface: 'HuggingFace',
  replicate: 'Replicate',
  unknown: '通用 / 中转',
}

/** 平台显示名（回退为原始标识）。 */
export function platformLabel(platform: string): string {
  return PLATFORM_LABELS[platform] ?? platform
}

/** 状态 → 中文标签。 */
export const STATUS_LABELS: Record<KeyStatus, string> = {
  active: '可用',
  insufficient: '余额不足',
  invalid: '已失效',
  error: '查询失败',
}

/**
 * 相对时间：`2min前` / `1h前` / `3d前`。
 *
 * `null` 表示从未检测，返回「未检测」—— 这是界面上区分
 * 「新导入」与「查询失败」的唯一依据（两者 status 都是 error）。
 */
export function formatRelativeTime(ts: number | null): string {
  if (!ts) return '未检测'
  const diff = Date.now() - ts
  if (diff < 0) return '刚刚'
  const seconds = Math.floor(diff / 1000)
  if (seconds < 60) return '刚刚'
  const minutes = Math.floor(seconds / 60)
  if (minutes < 60) return `${minutes}min前`
  const hours = Math.floor(minutes / 60)
  if (hours < 24) return `${hours}h前`
  const days = Math.floor(hours / 24)
  return `${days}d前`
}

/** 余额展示：未知显示 `—`。 */
export function formatBalance(entry: KeyEntry): string {
  if (!entry.balanceKnown || entry.balance === null) return '—'
  const symbol = entry.currency === 'CNY' ? '¥' : '$'
  return `${symbol}${entry.balance.toFixed(2)}`
}

/* ---------------------------------------------------------------------------
 * Store
 * ------------------------------------------------------------------------- */

export const useKeyButlerStore = defineStore('keyButler', () => {
  /* ---------------------------- 状态 ---------------------------- */

  /** 脱敏 Key 列表（后端保证不含明文） */
  const entries = ref<KeyEntry[]>([])
  /** 加密保存状态 */
  const vault = ref<VaultStatus>({ enabled: false, unlocked: false, entryCount: 0 })
  /** 首次加载中 */
  const loading = ref(false)
  /** 批量检测进行中 */
  const checking = ref(false)
  /** 当前运行 id（用于丢弃过期事件） */
  const runId = ref('')
  /** 检测进度（已节流） */
  const progress = ref({
    completed: 0,
    total: 0,
    percent: 0,
    currentMaskedKey: '',
    stageLabel: '',
  })
  /** 最近一次整批汇总 */
  const lastSummary = ref<KeyCheckSummary | null>(null)

  /* ---------------------------- 过滤条件 ---------------------------- */

  const keyword = ref('')
  const filterPlatform = ref<string | null>(null)
  const filterStatus = ref<KeyStatus[]>([])
  /** 余额不足阈值：余额 ≤ 该值判为不足 */
  const lowBalanceThreshold = ref(0)

  /* ---------------------------- 计算属性 ---------------------------- */

  /** 密文库已启用但未解锁 —— 界面要引导用户先解锁 */
  const isLocked = computed(() => vault.value.enabled && !vault.value.unlocked)

  /** 平台下拉选项（由当前列表去重生成） */
  const platformOptions = computed(() => {
    const seen = new Set<string>()
    for (const entry of entries.value) seen.add(entry.platform)
    return Array.from(seen)
      .sort((a, b) => {
        // 「通用 / 中转」永远排最后：它不是具体厂商，混在中间会打断阅读
        if (a === 'unknown') return 1
        if (b === 'unknown') return -1
        return platformLabel(a).localeCompare(platformLabel(b))
      })
      .map((value) => ({ label: platformLabel(value), value }))
  })

  /** 过滤后的列表 */
  const filtered = computed(() => {
    const kw = keyword.value.trim().toLowerCase()
    return entries.value.filter((entry) => {
      if (filterPlatform.value && entry.platform !== filterPlatform.value) return false
      if (filterStatus.value.length > 0 && !filterStatus.value.includes(entry.status)) return false
      if (kw) {
        const haystack = `${entry.maskedKey} ${entry.note} ${entry.platform}`.toLowerCase()
        if (!haystack.includes(kw)) return false
      }
      return true
    })
  })

  /**
   * 按平台分组；组内余额降序，余额未知排组尾。
   *
   * 为什么余额未知要排尾：列表的阅读顺序是「先看有信息的」，
   * 未知值放在中间会让人误以为它也是某个具体数额。
   */
  const grouped = computed(() => {
    const buckets = new Map<string, KeyEntry[]>()
    for (const entry of filtered.value) {
      const list = buckets.get(entry.platform)
      if (list) list.push(entry)
      else buckets.set(entry.platform, [entry])
    }

    const groups = Array.from(buckets.entries()).map(([platform, list]) => {
      const sorted = [...list].sort((a, b) => {
        const aKnown = a.balanceKnown && a.balance !== null
        const bKnown = b.balanceKnown && b.balance !== null
        if (aKnown && bKnown) return (b.balance ?? 0) - (a.balance ?? 0)
        if (aKnown) return -1
        if (bKnown) return 1
        return a.createdAt - b.createdAt
      })
      const known = sorted.filter((e) => e.balanceKnown && e.balance !== null)
      return {
        platform,
        label: platformLabel(platform),
        entries: sorted,
        /** 组内已知余额合计；一个都没有时为 null */
        balanceSum: known.length > 0
          ? known.reduce((sum, e) => sum + (e.balance ?? 0), 0)
          : null,
        currency: known[0]?.currency ?? null,
      }
    })

    return groups.sort((a, b) => {
      if (a.platform === 'unknown') return 1
      if (b.platform === 'unknown') return -1
      return a.label.localeCompare(b.label)
    })
  })

  /**
   * 四态计数。
   *
   * **只统计已检测过的条目**（`lastCheckedAt !== null`）：
   * 新导入的 Key 状态字段是 `error`，若不排除，「查询失败」会虚高。
   */
  const stats = computed(() => {
    const counter: Record<KeyStatus, number> = {
      active: 0,
      insufficient: 0,
      invalid: 0,
      error: 0,
    }
    let unchecked = 0
    for (const entry of entries.value) {
      if (entry.lastCheckedAt === null) {
        unchecked += 1
        continue
      }
      counter[entry.status] += 1
    }
    return { ...counter, unchecked }
  })

  /* ---------------------------- 进度节流 ---------------------------- */

  let progressTimer: number | null = null
  let pendingProgress: KeyCheckProgress | null = null

  /** 应用一条进度事件（100ms 时间窗合并）。 */
  function applyProgress(payload: KeyCheckProgress): void {
    if (runId.value && payload.runId && payload.runId !== runId.value) return
    if (!runId.value) runId.value = payload.runId

    pendingProgress = payload
    if (progressTimer !== null) return
    progressTimer = window.setTimeout(() => {
      progressTimer = null
      const next = pendingProgress
      pendingProgress = null
      if (!next) return
      progress.value = {
        completed: next.completed,
        total: next.total,
        percent: next.percent,
        currentMaskedKey: next.maskedKey,
        stageLabel: next.stageLabel,
      }
    }, 100)
  }

  /** 应用一条「单条完成」事件：就地替换。 */
  function applyEntryDone(payload: KeyCheckEntryDone): void {
    if (runId.value && payload.runId && payload.runId !== runId.value) return
    const index = entries.value.findIndex((e) => e.id === payload.entry.id)
    if (index >= 0) entries.value[index] = payload.entry
    else entries.value.push(payload.entry)
    progress.value = {
      ...progress.value,
      completed: payload.completed,
      total: payload.total,
    }
  }

  /** 应用「整批结束」事件。 */
  function applyFinished(payload: KeyCheckFinished): void {
    if (runId.value && payload.runId && payload.runId !== runId.value) return
    checking.value = false
    lastSummary.value = payload.summary
    progress.value = {
      ...progress.value,
      completed: payload.checked,
      total: payload.total,
      percent: payload.total > 0 ? Math.round((payload.checked / payload.total) * 100) : 100,
      currentMaskedKey: '',
      stageLabel: '',
    }
  }

  /* ---------------------------- 动作 ---------------------------- */

  /** 载入列表与加密保存状态。 */
  async function load(): Promise<void> {
    loading.value = true
    try {
      const [list, status] = await Promise.all([keyButlerList(), keyButlerVaultStatus()])
      entries.value = list
      vault.value = status
    } finally {
      loading.value = false
    }
  }

  /** 批量导入。 */
  async function importKeys(text: string) {
    const result = await keyButlerImport(text)
    entries.value = result.entries
    vault.value = await keyButlerVaultStatus()
    return result
  }

  /**
   * 发起检测。
   *
   * @param ids 要检测的 id；不传或传空数组表示全部
   */
  async function check(ids: string[] = []): Promise<KeyCheckSummary> {
    if (checking.value) throw new Error('已有检测任务在运行。')
    checking.value = true
    runId.value = ''
    progress.value = {
      completed: 0,
      total: ids.length > 0 ? ids.length : entries.value.length,
      percent: 0,
      currentMaskedKey: '',
      stageLabel: '排队等待',
    }
    try {
      const summary = await keyButlerCheck({
        ids,
        lowBalanceThreshold: lowBalanceThreshold.value,
      })
      // 事件可能全部丢失，这里用返回值兜底覆盖一次
      lastSummary.value = summary
      checking.value = false
      return summary
    } catch (error) {
      checking.value = false
      throw error
    }
  }

  /** 请求中止检测。 */
  async function cancelCheck(): Promise<void> {
    await keyButlerCheckCancel()
  }

  /** 删除一条。 */
  async function remove(id: string): Promise<void> {
    entries.value = await keyButlerDelete(id)
  }

  /** 改备注。 */
  async function updateNote(id: string, note: string): Promise<void> {
    entries.value = await keyButlerUpdateNote(id, note)
  }

  /** 改 Base URL / 余额接口。 */
  async function setConfig(id: string, patch: KeyConfigPatch): Promise<void> {
    entries.value = await keyButlerSetConfig(id, patch)
  }

  /**
   * 取单个完整 Key（仅供一键复制）。
   *
   * 刻意**不**写进 store —— 明文只应在调用栈里短暂存在。
   */
  async function reveal(id: string): Promise<string> {
    return keyButlerReveal(id)
  }

  /** 查询余额快照。 */
  async function loadSnapshots(keyId: string): Promise<Snapshot[]> {
    return keyButlerSnapshots(keyId)
  }

  /** 启用加密保存。 */
  async function enableVault(password: string): Promise<void> {
    vault.value = await keyButlerVaultEnable(password)
    entries.value = await keyButlerList()
  }

  /** 解锁密文库。 */
  async function unlockVault(password: string): Promise<void> {
    vault.value = await keyButlerVaultUnlock(password)
    entries.value = await keyButlerList()
  }

  /** 关闭加密保存。 */
  async function disableVault(): Promise<void> {
    vault.value = await keyButlerVaultDisable()
  }

  /** 清空全部本地状态（切走页面时可选调用）。 */
  function reset(): void {
    entries.value = []
    runId.value = ''
    checking.value = false
    lastSummary.value = null
    progress.value = { completed: 0, total: 0, percent: 0, currentMaskedKey: '', stageLabel: '' }
  }

  return {
    // 状态
    entries,
    vault,
    loading,
    checking,
    runId,
    progress,
    lastSummary,
    keyword,
    filterPlatform,
    filterStatus,
    lowBalanceThreshold,
    // 计算属性
    isLocked,
    platformOptions,
    filtered,
    grouped,
    stats,
    // 事件
    applyProgress,
    applyEntryDone,
    applyFinished,
    // 动作
    load,
    importKeys,
    check,
    cancelCheck,
    remove,
    updateNote,
    setConfig,
    reveal,
    loadSnapshots,
    enableVault,
    unlockVault,
    disableVault,
    reset,
  }
})
