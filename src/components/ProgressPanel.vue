<script setup lang="ts">
/**
 * 进度与日志面板（左栏）：总进度条、分阶段进度气泡、
 * 每个模型的实时阶段指示（由进度事件驱动）、滚动日志区（按级别着色）。
 *
 * 性能考虑：日志可能高频推送（每个模型的每个阶段都会产生若干条）。
 * 在 store 层做 600 行上限裁剪，渲染层直接渲染（600 行 DOM 完全可控）；
 * 若日志量级上升到数万行，可替换为 NVirtualList。
 */

import { computed, nextTick, ref, watch } from 'vue'
import { NTag, NTooltip, NEmpty } from 'naive-ui'

import { useProbe } from '@/composables/useProbe'
import { formatTokens } from '@/composables/useProbe'
import { useScrollSafeTooltip } from '@/composables/useScrollSafeTooltip'
import type { ModelInfo, ProbeStage } from '@/types/contracts'

const props = defineProps<{
  probe: ReturnType<typeof useProbe>
}>()

/** 是否自动滚动到底部（用户手动上滚时暂停自动滚动） */
const autoScroll = ref(true)

/**
 * 模型行能力摘要的浮层守卫。
 *
 * 手动控制显隐而非 trigger="hover" 的原因：模型列表可滚动，
 * 而 Naive 的浮层 teleport 到 body 按坐标定位，滚动过程中浮层
 * 会跟着行滚出列表容器。此外滚动会让新行滑到光标下并补发
 * mouseenter，令浮层「收起→重开」反复竞速，既会跳出面板边缘，
 * 又会残留透明但拦截指针的僵尸浮层（详见 useScrollSafeTooltip）。
 * 守卫在滚动时收起浮层并锁定悬停，直到鼠标真正移动才恢复。
 */
const {
  openKey: tooltipModelId,
  closeOnScroll,
  unlockHover,
  enter: enterTooltip,
  leave: leaveTooltip,
} = useScrollSafeTooltip()

/** 日志容器的 DOM 引用 */
const logContainer = ref<HTMLElement | null>(null)

/** 按完成顺序排列的模型（供阶段指示器渲染） */
const modelRows = computed(() => props.probe.store.models)

/**
 * 正在进行中的模型（未出现在 models 里但已有阶段记录的）。
 *
 * `models` 只在模型完全探测完成时才追加；探测中只能从
 * `stageByModel` 得知某个 ID 正在处理。合并两者展示，
 * 用户才能看到实时进展。
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
 * 用 nextTick 等待 DOM 更新后再滚动，否则 scrollHeight 是旧值，
 * 会出现差一行的问题。
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
  // 日志区滚动时同样收起模型行的浮层并锁定悬停，机制见浮层守卫注释
  closeOnScroll()

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
  <aside
    class="ms-panel flex min-h-0 flex-col overflow-hidden"
    @mousemove="unlockHover"
  >
    <!-- ---- 头部统计 ---- -->
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
    <!--
      @scroll 经浮层守卫收起浮层并锁定悬停：滚动补发的 mouseenter
      不含 mousemove，被锁挡住，浮层不会在滚动中反复重开、跳出面板
      边缘或残留僵尸浮层。机制详见脚本里的浮层守卫注释。
    -->
    <div
      class="max-h-[42%] shrink-0 overflow-y-auto border-b border-ms-border px-3 py-2"
      @scroll="closeOnScroll"
    >
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
        trigger="manual"
        :show="tooltipModelId === model.id"
        style="max-width: 360px"
      >
        <template #trigger>
          <div
            class="ms-fade-in flex cursor-help items-center justify-between gap-2 py-1"
            @mouseenter="enterTooltip(model.id)"
            @mouseleave="leaveTooltip()"
          >
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
