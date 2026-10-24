<script setup lang="ts">
/**
 * 顶部标题栏：展示应用标识与版本、全局运行状态指示灯，
 * 以及窗口拖拽区（data-tauri-drag-region）。
 *
 * 窗口拖拽需要在 capabilities/default.json 中授权
 * `core:window:allow-start-dragging`，已在权限文件中配置。
 */

import { computed, onMounted, ref } from 'vue'
import { NTag, NTooltip } from 'naive-ui'
import { getVersion } from '@tauri-apps/api/app'

import { getDictionaryInfo } from '@/api/commands'

/** 应用版本（来自 tauri.conf.json，由 Tauri 运行时提供） */
const version = ref('')

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
      return { color: 'var(--color-ms-accent)', label: '运行中', pulse: true }
    case 'cancelling':
      return { color: 'var(--color-ms-warning)', label: '正在中止', pulse: true }
    case 'done':
      return { color: 'var(--color-ms-success)', label: '已完成', pulse: false }
    case 'cancelled':
      return { color: 'var(--color-ms-text-dim)', label: '已中止', pulse: false }
    case 'failed':
      return { color: 'var(--color-ms-danger)', label: '失败', pulse: false }
    default:
      return { color: 'var(--color-ms-text-faint)', label: '空闲', pulse: false }
  }
})

const props = defineProps<{
  /** 当前探测状态（由 App.vue 传入） */
  status?: string
}>()

const emit = defineEmits<{
  /** 打开「关于」弹窗（由 App.vue 监听） */
  (e: 'open-about'): void
  /** 打开「设置」弹窗（主题切换等，由 App.vue 监听） */
  (e: 'open-settings'): void
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

      <!-- 设置入口（主题切换等） -->
      <NTooltip trigger="hover">
        <template #trigger>
          <button
            class="flex h-7 w-7 items-center justify-center rounded-md text-ms-text-dim transition-colors hover:bg-ms-accent/15 hover:text-ms-accent"
            aria-label="设置"
            @click="emit('open-settings')"
          >
            <svg viewBox="0 0 16 16" width="15" height="15" fill="currentColor" aria-hidden="true">
              <path
                d="M8 4.754a3.246 3.246 0 1 0 0 6.492 3.246 3.246 0 0 0 0-6.492zm0 5.092a1.846 1.846 0 1 1 0-3.692 1.846 1.846 0 0 1 0 3.692z"
              />
              <path
                d="M9.796 1.343c-.527-1.79-3.065-1.79-3.592 0l-.094.319a.873.873 0 0 1-1.255.52l-.292-.16c-1.64-.892-3.433.902-2.54 2.541l.159.292a.873.873 0 0 1-.52 1.255l-.319.094c-1.79.527-1.79 3.065 0 3.592l.319.094a.873.873 0 0 1 .52 1.255l-.16.292c-.892 1.64.901 3.434 2.541 2.54l.292-.159a.873.873 0 0 1 1.255.52l.094.319c.527 1.79 3.065 1.79 3.592 0l.094-.319a.873.873 0 0 1 1.255-.52l.292.16c1.64.893 3.434-.902 2.54-2.541l-.159-.292a.873.873 0 0 1 .52-1.255l.319-.094c1.79-.527 1.79-3.065 0-3.592l-.319-.094a.873.873 0 0 1-.52-1.255l.16-.292c.893-1.64-.902-3.433-2.541-2.54l-.292.159a.873.873 0 0 1-1.255-.52l-.094-.319zm-2.633.283c.246-.835 1.428-.835 1.674 0l.094.319a1.873 1.873 0 0 0 2.693 1.115l.291-.16c.764-.415 1.6.42 1.184 1.185l-.159.292a1.873 1.873 0 0 0 1.116 2.692l.318.094c.835.246.835 1.428 0 1.674l-.319.094a1.873 1.873 0 0 0-1.115 2.693l.16.291c.415.764-.42 1.6-1.185 1.184l-.291-.159a1.873 1.873 0 0 0-2.693 1.116l-.094.318c-.246.835-1.428.835-1.674 0l-.094-.319a1.873 1.873 0 0 0-2.692-1.115l-.292.16c-.764.415-1.6-.421-1.184-1.185l.159-.291A1.873 1.873 0 0 0 1.945 8.93l-.319-.094c-.835-.246-.835-1.428 0-1.674l.319-.094a1.873 1.873 0 0 0 1.115-2.692l-.16-.292c-.415-.764.42-1.601 1.185-1.184l.292.159a1.873 1.873 0 0 0 2.692-1.116l.094-.318z"
              />
            </svg>
          </button>
        </template>
        设置（主题切换）
      </NTooltip>

      <!-- 关于入口 -->
      <NTooltip trigger="hover">
        <template #trigger>
          <button
            class="flex h-7 w-7 items-center justify-center rounded-md text-ms-text-dim transition-colors hover:bg-ms-accent/15 hover:text-ms-accent"
            aria-label="关于 ModelScout"
            @click="emit('open-about')"
          >
            <svg
              viewBox="0 0 16 16"
              width="15"
              height="15"
              fill="currentColor"
              aria-hidden="true"
            >
              <path
                d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0 0 16 8c0-4.42-3.58-8-8-8z"
              />
            </svg>
          </button>
        </template>
        关于 ModelScout
      </NTooltip>
    </div>
  </header>
</template>
