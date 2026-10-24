<script setup lang="ts">
/**
 * 平台分组：一个平台一个分组，标题行显示「平台名 (条数)」
 * 与组内已知余额合计，下方渲染若干 `KeyRow`。
 *
 * 组内排序（余额降序、未知排尾）由 store 的 `grouped` 计算属性完成，
 * 本组件只负责渲染，排序逻辑集中一处避免不一致。
 * 标题行左侧多选框为「整组全选」，服务于批量改平台场景。
 */

import { computed } from 'vue'
import { NCheckbox, NTag } from 'naive-ui'

import type { KeyEntry } from '@/types/contracts'
import KeyRow from './KeyRow.vue'

const props = defineProps<{
  platform: string
  label: string
  entries: KeyEntry[]
  /** 组内已知余额合计；一条都没有时为 null */
  balanceSum: number | null
  currency: string | null
  /** 锁定态：禁用全部操作 */
  locked?: boolean
  /** 是否有检测任务在跑（跑的时候单条「检测」也点不了，后端会拒绝并发） */
  checking?: boolean
  /** 已勾选的 Key id（批量编辑） */
  selectedIds: string[]
}>()

const emit = defineEmits<{
  (e: 'check', entry: KeyEntry): void
  (e: 'history', entry: KeyEntry): void
  (e: 'edit', entry: KeyEntry): void
  (e: 'delete', entry: KeyEntry): void
  (e: 'select', entry: KeyEntry, selected: boolean): void
  (e: 'select-group', ids: string[], selected: boolean): void
  (e: 'probe', entry: KeyEntry): void
  (e: 'fill-config', entry: KeyEntry): void
}>()

/** 组内全部 id */
const ids = computed(() => props.entries.map((entry) => entry.id))

/** 组内已选中的条数 */
const selectedCount = computed(
  () => ids.value.filter((id) => props.selectedIds.includes(id)).length,
)

/** 整组是否已全选 */
const allSelected = computed(() => ids.value.length > 0 && selectedCount.value === ids.value.length)

/** 部分选中（半选态） */
const indeterminate = computed(
  () => selectedCount.value > 0 && selectedCount.value < ids.value.length,
)

/** 余额合计的展示文案 */
function sumText(): string {
  if (props.balanceSum === null) return '余额未知'
  const symbol = props.currency === 'CNY' ? '¥' : '$'
  return `余额 ${symbol}${props.balanceSum.toFixed(2)}`
}
</script>

<template>
  <section class="mb-2 overflow-hidden rounded-lg border border-ms-border bg-ms-panel">
    <!-- 组标题 -->
    <header class="flex items-center justify-between border-b border-ms-border bg-ms-panel-2 px-3 py-1.5">
      <div class="flex items-center gap-2">
        <NCheckbox
          class="shrink-0"
          size="small"
          :checked="allSelected"
          :indeterminate="indeterminate"
          :disabled="locked"
          @update:checked="emit('select-group', ids, $event)"
        />
        <span class="text-[12px] font-semibold text-ms-text">{{ label }}</span>
        <NTag size="tiny" :bordered="false">{{ entries.length }}</NTag>
        <span v-if="selectedCount > 0" class="text-[10px] text-ms-text-faint">
          已选 {{ selectedCount }}
        </span>
      </div>
      <span class="text-[10px] text-ms-text-faint">{{ sumText() }}</span>
    </header>

    <!-- 行 -->
    <KeyRow
      v-for="entry in entries"
      :key="entry.id"
      :entry="entry"
      :locked="locked"
      :busy="checking"
      :selected="selectedIds.includes(entry.id)"
      @check="emit('check', $event)"
      @history="emit('history', $event)"
      @edit="emit('edit', $event)"
      @delete="emit('delete', $event)"
      @select="(target, value) => emit('select', target, value)"
      @probe="emit('probe', $event)"
      @fill-config="emit('fill-config', $event)"
    />
  </section>
</template>
