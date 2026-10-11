<script setup lang="ts">
/**
 * ============================================================================
 * Key 管家 —— 平台分组
 * ============================================================================
 *
 * 一个平台一个分组：标题行显示「平台名 (条数)」与组内已知余额合计，
 * 下面渲染若干 `KeyRow`。
 *
 * 组内排序（余额降序、未知排尾）由 store 的 `grouped` 计算属性完成，
 * 本组件只负责渲染 —— 排序逻辑放在一处，避免两个地方各排一次还不一致。
 * ============================================================================
 */

import { NTag } from 'naive-ui'

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
}>()

const emit = defineEmits<{
  (e: 'check', entry: KeyEntry): void
  (e: 'history', entry: KeyEntry): void
  (e: 'edit', entry: KeyEntry): void
  (e: 'delete', entry: KeyEntry): void
}>()

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
        <span class="text-[12px] font-semibold text-ms-text">{{ label }}</span>
        <NTag size="tiny" :bordered="false">{{ entries.length }}</NTag>
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
      @check="emit('check', $event)"
      @history="emit('history', $event)"
      @edit="emit('edit', $event)"
      @delete="emit('delete', $event)"
    />
  </section>
</template>
