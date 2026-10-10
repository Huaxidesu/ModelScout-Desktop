/**
 * ============================================================================
 * ModelScout —— 全局状态管理单元测试
 * ============================================================================
 *
 * 【为什么需要这个文件】（见 docs/DECISIONS.md D-37）
 *
 * D-37 中「界面永远 0/0」的缺陷有一条**前端数据链**：
 *
 *     后端 run_engine → probe://model-done 事件 → applyModelDone → models
 *                                 ↘ applySummary（命令返回值）→ models
 *
 * 后端那一半已经由 `src-tauri/tests/engine_integration.rs` 覆盖；
 * **前端这一半此前完全没有测试**。而它恰恰是「把已经拿到的数据又清空」的地方：
 *
 *     `applySummary` 原本无条件执行 `models.value = summary.models`，
 *     于是当后端返回空数组时，会把事件路径辛苦填好的结果**覆盖成空**，
 *     界面显示「0/0 暂无探测结果」，而 `selectedIds` 里还留着 10 个 id
 *     —— 这正是用户截图里那个自相矛盾的「生成配置 (10)」。
 *
 * 所以这里专门锁住：**`applySummary` 收到空数组时不得清空已有结果**。
 * ============================================================================
 */

import { beforeEach, describe, expect, it } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'

import { useProbeStore } from './probe'
import type { ModelInfo, ProbeModelDone, ProbeProgress, ProbeRunSummary } from '@/types/contracts'

/* ---------------------------------------------------------------------------
 * 测试数据工厂
 * ------------------------------------------------------------------------- */

/** 构造一个字段完整的 ModelInfo，避免每个用例都写 17 个字段。 */
function makeModel(id: string, overrides: Partial<ModelInfo> = {}): ModelInfo {
  return {
    id,
    displayName: id,
    ownedBy: 'test',
    created: 0,
    supportsTools: 'unknown',
    supportsVision: 'unknown',
    supportsStreaming: 'unknown',
    isReasoning: false,
    reasoningEvidence: '',
    contextLimit: 0,
    maxOutputLimit: 0,
    contextLimitSource: 'unknown',
    maxOutputLimitSource: 'unknown',
    capsSource: '实测探测',
    errors: [],
    elapsedMs: 0,
    finalStage: 'done',
    ...overrides,
  }
}

function makeSummary(overrides: Partial<ProbeRunSummary> = {}): ProbeRunSummary {
  return {
    runId: 'run-1',
    status: 'done',
    totalModels: 0,
    models: [],
    errors: [],
    elapsedMs: 0,
    ...overrides,
  }
}

function makeModelDone(modelId: string, completed: number, total: number): ProbeModelDone {
  return {
    runId: 'run-1',
    model: makeModel(modelId),
    completed,
    total,
  }
}

function makeProgress(overrides: Partial<ProbeProgress> = {}): ProbeProgress {
  return {
    runId: 'run-1',
    modelId: 'a',
    stage: 'toolProbe',
    stageLabel: '工具调用',
    completed: 0,
    total: 0,
    percent: 0,
    elapsedMs: 0,
    ...overrides,
  }
}

/** 每个用例都用一份干净的 store。 */
function freshStore() {
  setActivePinia(createPinia())
  return useProbeStore()
}

beforeEach(() => {
  setActivePinia(createPinia())
})

/* ---------------------------------------------------------------------------
 * D-37 回归测试：applySummary 不得用空数组清空已有结果
 * ------------------------------------------------------------------------- */

describe('applySummary —— D-37 回归', () => {
  it('后端返回空数组时，不得清空事件路径已经填好的结果', () => {
    const store = freshStore()

    // 模拟：事件路径已经收到 3 个模型（applyModelDone）
    store.applyModelDone(makeModelDone('alpha', 1, 3))
    store.applyModelDone(makeModelDone('beta', 2, 3))
    store.applyModelDone(makeModelDone('gamma', 3, 3))
    expect(store.models).toHaveLength(3)
    expect(store.selectedIds).toHaveLength(3)

    // 模拟：命令返回了一个**空** summary（正是 D-37 后端的症状）
    store.applySummary(
      makeSummary({ status: 'failed', totalModels: 0, models: [], errors: [] }),
    )

    // 核心断言：已有结果不能被空数组覆盖
    expect(store.models).toHaveLength(3)
    expect(store.total).toBe(3)
    expect(store.completed).toBe(3)
    // 状态仍如实反映后端返回的 failed（用户需要知道这轮有问题）
    expect(store.status).toBe('failed')
  })

  it('后端返回非空结果时，正常覆盖（这是主路径）', () => {
    const store = freshStore()
    store.applyModelDone(makeModelDone('stale', 1, 1))
    expect(store.models).toHaveLength(1)

    store.applySummary(
      makeSummary({
        status: 'done',
        totalModels: 2,
        models: [makeModel('x'), makeModel('y')],
      }),
    )

    expect(store.models.map((m) => m.id)).toEqual(['x', 'y'])
    expect(store.total).toBe(2)
    expect(store.completed).toBe(2)
    expect(store.status).toBe('done')
    // 新模型应被自动勾选，且旧的「幽灵 id」必须被清理
    expect(store.selectedIds).toEqual(['x', 'y'])
  })

  it('替换结果时清理「幽灵勾选 id」—— 截图中「生成配置 (10)」的成因', () => {
    const store = freshStore()

    // 事件路径先填了 3 个模型（都会被自动勾选）
    store.applyModelDone(makeModelDone('a', 1, 3))
    store.applyModelDone(makeModelDone('b', 2, 3))
    store.applyModelDone(makeModelDone('c', 3, 3))
    expect(store.selectedIds).toHaveLength(3)

    // 后端返回的结果里只剩 1 个（另外两个已不存在）
    store.applySummary(makeSummary({ status: 'done', totalModels: 1, models: [makeModel('a')] }))

    expect(store.models.map((m) => m.id)).toEqual(['a'])
    // 关键：勾选数量必须与「实际存在的模型」一致。
    //   否则界面会显示「生成配置 (3)」而实际只能生成 1 个 ——
    //   这正是 D-37 截图里「结果表 0/0、按钮却写生成配置 (10)」的成因。
    expect(store.selectedIds).toEqual(['a'])
  })

  it('applySnapshot 会清理幽灵勾选 id（重同步不做自动全选）', () => {
    const store = freshStore()
    store.applyModelDone(makeModelDone('ghost', 1, 1))
    expect(store.selectedIds).toEqual(['ghost'])

    store.applySnapshot({
      isRunning: false,
      runId: 'run-2',
      status: 'done',
      totalModels: 1,
      completedModels: 1,
      models: [makeModel('real')],
    })

    expect(store.models.map((m) => m.id)).toEqual(['real'])
    // 幽灵 id 必须清掉；快照路径刻意**不**做自动全选
    // （重同步是恢复现场，不应覆盖用户已有的勾选偏好）
    expect(store.selectedIds).not.toContain('ghost')
    expect(store.selectedIds).toEqual([])
  })

  it('空 store + 空 summary → 保持空（不应误判为「保留」）', () => {
    const store = freshStore()
    store.applySummary(makeSummary({ status: 'failed', models: [] }))
    expect(store.models).toHaveLength(0)
    expect(store.total).toBe(0)
    expect(store.status).toBe('failed')
  })
})

/* ---------------------------------------------------------------------------
 * applyModelDone
 * ------------------------------------------------------------------------- */

describe('applyModelDone', () => {
  it('新模型会被追加并自动勾选', () => {
    const store = freshStore()
    store.applyModelDone(makeModelDone('alpha', 1, 2))

    expect(store.models.map((m) => m.id)).toEqual(['alpha'])
    expect(store.selectedIds).toEqual(['alpha'])
    expect(store.completed).toBe(1)
    expect(store.total).toBe(2)
  })

  it('同一模型第二次到达时原地更新，不重复追加', () => {
    const store = freshStore()
    store.applyModelDone(makeModelDone('alpha', 1, 2))

    const updated = makeModel('alpha', { contextLimit: 128000 })
    store.applyModelDone({ runId: 'run-1', model: updated, completed: 1, total: 2 })

    expect(store.models).toHaveLength(1)
    expect(store.models[0].contextLimit).toBe(128000)
    // 勾选状态不应重复
    expect(store.selectedIds).toEqual(['alpha'])
  })

  it('runId 不匹配的事件被丢弃（防止旧运行的迟到事件污染新运行）', () => {
    const store = freshStore()
    store.beginRun('run-2', 5)

    store.applyModelDone({ runId: 'run-1', model: makeModel('ghost'), completed: 1, total: 1 })

    expect(store.models).toHaveLength(0)
  })
})

/* ---------------------------------------------------------------------------
 * applyProgress
 * ------------------------------------------------------------------------- */

describe('applyProgress', () => {
  it('写入 completed / total / percent 与阶段映射', () => {
    const store = freshStore()
    store.applyProgress(makeProgress({ completed: 2, total: 5, percent: 40, stage: 'visionProbe' }))

    expect(store.completed).toBe(2)
    expect(store.total).toBe(5)
    expect(store.percent).toBe(40)
    expect(store.stageByModel.a).toBe('visionProbe')
  })

  it('runId 不匹配时丢弃', () => {
    const store = freshStore()
    store.beginRun('run-2', 5)
    store.applyProgress(makeProgress({ runId: 'run-1', total: 99 }))
    expect(store.total).toBe(5)
  })

  it('首次事件先于 beginRun 到达时，采用事件里的 runId（竞态兜底）', () => {
    const store = freshStore()
    store.applyProgress(makeProgress({ runId: 'run-9', completed: 1, total: 4 }))

    expect(store.runId).toBe('run-9')
    expect(store.total).toBe(4)
  })
})

/* ---------------------------------------------------------------------------
 * resetRun / beginRun
 * ------------------------------------------------------------------------- */

describe('resetRun 与 beginRun', () => {
  it('resetRun 清空全部结果与勾选，但保留配置', () => {
    const store = freshStore()
    store.config.baseUrl = 'https://example.com/v1'
    store.applyModelDone(makeModelDone('alpha', 1, 1))
    store.applyLog({ runId: 'run-1', level: 'info', modelId: '', message: 'hi', timestamp: 0 })
    store.applyProgress(makeProgress({ completed: 1, total: 1, percent: 100 }))

    store.resetRun()

    expect(store.models).toHaveLength(0)
    expect(store.selectedIds).toHaveLength(0)
    expect(store.logs).toHaveLength(0)
    expect(store.total).toBe(0)
    expect(store.completed).toBe(0)
    expect(store.percent).toBe(0)
    expect(store.status).toBe('idle')
    // 配置必须保留
    expect(store.config.baseUrl).toBe('https://example.com/v1')
  })

  it('beginRun 重置结果并进入 running', () => {
    const store = freshStore()
    store.applyModelDone(makeModelDone('alpha', 1, 1))

    store.beginRun('run-2', 7)

    expect(store.runId).toBe('run-2')
    expect(store.total).toBe(7)
    expect(store.status).toBe('running')
    expect(store.hasRun).toBe(true)
    expect(store.models).toHaveLength(0)
  })
})

/* ---------------------------------------------------------------------------
 * applyFinished
 * ------------------------------------------------------------------------- */

describe('applyFinished', () => {
  it('写入终态与计数；done 时进度补到 100', () => {
    const store = freshStore()
    store.applyFinished({ status: 'done', completed: 3, total: 3, elapsedMs: 123, errors: [] })

    expect(store.status).toBe('done')
    expect(store.completed).toBe(3)
    expect(store.total).toBe(3)
    expect(store.percent).toBe(100)
    expect(store.finishedAt).toBeGreaterThan(0)
  })

  it('cancelled 时保留原有百分比，不强行补 100', () => {
    const store = freshStore()
    store.applyProgress(makeProgress({ completed: 1, total: 4, percent: 25 }))
    store.applyFinished({ status: 'cancelled', completed: 1, total: 4, elapsedMs: 10, errors: [] })

    expect(store.status).toBe('cancelled')
    expect(store.percent).toBe(25)
  })
})

/* ---------------------------------------------------------------------------
 * 勾选相关
 * ------------------------------------------------------------------------- */

describe('勾选逻辑', () => {
  function seedThree(store: ReturnType<typeof useProbeStore>) {
    store.applyModelDone(makeModelDone('a', 1, 3))
    store.applyModelDone(makeModelDone('b', 2, 3))
    store.applyModelDone(makeModelDone('c', 3, 3))
  }

  it('toggleModel 切换单个勾选', () => {
    const store = freshStore()
    seedThree(store)

    // toggleModel 是「切换」语义：勾选中则取消，反之则勾选
    store.toggleModel('b')
    expect([...store.selectedIds].sort()).toEqual(['a', 'c'])
    store.toggleModel('b')
    expect([...store.selectedIds].sort()).toEqual(['a', 'b', 'c'])
  })

  it('selectAll / selectNone / invertSelection', () => {
    const store = freshStore()
    seedThree(store)

    store.selectNone()
    expect(store.selectedIds).toHaveLength(0)

    store.selectAll()
    expect([...store.selectedIds].sort()).toEqual(['a', 'b', 'c'])

    store.toggleModel('b') // 取消 'b'
    store.invertSelection() // 反选 → 只剩 'b'
    expect(store.selectedIds).toEqual(['b'])
  })

  it('selectedModels 只返回被勾选的模型', () => {
    const store = freshStore()
    seedThree(store)
    store.selectNone()
    store.toggleModel('c')

    expect(store.selectedModels.map((m) => m.id)).toEqual(['c'])
  })
})

/* ---------------------------------------------------------------------------
 * patchModel（手动修改上限）
 * ------------------------------------------------------------------------- */

describe('patchModel', () => {
  it('手工改上限会把来源标为 fallback', () => {
    const store = freshStore()
    store.applyModelDone(makeModelDone('alpha', 1, 1))

    store.patchModel('alpha', { contextLimit: 65536 })

    const m = store.models[0]
    expect(m.contextLimit).toBe(65536)
    expect(m.contextLimitSource).toBe('fallback')
  })
})

/* ---------------------------------------------------------------------------
 * capabilityStats
 * ------------------------------------------------------------------------- */

describe('capabilityStats', () => {
  it('按三态统计支持数量', () => {
    const store = freshStore()
    store.applyModelDone({
      runId: 'run-1',
      model: makeModel('a', { supportsTools: 'supported', supportsVision: 'unsupported', isReasoning: true }),
      completed: 1,
      total: 2,
    })
    store.applyModelDone({
      runId: 'run-1',
      model: makeModel('b', { supportsTools: 'supported', supportsVision: 'supported', isReasoning: false }),
      completed: 2,
      total: 2,
    })

    expect(store.capabilityStats).toEqual({ tools: 2, vision: 1, reasoning: 1, total: 2 })
  })
})
