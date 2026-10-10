<script setup lang="ts">
/**
 * ============================================================================
 * ModelTable —— 探测结果表格
 * ============================================================================
 *
 * 【职责】
 *   1. 展示全部探测字段（ID、显示名、工具、附件、推理、上下文、输出、耗时、来源）
 *   2. 支持逐行勾选 / 全选 / 反选 / 按条件批量选择
 *   3. 支持**行内手动纠正**上下文与输出上限（应对服务商不回显数值的情况）
 *   4. 提供「生成配置」入口
 *
 * 【为什么允许手动编辑上限】
 * 上限探测依赖服务端在报错里回显数值。实测中总有服务商只回
 * "invalid max_tokens" 而不给具体约束 —— 与其让用户去改 JSON，
 * 不如在表格里直接改。改动后该项来源标记为「手动」（决策日志 D-10）。
 * ============================================================================
 */

import { computed, h, ref } from 'vue'
import {
  NDataTable,
  NButton,
  NButtonGroup,
  NTag,
  NTooltip,
  NInputNumber,
  NSelect,
  NEmpty,
  NBadge,
  type DataTableColumns,
} from 'naive-ui'

import { useProbe } from '@/composables/useProbe'
import { formatDuration, formatTokens, LIMIT_SOURCE_LABELS } from '@/composables/useProbe'
import type { ModelInfo, TriState, LimitSource } from '@/types/contracts'

const props = defineProps<{
  probe: ReturnType<typeof useProbe>
}>()

const emit = defineEmits<{
  /** 用户点击「生成配置」 */
  (e: 'generate'): void
}>()

/** 表格是否处于紧凑模式 */
const dense = ref(true)

/* ---------------------------------------------------------------------------
 * 三态与来源的渲染辅助
 * ------------------------------------------------------------------------- */

/** 三态 → 标签类型 */
function triType(state: TriState): 'success' | 'error' | 'default' {
  if (state === 'supported') return 'success'
  if (state === 'unsupported') return 'error'
  return 'default'
}

/** 三态 → 显示文案 */
function triLabel(state: TriState): string {
  if (state === 'supported') return '支持'
  if (state === 'unsupported') return '不支持'
  return '未知'
}

/** 三态 → 符号（紧凑展示用） */
function triMark(state: TriState): string {
  if (state === 'supported') return '✓'
  if (state === 'unsupported') return '✕'
  return '?'
}

/** 上限来源 → 标签类型 */
function sourceType(source: LimitSource): 'info' | 'warning' | 'default' {
  if (source === 'declared') return 'info'
  if (source === 'dictionary') return 'info'
  if (source === 'errorParse') return 'info'
  if (source === 'fallback') return 'warning'
  return 'default'
}

/** 上限来源的说明文案（tooltip） */
const SOURCE_HINTS: Record<LimitSource, string> = {
  declared: '接口在 /models 响应中直接声明，来自服务端本身，可信度最高。',
  dictionary: '来自内置模型参数字典，可信度高。',
  errorParse: '通过发送超大 max_tokens 触发服务端报错后解析得出。',
  fallback: '用户手动填写的值。',
  unknown: '未能获取该上限，建议手工核对官方文档。',
}

/* ---------------------------------------------------------------------------
 * 过滤与排序
 * ------------------------------------------------------------------------- */

/** 当前筛选条件 */
const filterCapability = ref<'all' | 'tools' | 'vision' | 'reasoning'>('all')
/** 搜索关键字 */
const keyword = ref('')

/** 经过筛选的模型列表（表格数据源） */
const filteredModels = computed(() => {
  const list = props.probe.store.models
  const kw = keyword.value.trim().toLowerCase()

  return list.filter((model: ModelInfo) => {
    // 关键字过滤
    if (kw) {
      const haystack = `${model.id} ${model.displayName} ${model.ownedBy}`.toLowerCase()
      if (!haystack.includes(kw)) return false
    }
    // 能力过滤
    switch (filterCapability.value) {
      case 'tools':
        return model.supportsTools === 'supported'
      case 'vision':
        return model.supportsVision === 'supported'
      case 'reasoning':
        return model.isReasoning
      default:
        return true
    }
  })
})

/* ---------------------------------------------------------------------------
 * 表格列定义
 * ------------------------------------------------------------------------- */

const columns = computed<DataTableColumns<ModelInfo>>(() => [
  /* ---- 勾选列 ---- */
  {
    type: 'selection',
    // 手动实现选择逻辑，因为此处需要把选择状态同步到 store
    disabled: () => false,
  },

  /* ---- 模型 ID ---- */
  {
    title: () =>
      renderHeader(
        '模型 ID',
        '接口 /models 返回的模型标识。若该模型的上下文/输出/工具/视觉/推理能力来自接口自报（而非实测探测），ID 右侧会标出「接口声明」，悬停可看明细。',
      ),
    key: 'id',
    width: 300,
    fixed: 'left',
    ellipsis: { tooltip: true },
    render: (row) =>
      h('span', { class: 'inline-flex items-center gap-1.5 min-w-0' }, [
        h(
          'span',
          {
            class: 'font-mono text-[11px] text-ms-text truncate',
            // 悬停显示完整的能力来源说明（接口声明 / 实测探测）
            title: row.capsSource || undefined,
          },
          row.id,
        ),
        // 能力来自「接口自报」时给一个可悬停的小标记 ——
        // 这样用户能一眼分辨哪些值不是实测出来的（见 D-34）
        row.capsSource && row.capsSource.startsWith('接口声明')
          ? h(
              'span',
              {
                style:
                  'flex-shrink:0;font-size:10px;line-height:16px;padding:0 4px;border:1px solid currentColor;border-radius:3px;opacity:.65;cursor:help;white-space:nowrap',
                title: row.capsSource,
              },
              '接口声明',
            )
          : null,
      ]),
  },

  /* ---- 显示名 ---- */
  {
    title: '显示名',
    key: 'displayName',
    width: 180,
    ellipsis: { tooltip: true },
    render: (row) =>
      h(
        'span',
        { class: 'text-[11px] text-ms-text-dim' },
        // 显示名与 ID 相同时不重复展示，用一个虚线占位
        row.displayName === row.id ? '—' : row.displayName,
      ),
  },

  /* ---- 工具调用 ---- */
  {
    title: () => renderHeader('工具调用', '发送带 tools 参数的最小请求，检查端点是否接受。'),
    key: 'supportsTools',
    width: 90,
    align: 'center',
    sorter: (a, b) => a.supportsTools.localeCompare(b.supportsTools),
    render: (row) =>
      dense.value
        ? h('span', { class: triColor(row.supportsTools) }, triMark(row.supportsTools))
        : h(
            NTag,
            { size: 'tiny', bordered: false, type: triType(row.supportsTools) },
            { default: () => triLabel(row.supportsTools) },
          ),
  },

  /* ---- 附件上传 ---- */
  {
    title: () => renderHeader('附件上传', '发送 1x1 像素 Base64 图片的 image_url，检查是否被拒绝。'),
    key: 'supportsVision',
    width: 90,
    align: 'center',
    sorter: (a, b) => a.supportsVision.localeCompare(b.supportsVision),
    render: (row) =>
      dense.value
        ? h('span', { class: triColor(row.supportsVision) }, triMark(row.supportsVision))
        : h(
            NTag,
            { size: 'tiny', bordered: false, type: triType(row.supportsVision) },
            { default: () => triLabel(row.supportsVision) },
          ),
  },

  /* ---- 推理模型 ---- */
  {
    title: () => renderHeader('推理', '依据模型名正则（r1 / o1 / thinking 等）或响应中的 reasoning_content 判定。'),
    key: 'isReasoning',
    width: 76,
    align: 'center',
    sorter: (a, b) => Number(a.isReasoning) - Number(b.isReasoning),
    render: (row) =>
      h(
        NTooltip,
        { trigger: 'hover', style: 'max-width: 340px' },
        {
          trigger: () =>
            h(
              'span',
              { class: row.isReasoning ? 'text-ms-info' : 'text-ms-text-faint' },
              row.isReasoning ? '●' : '—',
            ),
          default: () => row.reasoningEvidence || '无判定依据',
        },
      ),
  },

  /* ---- 上下文上限（可编辑） ---- */
  {
    title: () => renderHeader('上下文上限', '模型可接受的最大输入 token 数。双击数字可手动修改。'),
    key: 'contextLimit',
    width: 128,
    align: 'center',
    sorter: (a, b) => a.contextLimit - b.contextLimit,
    render: (row) => renderLimitCell(row, 'contextLimit'),
  },

  /* ---- 输出上限（可编辑） ---- */
  {
    title: () => renderHeader('输出上限', '单次回复允许的最大 token 数。双击数字可手动修改。'),
    key: 'maxOutputLimit',
    width: 128,
    align: 'center',
    sorter: (a, b) => a.maxOutputLimit - b.maxOutputLimit,
    render: (row) => renderLimitCell(row, 'maxOutputLimit'),
  },

  /* ---- 耗时 ---- */
  {
    title: '耗时',
    key: 'elapsedMs',
    width: 80,
    align: 'right',
    sorter: (a, b) => a.elapsedMs - b.elapsedMs,
    render: (row) =>
      h(
        'span',
        { class: 'font-mono text-[10px] text-ms-text-faint' },
        formatDuration(row.elapsedMs),
      ),
  },

  /* ---- 错误 ---- */
  {
    title: '错误',
    key: 'errors',
    width: 70,
    align: 'center',
    render: (row) => {
      const count = row.errors.filter((e) => e.kind !== 'cancelled').length
      if (count === 0) {
        return h('span', { class: 'text-ms-text-faint' }, '—')
      }
      return h(
        NTooltip,
        { trigger: 'hover', style: 'max-width: 380px' },
        {
          trigger: () =>
            h(NBadge, { value: count, type: 'error', 'show-zero': false }),
          default: () =>
            row.errors
              .filter((e) => e.kind !== 'cancelled')
              .map((e) => h('div', { class: 'text-[11px]' }, e.message))
              .concat([h('div', { class: 'mt-1 text-[10px] opacity-70' }, `共 ${count} 条`)])[0],
        },
      )
    },
  },
])

/* ---------------------------------------------------------------------------
 * 单元格渲染辅助
 * ------------------------------------------------------------------------- */

/** 三态的颜色类 */
function triColor(state: TriState): string {
  if (state === 'supported') return 'text-ms-success'
  if (state === 'unsupported') return 'text-ms-danger'
  return 'text-ms-text-faint'
}

/** 渲染一个带 tooltip 的表头 */
function renderHeader(title: string, hint: string) {
  return h(
    NTooltip,
    { trigger: 'hover', style: 'max-width: 320px' },
    {
      trigger: () =>
        h('span', { class: 'cursor-help border-b border-dashed border-ms-border-strong' }, title),
      default: () => hint,
    },
  )
}

/** 正在编辑的单元格标识（`模型ID:字段`） */
const editingCell = ref<string | null>(null)

/**
 * 渲染可行内编辑的上限单元格。
 *
 * 交互：单击展示值 + 来源徽标；双击进入编辑态；回车/失焦提交。
 */
function renderLimitCell(row: ModelInfo, field: 'contextLimit' | 'maxOutputLimit') {
  const cellKey = `${row.id}:${field}`
  const value = row[field]
  const source = field === 'contextLimit' ? row.contextLimitSource : row.maxOutputLimitSource

  // 编辑态：渲染一个数字输入框
  if (editingCell.value === cellKey) {
    return h(NInputNumber, {
      value: value > 0 ? value : null,
      min: 0,
      max: 32_000_000,
      size: 'tiny',
      autofocus: true,
      showButton: false,
      placeholder: '留空=未知',
      style: 'width: 104px',
      'onUpdate:value': (next: number | null) => {
        props.probe.store.patchModel(row.id, {
          [field]: next ?? 0,
        } as Partial<ModelInfo>)
      },
      onBlur: () => {
        editingCell.value = null
      },
      onKeydown: (event: KeyboardEvent) => {
        if (event.key === 'Enter' || event.key === 'Escape') {
          editingCell.value = null
        }
      },
    })
  }

  // 展示态
  return h(
    NTooltip,
    { trigger: 'hover', style: 'max-width: 340px' },
    {
      trigger: () =>
        h(
          'div',
          {
            class: 'flex cursor-text items-center justify-center gap-1',
            onDblclick: () => {
              editingCell.value = cellKey
            },
          },
          [
            h(
              'span',
              {
                class:
                  value > 0
                    ? 'font-mono text-[11px] text-ms-text'
                    : 'font-mono text-[11px] text-ms-text-faint',
              },
              value > 0 ? formatTokens(value) : '未知',
            ),
            value > 0
              ? h(
                  NTag,
                  {
                    size: 'tiny',
                    bordered: false,
                    type: sourceType(source),
                    style: 'font-size: 9px; padding: 0 3px;',
                  },
                  { default: () => LIMIT_SOURCE_LABELS[source] ?? source },
                )
              : null,
          ],
        ),
      default: () =>
        value > 0
          ? `${value.toLocaleString()} tokens · 来源：${LIMIT_SOURCE_LABELS[source] ?? source}。${SOURCE_HINTS[source] ?? ''}（双击可修改）`
          : `${SOURCE_HINTS[source] ?? ''}（双击可手动填写）`,
    },
  )
}

/* ---------------------------------------------------------------------------
 * 选择逻辑（与 store 双向同步）
 * ------------------------------------------------------------------------- */

/**
 * 表格的勾选行 key。
 *
 * NDataTable 的 `checked-row-keys` 受控模式需要一个字符串/数字数组，
 * 直接把 store 的 selectedIds 传进去即可。
 */
const checkedRowKeys = computed({
  get: () => props.probe.store.selectedIds,
  set: (keys: (string | number)[]) => {
    props.probe.store.selectedIds = keys.map((k) => String(k))
  },
})

/** 选择当前筛选结果 */
function selectFiltered(): void {
  const ids = filteredModels.value.map((m: ModelInfo) => m.id)
  const merged = new Set([...props.probe.store.selectedIds, ...ids])
  props.probe.store.selectedIds = Array.from(merged)
}

/** 取消选择当前筛选结果 */
function unselectFiltered(): void {
  const ids = new Set(filteredModels.value.map((m: ModelInfo) => m.id))
  props.probe.store.selectedIds = props.probe.store.selectedIds.filter(
    (id: string) => !ids.has(id),
  )
}

/** 按条件选择预设 */
const SELECT_PRESETS = [
  { label: '仅选支持工具调用的', value: 'tools' },
  { label: '仅选支持附件上传的', value: 'vision' },
  { label: '仅选推理模型', value: 'reasoning' },
  { label: '仅选三项全支持的', value: 'allCapability' },
  { label: '仅选上限已知的', value: 'knownLimit' },
]

/** 应用一个选择预设 */
function applySelectPreset(value: string): void {
  const store = props.probe.store
  switch (value) {
    case 'tools':
      store.selectByPredicate((m) => m.supportsTools === 'supported')
      break
    case 'vision':
      store.selectByPredicate((m) => m.supportsVision === 'supported')
      break
    case 'reasoning':
      store.selectByPredicate((m) => m.isReasoning)
      break
    case 'allCapability':
      store.selectByPredicate(
        (m) =>
          m.supportsTools === 'supported' &&
          m.supportsVision === 'supported' &&
          m.isReasoning,
      )
      break
    case 'knownLimit':
      store.selectByPredicate((m) => m.contextLimit > 0 && m.maxOutputLimit > 0)
      break
    default:
      break
  }
}

/** 筛选选项 */
const FILTER_OPTIONS = [
  { label: '全部', value: 'all' },
  { label: '支持工具', value: 'tools' },
  { label: '支持附件', value: 'vision' },
  { label: '推理模型', value: 'reasoning' },
]
</script>

<template>
  <section class="ms-panel flex min-h-0 flex-col overflow-hidden">
    <!-- ---------- 工具条 ---------- -->
    <div class="flex shrink-0 flex-wrap items-center gap-2 border-b border-ms-border px-3 py-2">
      <span class="ms-panel-title mr-1">探测结果</span>
      <NTag size="tiny" :bordered="false">
        {{ filteredModels.length }} / {{ probe.store.models.length }}
      </NTag>

      <div class="mx-1 h-4 w-px bg-ms-border" />

      <!-- 关键字搜索 -->
      <input
        v-model="keyword"
        type="text"
        placeholder="搜索模型…"
        class="w-[150px] rounded border border-ms-border-strong bg-ms-bg px-2 py-0.5 text-[11px] text-ms-text outline-none focus:border-ms-accent"
      />

      <!-- 能力筛选 -->
      <NSelect
        v-model:value="filterCapability"
        :options="FILTER_OPTIONS"
        size="tiny"
        style="width: 104px"
      />

      <div class="mx-1 h-4 w-px bg-ms-border" />

      <!-- 选择操作 -->
      <NButtonGroup size="tiny">
        <NButton @click="selectFiltered()">选中筛选结果</NButton>
        <NButton @click="unselectFiltered()">取消筛选结果</NButton>
        <NButton @click="probe.store.invertSelection()">反选</NButton>
        <NButton @click="probe.store.selectAll()">全选</NButton>
        <NButton @click="probe.store.selectNone()">清空</NButton>
      </NButtonGroup>

      <!-- 条件预设 -->
      <NSelect
        :options="SELECT_PRESETS"
        size="tiny"
        placeholder="条件选择…"
        style="width: 160px"
        :value="null"
        @update:value="applySelectPreset"
      />

      <div class="flex-1" />

      <!-- 生成配置 -->
      <NButton
        type="primary"
        size="small"
        :disabled="probe.store.selectedIds.length === 0"
        @click="emit('generate')"
      >
        生成配置（{{ probe.store.selectedIds.length }}）
      </NButton>
    </div>

    <!-- ---------- 表格 ---------- -->
    <div class="min-h-0 flex-1">
      <NEmpty
        v-if="probe.store.models.length === 0"
        description="暂无探测结果"
        class="py-16"
      />
      <NDataTable
        v-else
        v-model:checked-row-keys="checkedRowKeys"
        :columns="columns"
        :data="filteredModels"
        :row-key="(row: ModelInfo) => row.id"
        :size="dense ? 'small' : 'medium'"
        :bordered="false"
        :single-line="false"
        :max-height="'100%'"
        virtual-scroll
        :scroll-x="1180"
        flex-height
        class="h-full"
      />
    </div>

    <!-- ---------- 底部说明 ---------- -->
    <div
      class="shrink-0 border-t border-ms-border px-3 py-1.5 text-[10px] text-ms-text-faint"
    >
      图例：<span class="text-ms-success">✓ 支持</span> ·
      <span class="text-ms-danger">✕ 不支持</span> ·
      <span class="text-ms-text-faint">? 未知</span>
      &nbsp;|&nbsp; 上限一列双击数字可手动修改
    </div>
  </section>
</template>
