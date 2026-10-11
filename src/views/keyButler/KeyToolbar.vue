<script setup lang="ts">
/**
 * ============================================================================
 * Key 管家 —— 工具栏
 * ============================================================================
 *
 * 左侧：批量粘贴 / 检测全部 / 停止 / 加密保存开关
 * 右侧：过滤（平台 / 状态 / 搜索）+ 余额不足阈值
 *
 * 【为什么阈值放在这里】
 * 阈值只影响「有效但余额低」这一类怎么归类，属于全局判定参数，
 * 不适合塞进每一条 Key 里。它随检测请求一起下发给后端。
 * ============================================================================
 */

import { computed } from 'vue'
import { NButton, NInput, NInputNumber, NSelect, NTag, NTooltip, useMessage } from 'naive-ui'

import { useKeyButlerStore } from '@/stores/keyButler'
import type { KeyStatus } from '@/types/contracts'
import { ProbeApiError } from '@/api/commands'

const emit = defineEmits<{
  (e: 'import'): void
  (e: 'vault'): void
}>()

const store = useKeyButlerStore()
const message = useMessage()

/** 状态过滤选项 */
const STATUS_OPTIONS: Array<{ label: string; value: KeyStatus }> = [
  { label: '可用', value: 'active' },
  { label: '余额不足', value: 'insufficient' },
  { label: '已失效', value: 'invalid' },
  { label: '查询失败', value: 'error' },
]

/** 加密保存按钮的文案与类型 */
const vaultMeta = computed(() => {
  if (!store.vault.enabled) {
    return { text: '启用本地保存 🔒', type: 'default' as const, hint: '默认不保存：关闭应用即清空。启用后会用主密码加密写入本机。' }
  }
  if (!store.vault.unlocked) {
    return { text: '已加密 · 待解锁', type: 'warning' as const, hint: '密文库已存在但当前未解锁，点此输入主密码。' }
  }
  return { text: '已加密 · 已解锁', type: 'success' as const, hint: 'Key 已用主密码加密保存在本机，改动会即时重新加密落盘。' }
})

/** 发起检测（全部） */
async function checkAll(): Promise<void> {
  if (store.entries.length === 0) {
    message.warning('还没有 Key，先点「批量粘贴」导入。')
    return
  }
  try {
    await store.check()
  } catch (error) {
    message.error(error instanceof ProbeApiError ? error.message : String(error))
  }
}

/** 中止检测 */
async function stopCheck(): Promise<void> {
  try {
    await store.cancelCheck()
    message.info('已请求停止，正在等待在途请求结束…')
  } catch (error) {
    message.error(error instanceof ProbeApiError ? error.message : String(error))
  }
}
</script>

<template>
  <div class="flex flex-wrap items-center gap-2 border-b border-ms-border px-3 py-2">
    <!-- 主操作 -->
    <NButton size="small" type="primary" ghost :disabled="store.isLocked" @click="emit('import')">
      批量粘贴
    </NButton>

    <NButton
      size="small"
      type="primary"
      :disabled="store.checking || store.entries.length === 0 || store.isLocked"
      :loading="store.checking"
      @click="checkAll"
    >
      {{ store.checking ? '检测中…' : '检测全部' }}
    </NButton>

    <NButton size="small" type="error" ghost :disabled="!store.checking" @click="stopCheck">
      停止
    </NButton>

    <!-- 加密保存 -->
    <NTooltip trigger="hover" style="max-width: 340px">
      <template #trigger>
        <NButton size="small" :type="vaultMeta.type" quaternary @click="emit('vault')">
          {{ vaultMeta.text }}
        </NButton>
      </template>
      {{ vaultMeta.hint }}
    </NTooltip>

    <div class="mx-1 h-4 w-px bg-ms-border" />

    <!-- 过滤 -->
    <!-- consistent-menu-width=false：菜单按内容自适应宽度，避免长平台名被截断 -->
    <NSelect
      v-model:value="store.filterPlatform"
      :options="store.platformOptions"
      size="tiny"
      clearable
      placeholder="全部平台"
      style="width: 132px"
      :consistent-menu-width="false"
    />
    <NSelect
      v-model:value="store.filterStatus"
      :options="STATUS_OPTIONS"
      size="tiny"
      multiple
      clearable
      max-tag-count="responsive"
      placeholder="全部状态"
      style="width: 190px"
      :consistent-menu-width="false"
    />
    <NInput
      v-model:value="store.keyword"
      size="tiny"
      clearable
      placeholder="搜索备注 / 脱敏 Key"
      style="width: 168px"
    />
    <div class="flex items-center gap-1">
      <span class="text-[10px] text-ms-text-faint">余额阈值</span>
      <NInputNumber
        v-model:value="store.lowBalanceThreshold"
        size="tiny"
        :min="0"
        :show-button="false"
        style="width: 84px"
      />
    </div>

    <div class="flex-1" />

    <!-- 检测进度 -->
    <div v-if="store.checking" class="flex items-center gap-2 text-[11px] text-ms-text-dim">
      <NTag size="tiny" :bordered="false" type="info">
        {{ store.progress.completed }}/{{ store.progress.total }}
      </NTag>
      <span class="max-w-[220px] truncate font-mono text-[10px] text-ms-text-faint">
        {{ store.progress.currentMaskedKey }} · {{ store.progress.stageLabel }}
      </span>
    </div>
  </div>
</template>
