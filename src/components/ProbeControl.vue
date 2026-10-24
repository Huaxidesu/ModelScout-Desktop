<script setup lang="ts">
/**
 * 探测控制条：提供开始 / 停止 / 重置三个主操作，
 * 展示整体进度摘要（百分比、完成数、耗时、当前阶段），
 * 以及错误分类汇总卡片。
 *
 * 错误分类展示的意义：看到「Key 无效」与看到「HTTP 401」的排障效率不同，
 * 此处把后端 `ProbeErrorKind` 的分类结果翻译为中文标签并附排障建议。
 */

import { computed } from 'vue'
import { NButton, NProgress, NTag, NTooltip, NStatistic } from 'naive-ui'

import { useProbe, ERROR_KIND_LABELS, ERROR_KIND_TYPES } from '@/composables/useProbe'
import { formatDuration } from '@/composables/useProbe'

const props = defineProps<{
  probe: ReturnType<typeof useProbe>
}>()

/** 是否正在运行 */
const running = computed(() => props.probe.store.isRunning)

/** 是否有可重置的内容 */
const hasContent = computed(
  () => props.probe.store.hasRun || props.probe.store.models.length > 0,
)

/** 当前运行阶段的中文描述（取最后一个有阶段记录的模型） */
const currentPhase = computed(() => {
  const stageMap = props.probe.store.stageByModel
  const keys = Object.keys(stageMap)
  if (keys.length === 0) return '准备中'

  // 找一个「尚未 done」的模型作为当前处理对象
  for (const key of keys) {
    const stage = stageMap[key]
    if (stage && stage !== 'done' && stage !== 'failed' && stage !== 'pending') {
      return `${key} · ${STAGE_LABELS[stage] ?? stage}`
    }
  }
  return `已完成 ${props.probe.store.completed}/${props.probe.store.total}`
})

/** 阶段 → 中文 */
const STAGE_LABELS: Record<string, string> = {
  pending: '排队等待',
  listing: '拉取模型列表',
  toolProbe: '探测工具调用',
  visionProbe: '探测附件上传',
  reasoningProbe: '探测推理能力',
  limitProbe: '探测上下文上限',
  done: '完成',
  failed: '失败',
}

/**
 * 每个错误分类对应的排障建议。
 * 本组件的核心价值：不仅指出错误类别，还给出对应的处理方式。
 */
const ERROR_ADVICE: Record<string, string> = {
  keyInvalid: '请检查 API Key 是否正确、是否已过期，以及该 Key 是否有权限访问此端点。',
  baseUrlInvalid: '请检查 Base URL 是否完整（通常以 /v1 结尾），以及主机名与端口是否正确。',
  networkTimeout: '请检查网络连通性、代理设置，或适当调大超时时间。',
  rateLimited: '请求过于频繁。请把并发数降到 1-2 后重试。',
  serverError: '上游服务端故障，与本机配置无关，建议稍后重试。',
  modelNotFound: '该模型对当前 Key 不可用，可能需要在服务商控制台开通。',
  parseError: '服务端返回的不是标准 OpenAI 兼容响应，请确认 Base URL 指向正确的端点。',
  cancelled: '任务被主动中止，非错误。',
  unknown: '请查看右侧日志获取更多细节。',
}
</script>

<template>
  <section class="ms-panel px-4 py-3">
    <div class="flex items-center gap-4">
      <!-- ---------- 左：主操作按钮 ---------- -->
      <div class="flex items-center gap-2">
        <NButton
          type="primary"
          size="medium"
          :disabled="running"
          :loading="running && !probe.stopping.value"
          @click="probe.start()"
        >
          {{ running ? '探测进行中…' : '▶ 开始探测' }}
        </NButton>

        <NButton
          type="error"
          ghost
          size="medium"
          :disabled="!running || probe.stopping.value"
          :loading="probe.stopping.value"
          @click="probe.stop()"
        >
          ■ 停止探测
        </NButton>

        <NButton
          size="medium"
          quaternary
          :disabled="!hasContent || running"
          @click="probe.store.resetRun()"
        >
          重置
        </NButton>
      </div>

      <!-- ---------- 中：进度 ---------- -->
      <div class="min-w-0 flex-1">
        <div class="mb-1 flex items-center justify-between text-[11px]">
          <span class="truncate text-ms-text-dim">{{ currentPhase }}</span>
          <span class="shrink-0 font-mono text-ms-text-faint">
            {{ probe.store.percent }}%
          </span>
        </div>
        <NProgress
          type="line"
          :percentage="probe.store.percent"
          :show-indicator="false"
          :height="6"
          :border-radius="3"
          :color="running ? 'var(--color-ms-accent)' : 'var(--color-ms-success)'"
          rail-color="var(--color-ms-elevated)"
        />
      </div>

      <!-- ---------- 右：关键指标 ---------- -->
      <div class="flex shrink-0 items-center gap-5">
        <NStatistic label="完成" :value="`${probe.store.completed}/${probe.store.total}`" />
        <NStatistic label="耗时" :value="formatDuration(probe.store.elapsedMs)" />
        <NStatistic label="并发" :value="String(probe.store.config.concurrency)" />
      </div>
    </div>

    <!-- ---------- 错误分类汇总 ---------- -->
    <div
      v-if="probe.store.errorSummary.length > 0"
      class="mt-3 border-t border-ms-border pt-3"
    >
      <div class="mb-2 flex items-center gap-2">
        <span class="ms-panel-title">错误分类</span>
        <span class="text-[11px] text-ms-text-faint">
          共 {{ probe.store.errorSummary.reduce((sum: number, e: { count: number }) => sum + e.count, 0) }} 条
        </span>
      </div>

      <div class="flex flex-wrap gap-2">
        <NTooltip
          v-for="item in probe.store.errorSummary"
          :key="item.kind"
          trigger="hover"
          style="max-width: 320px"
        >
          <template #trigger>
            <NTag
              :type="ERROR_KIND_TYPES[item.kind] ?? 'default'"
              size="small"
              :bordered="false"
              class="cursor-help"
            >
              {{ ERROR_KIND_LABELS[item.kind] ?? item.kind }}
              <span class="ml-1 font-mono font-semibold">{{ item.count }}</span>
            </NTag>
          </template>
          {{ ERROR_ADVICE[item.kind] ?? '请查看日志获取详情。' }}
        </NTooltip>
      </div>
    </div>
  </section>
</template>
