<script setup lang="ts">
/**
 * ============================================================================
 * ProgressPanel —— 进度与日志面板（左栏）
 * ============================================================================
 *
 * 【职责】
 *   1. 总进度条 + 分阶段进度气泡
 *   2. 每个模型的实时阶段指示（由 probe://progress 事件驱动）
 *   3. 滚动日志区（由 probe://log 事件驱动），按级别着色
 *
 * 【性能考虑】
 * 日志可能高频推送（每个模型的每个阶段都会产生若干条）。
 * 因此这里用 `v-for` + `key` 的虚拟滚动替代方案：
 * 只在 store 层做 600 行上限裁剪，渲染层直接渲染（600 行 DOM 完全可控）。
 * 若未来日志量级上升到数万行，可替换为 NVirtualList。
 * ============================================================================
 */

import { computed, nextTick, ref, watch } from 'vue'
import { NTag, NTooltip, NEmpty } from 'naive-ui'

import { useProbe } from '@/composables/useProbe'
import { formatTokens } from '@/composables/useProbe'
import type { ModelInfo, ProbeStage } from '@/types/contracts'

const props = defineProps<{
  probe: ReturnType<typeof useProbe>
}>()

/** 是否自动滚动到底部（用户手动上滚时暂停自动滚动） */
const autoScroll = ref(true)

/** 日志容器的 DOM 引用 */
const logContainer = ref<HTMLElement | null>(null)

/** 按完成顺序排列的模型（供阶段指示器渲染） */
const modelRows = computed(() => props.probe.store.models)

/**
 * 正在进行中的模型（还没出现在 models 里但已有阶段记录的）。
 *
 * 【为什么需要这个】
 * `models` 只在模型**完全探测完**时才被追加。
 * 探测中我们只能从 `stageByModel` 得知「某个 ID 正在处理」。
 * 把这两者合并展示，用户才能看到实时进展而不是一片空白。
 */
const inflight = computed(() => {
  const done = new Set(modelRows.value.map((m: ModelInfo) => m.id))
  const stageMap = props.probe.store.stageByModel
  return Object.keys(stageMap)
    .filter((id) => !done.has(id))
    .map((id) => ({ id, stage: stageMap[id] as ProbeStage }))
})

/** 阶段 → 中文标签 */
const STAGE_LABELS: Record<string, string> = {
  pending: '排队',
  listing: '列表',
  toolProbe: '工具',
  visionProbe: '附件',
  reasoningProbe: '推理',
  limitProbe: '上限',
  done: '完成',
  failed: '失败',
}

/** 阶段 → 标签颜色类型 */
const STAGE_TYPES: Record<string, 'default' | 'info' | 'success' | 'warning' | 'error'> = {
  pending: 'default',
  listing: 'default',
  toolProbe: 'info',
  visionProbe: 'info',
  reasoningProbe: 'info',
  limitProbe: 'info',
  done: 'success',
  failed: 'error',
}

/** 日志级别 → 颜色类 */
const LOG_LEVEL_CLASS: Record<string, string> = {
  debug: 'text-ms-text-faint',
  info: 'text-ms-text-dim',
  warn: 'text-ms-warning',
  error: 'text-ms-danger',
}

/** 日志级别 → 前缀符号 */
const LOG_LEVEL_MARK: Record<string, string> = {
  debug: '·',
  info: '›',
  warn: '!',
  error: '✕',
}

/**
 * 日志滚动到底部。
 *
 * 用 nextTick 等待 DOM 更新后再滚动，否则 scrollHeight 还是旧值，
 * 会出现「差一行」的经典问题。
 */
async function scrollToBottom(): Promise<void> {
  if (!autoScroll.value) return
  await nextTick()
  const el = logContainer.value
  if (el) {
    el.scrollTop = el.scrollHeight
  }
}

/** 日志条数变化时自动滚动 */
watch(
  () => props.probe.store.logs.length,
  () => {
    void scrollToBottom()
  },
)

/** 用户滚动时更新自动滚动开关 */
function handleScroll(): void {
  const el = logContainer.value
  if (!el) return
  // 距底部 24px 以内视为「贴着底」，此时才继续自动滚动
  const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 24
  autoScroll.value = atBottom
}

/** 某模型的能力摘要文案（用于完成行的 tooltip） */
function summarizeModel(model: ModelInfo): string {
  const parts: string[] = []
  parts.push(`工具调用：${model.supportsTools === 'supported' ? '支持' : model.supportsTools === 'unsupported' ? '不支持' : '未知'}`)
  parts.push(`附件上传：${model.supportsVision === 'supported' ? '支持' : model.supportsVision === 'unsupported' ? '不支持' : '未知'}`)
  parts.push(`推理模型：${model.isReasoning ? '是' : '否'}`)
  parts.push(`上下文：${formatTokens(model.contextLimit)}`)
  parts.push(`输出上限：${formatTokens(model.maxOutputLimit)}`)
  return parts.join(' · ')
}
</script>

<template>
  <aside class="ms-panel flex min-h-0 flex-col overflow-hidden">
    <!-- ---------- 头部统计 ---------- -->
    <div class="border-b border-ms-border px-3 py-2.5">
      <div class="flex items-center justify-between">
        <span class="ms-panel-title">实时进度</span>
        <span class="font-mono text-[11px] text-ms-text-faint">
          {{ probe.store.completed }}/{{ probe.store.total }}
        </span>
      </div>

      <!-- 能力统计小卡片 -->
      <div
        v-if="probe.store.models.length > 0"
        class="mt-2 grid grid-cols-3 gap-1.5"
      >
        <div class="rounded bg-ms-panel-2 px-2 py-1.5">
          <div class="text-[10px] text-ms-text-faint">支持工具</div>
          <div class="font-mono text-sm text-ms-accent">
            {{ probe.store.capabilityStats.tools }}
          </div>
        </div>
        <div class="rounded bg-ms-panel-2 px-2 py-1.5">
          <div class="text-[10px] text-ms-text-faint">支持附件</div>
          <div class="font-mono text-sm text-ms-purple">
            {{ probe.store.capabilityStats.vision }}
          </div>
        </div>
        <div class="rounded bg-ms-panel-2 px-2 py-1.5">
          <div class="text-[10px] text-ms-text-faint">推理模型</div>
          <div class="font-mono text-sm text-ms-info">
            {{ probe.store.capabilityStats.reasoning }}
          </div>
        </div>
      </div>
    </div>

    <!-- ---------- 模型阶段列表 ---------- -->
    <div class="max-h-[42%] shrink-0 overflow-y-auto border-b border-ms-border px-3 py-2">
      <!-- 空态 -->
      <div
        v-if="modelRows.length === 0 && inflight.length === 0"
        class="py-6 text-center text-[11px] text-ms-text-faint"
      >
        尚未开始
      </div>

      <!-- 探测中的模型 -->
      <div
        v-for="item in inflight"
        :key="`flight-${item.id}`"
        class="ms-fade-in flex items-center justify-between gap-2 py-1"
      >
        <span class="min-w-0 flex-1 truncate font-mono text-[11px] text-ms-text-dim">
          {{ item.id }}
        </span>
        <NTag size="tiny" :type="STAGE_TYPES[item.stage] ?? 'info'" :bordered="false">
          <span class="ms-pulse">{{ STAGE_LABELS[item.stage] ?? item.stage }}</span>
        </NTag>
      </div>

      <!-- 已完成的模型 -->
      <NTooltip
        v-for="model in modelRows"
        :key="`done-${model.id}`"
        trigger="hover"
        style="max-width: 360px"
      >
        <template #trigger>
          <div class="ms-fade-in flex items-center justify-between gap-2 py-1">
            <span class="min-w-0 flex-1 truncate font-mono text-[11px] text-ms-text">
              {{ model.id }}
            </span>
            <div class="flex shrink-0 items-center gap-1">
              <span
                v-if="model.supportsTools === 'supported'"
                class="text-[10px] text-ms-accent"
                title="支持工具调用"
              >
                T
              </span>
              <span
                v-if="model.supportsVision === 'supported'"
                class="text-[10px] text-ms-purple"
                title="支持附件上传"
              >
                V
              </span>
              <span
                v-if="model.isReasoning"
                class="text-[10px] text-ms-info"
                title="推理模型"
              >
                R
              </span>
              <NTag
                size="tiny"
                :type="model.finalStage === 'done' ? 'success' : 'error'"
                :bordered="false"
              >
                {{ model.finalStage === 'done' ? '完成' : '有误' }}
              </NTag>
            </div>
          </div>
        </template>
        {{ summarizeModel(model) }}
      </NTooltip>
    </div>

    <!-- ---------- 日志区 ---------- -->
    <div class="flex min-h-0 flex-1 flex-col">
      <div class="flex items-center justify-between border-b border-ms-border px-3 py-1.5">
        <span class="ms-panel-title">运行日志</span>
        <div class="flex items-center gap-2">
          <span class="text-[10px] text-ms-text-faint">
            {{ probe.store.logs.length }} 行
          </span>
          <NTag
            size="tiny"
            :bordered="false"
            :type="autoScroll ? 'success' : 'default'"
            class="cursor-pointer"
            @click="autoScroll = !autoScroll"
          >
            {{ autoScroll ? '自动滚动' : '已暂停' }}
          </NTag>
        </div>
      </div>

      <div
        ref="logContainer"
        class="ms-code min-h-0 flex-1 overflow-y-auto px-3 py-2"
        @scroll="handleScroll"
      >
        <NEmpty
          v-if="probe.store.logs.length === 0"
          description="等待日志输出"
          class="py-8"
        />

        <div
          v-for="(line, index) in probe.store.logs"
          :key="`${line.timestamp}-${index}`"
          class="ms-fade-in flex gap-1.5 leading-relaxed"
        >
          <span
            class="shrink-0 select-none"
            :class="LOG_LEVEL_CLASS[line.level]"
          >
            {{ LOG_LEVEL_MARK[line.level] }}
          </span>
          <span
            v-if="line.modelId"
            class="shrink-0 max-w-[130px] truncate text-ms-accent-dim"
          >
            [{{ line.modelId }}]
          </span>
          <span class="min-w-0 flex-1 break-words" :class="LOG_LEVEL_CLASS[line.level]">
            {{ line.message }}
          </span>
        </div>
      </div>
    </div>
  </aside>
</template>
