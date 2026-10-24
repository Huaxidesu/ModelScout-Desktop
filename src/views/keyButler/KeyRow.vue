<script setup lang="ts">
/**
 * 单条 Key 行：展示脱敏串、状态、余额、最近检测时间，
 * 并提供「检测 / 历史 / 复制 / 编辑·删除」操作。
 *
 * 状态显示的三条规则：
 *   1. `lastCheckedAt === null` 显示「未检测」——新导入的 Key 状态字段是
 *      `error`，只看 status 会误显示成「查询失败」；
 *   2. `status === 'error' && keyValid === true` 时 tooltip 必须写
 *      「检测未完成（Key 有效）」，这是「部分成功」的唯一表达方式；
 *   3. 余额未知（`balanceKnown === false`）显示 `—`，不显示 `$0.00`。
 *
 * 复制的安全约定：明文只在 `reveal()` 的返回值里短暂存在，
 * 复制完成后立即把局部变量置空，不进 store、不进日志、不进 DOM。
 */

import { computed, ref } from 'vue'
import { NButton, NCheckbox, NDropdown, NTooltip, useMessage } from 'naive-ui'

import { keyButlerReveal } from '@/api/commands'
import type { KeyEntry, KeyStatus } from '@/types/contracts'
import { formatBalance, formatRelativeTime, STATUS_LABELS } from '@/stores/keyButler'
import { copyText } from './clipboard'

const props = defineProps<{
  entry: KeyEntry
  /** 该行是否正在检测（禁用按钮） */
  busy?: boolean
  /** 是否处于锁定态（禁用全部操作） */
  locked?: boolean
  /** 是否已勾选（批量编辑） */
  selected?: boolean
}>()

const emit = defineEmits<{
  (e: 'check', entry: KeyEntry): void
  (e: 'history', entry: KeyEntry): void
  (e: 'edit', entry: KeyEntry): void
  (e: 'delete', entry: KeyEntry): void
  (e: 'select', entry: KeyEntry, selected: boolean): void
}>()

const message = useMessage()

/** 是否从未检测过 */
const neverChecked = computed(() => props.entry.lastCheckedAt === null)

/**
 * 状态徽标的外观。
 *
 * 四态各配图标与颜色（✅ 绿 / ❌ 橙(余额不足) / ❌ 红(失效) / ⚠ 灰(查询失败)）。
 */
const statusMeta = computed<{ icon: string; className: string; label: string }>(() => {
  if (neverChecked.value) {
    return { icon: '○', className: 'text-ms-text-faint', label: '未检测' }
  }
  switch (props.entry.status as KeyStatus) {
    case 'active':
      return { icon: '✅', className: 'text-ms-success', label: STATUS_LABELS.active }
    case 'insufficient':
      return { icon: '❌', className: 'text-ms-warning', label: STATUS_LABELS.insufficient }
    case 'invalid':
      return { icon: '❌', className: 'text-ms-danger', label: STATUS_LABELS.invalid }
    default:
      return { icon: '⚠', className: 'text-ms-text-faint', label: STATUS_LABELS.error }
  }
})

/** 状态的详细说明（tooltip） */
const statusHint = computed(() => {
  if (neverChecked.value) return '这条 Key 还没有检测过，点「检测」开始。'
  const { status, keyValid, errorCategory } = props.entry
  if (status === 'error' && keyValid === true) {
    return `检测未完成（Key 有效）：Key 本身可用，但余额查询没成功${
      errorCategory ? `（${errorCategory}）` : ''
    }。`
  }
  if (status === 'error') {
    return `查询失败${errorCategory ? `（${errorCategory}）` : ''}：网络、限流或服务端问题，Key 是否有效尚未确认。`
  }
  if (status === 'insufficient') return 'Key 有效，但余额低于阈值。'
  if (status === 'invalid') return '服务端返回 401 / 403，这条 Key 已失效。'
  return 'Key 有效，余额充足。'
})

/** 「⋯」菜单项 */
const menuOptions = [
  { label: '编辑（备注 / Base URL / 余额接口）', key: 'edit' },
  { label: '删除', key: 'delete' },
]

/** 复制中（避免重复点击） */
const copying = ref(false)

/**
 * 复制完整 Key：取明文 → 写剪贴板 → 丢弃引用。
 *
 * 明文只作为局部变量存在，不进 store、不进日志、不进 DOM。
 * （JS 无法像 Rust 那样主动清零字符串内存，但至少保证它不被持有。）
 */
async function handleCopy(): Promise<void> {
  if (copying.value) return
  copying.value = true
  try {
    const plaintext = await keyButlerReveal(props.entry.id)
    if (!plaintext) {
      message.warning('没有取到 Key 内容。')
      return
    }
    await copyText(plaintext)
    message.success('已复制完整 Key')
  } catch (error) {
    const text = error instanceof Error ? error.message : String(error)
    message.error(`复制失败：${text}`)
  } finally {
    copying.value = false
  }
}

/** 「⋯」菜单点击 */
function handleMenu(key: string): void {
  if (key === 'edit') emit('edit', props.entry)
  if (key === 'delete') emit('delete', props.entry)
}
</script>

<template>
  <div class="flex items-center gap-3 border-b border-ms-border/60 px-3 py-1.5 text-[11px] last:border-b-0 hover:bg-white/[0.02]">
    <!-- 多选（批量编辑） -->
    <NCheckbox
      class="shrink-0"
      size="small"
      :checked="selected"
      :disabled="locked"
      @update:checked="emit('select', entry, $event)"
    />

    <!-- 脱敏 Key -->
    <div class="min-w-0 flex-1">
      <span class="font-mono text-ms-text">{{ entry.maskedKey }}</span>
      <span v-if="entry.note" class="ml-2 text-[10px] text-ms-text-faint">#{{ entry.note }}</span>
    </div>

    <!-- 状态 -->
    <NTooltip trigger="hover" style="max-width: 320px">
      <template #trigger>
        <span class="flex w-[86px] shrink-0 items-center gap-1" :class="statusMeta.className">
          <span>{{ statusMeta.icon }}</span>
          <span>{{ statusMeta.label }}</span>
        </span>
      </template>
      {{ statusHint }}
    </NTooltip>

    <!-- 余额 -->
    <span class="w-[76px] shrink-0 text-right font-mono text-ms-text-dim">
      {{ formatBalance(entry) }}
    </span>

    <!-- 最近检测 -->
    <span class="w-[64px] shrink-0 text-right text-ms-text-faint">
      {{ formatRelativeTime(entry.lastCheckedAt) }}
    </span>

    <!-- 操作 -->
    <div class="flex shrink-0 items-center gap-1">
      <NButton size="tiny" quaternary :disabled="busy || locked" @click="emit('check', entry)">
        检测
      </NButton>
      <NButton size="tiny" quaternary :disabled="locked" @click="emit('history', entry)">
        历史
      </NButton>
      <NButton size="tiny" quaternary :loading="copying" :disabled="locked" @click="handleCopy">
        复制
      </NButton>
      <NDropdown trigger="click" :options="menuOptions" @select="handleMenu">
        <NButton size="tiny" quaternary :disabled="locked">⋯</NButton>
      </NDropdown>
    </div>
  </div>
</template>
