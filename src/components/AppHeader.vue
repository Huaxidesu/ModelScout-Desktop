<script setup lang="ts">
/**
 * ============================================================================
 * AppHeader —— 顶部标题栏
 * ============================================================================
 *
 * 【职责】
 *   1. 展示应用标识与版本
 *   2. 展示全局运行状态指示灯
 *   3. 提供数据区（Tauri 窗口拖拽）—— 用 data-tauri-drag-region 实现
 *
 * 【权限说明】
 * 窗口拖拽需要 capabilities/default.json 中授权
 * `core:window:allow-start-dragging`，已在权限文件中配置。
 * ============================================================================
 */

import { computed, onMounted, ref } from 'vue'
import { NTag, NTooltip } from 'naive-ui'
import { getVersion } from '@tauri-apps/api/app'

import { getDictionaryInfo } from '@/api/commands'

/** 应用版本（来自 tauri.conf.json，由 Tauri 运行时提供） */
const version = ref('0.1.0')

/** 内置字典条目数，用于展示「数据新鲜度」 */
const dictCount = ref<number | null>(null)

onMounted(async () => {
  try {
    version.value = await getVersion()
  } catch {
    // 在浏览器中裸跑前端时 Tauri API 不可用，保留默认值即可
  }
  try {
    const info = await getDictionaryInfo()
    dictCount.value = info.entryCount
  } catch {
    dictCount.value = null
  }
})

/**
 * 状态指示灯的颜色与文案。
 * 采用 computed 而非模板内三元，便于维护。
 */
const statusMeta = computed(() => {
  const status = props.status
  switch (status) {
    case 'running':
      return { color: '#14b8a6', label: '运行中', pulse: true }
    case 'cancelling':
      return { color: '#f59e0b', label: '正在中止', pulse: true }
    case 'done':
      return { color: '#22c55e', label: '已完成', pulse: false }
    case 'cancelled':
      return { color: '#8b98ad', label: '已中止', pulse: false }
    case 'failed':
      return { color: '#ef4444', label: '失败', pulse: false }
    default:
      return { color: '#5c6b82', label: '空闲', pulse: false }
  }
})

const props = defineProps<{
  /** 当前探测状态（由 App.vue 传入） */
  status?: string
}>()
</script>

<template>
  <header
    class="flex items-center justify-between border-b border-ms-border bg-ms-panel px-4 py-2.5"
    data-tauri-drag-region
  >
    <!-- 左侧：Logo 与名称 -->
    <div class="flex items-center gap-3" data-tauri-drag-region>
      <div
        class="flex h-7 w-7 items-center justify-center rounded-md bg-ms-accent/15 text-base font-bold text-ms-accent"
      >
        ◈
      </div>
      <div class="flex items-baseline gap-2" data-tauri-drag-region>
        <h1 class="text-[15px] font-semibold tracking-wide text-ms-text">
          ModelScout
        </h1>
        <span class="text-[11px] text-ms-text-faint">大模型 API 探测器</span>
      </div>
      <NTag size="tiny" :bordered="false" class="ml-1">
        v{{ version }}
      </NTag>
    </div>

    <!-- 右侧：状态指示 + 字典信息 -->
    <div class="flex items-center gap-4" data-tauri-drag-region>
      <NTooltip v-if="dictCount !== null" trigger="hover">
        <template #trigger>
          <span class="cursor-help text-[11px] text-ms-text-faint">
            参数字典 {{ dictCount }} 条
          </span>
        </template>
        内置模型参数字典的条目数量。命中字典的模型会直接采用其上下文/输出上限，
        无需发起探测请求。可在应用资源目录中编辑 model_params.json 扩充。
      </NTooltip>

      <div class="flex items-center gap-2">
        <span
          class="h-2 w-2 rounded-full"
          :class="{ 'ms-pulse': statusMeta.pulse }"
          :style="{ backgroundColor: statusMeta.color }"
        />
        <span class="text-[11px] text-ms-text-dim">{{ statusMeta.label }}</span>
      </div>
    </div>
  </header>
</template>
