<script setup lang="ts">
/**
 * Key 管家余额历史抽屉：右侧抽屉，内含「时间-余额」折线图与快照列表。
 *
 * 图表库选型：Chart.js 按需注册（而非 `chart.js/auto`），
 * 避免把用不到的图表类型打进包里；不引入全量 ECharts。
 *
 * 边界处理：
 *   - 快照为空：不建图，显示占位文案；
 *   - 只有一个点：Chart.js 画不出线段，此时把点画大并显示占位说明；
 *   - 余额为 null 的快照不参与画线（会被跳过），但仍在列表里如实展示。
 */

import { computed, nextTick, onUnmounted, ref, watch } from 'vue'
// 只引类型（编译期擦除，不进包）；运行时用动态 import 按需加载
import type { Chart as ChartInstance } from 'chart.js'
import { NDrawer, NDrawerContent, NEmpty, NTag } from 'naive-ui'

import type { KeyEntry, Snapshot } from '@/types/contracts'
import { useKeyButlerStore } from '@/stores/keyButler'
import { useThemeStore } from '@/stores/theme'
import { ProbeApiError } from '@/api/commands'

const props = defineProps<{
  show: boolean
  entry: KeyEntry | null
}>()
const emit = defineEmits<{ (e: 'update:show', value: boolean): void }>()

const store = useKeyButlerStore()

/** 主题令牌:canvas 图表拿不到 CSS 变量,取已解析的令牌值(抽屉打开时读取) */
const theme = useThemeStore()

const snapshots = ref<Snapshot[]>([])
const loading = ref(false)
const errorText = ref('')

const canvasRef = ref<HTMLCanvasElement | null>(null)
/** 当前 Chart 实例；销毁时必须显式 destroy，否则会泄漏 canvas 上下文 */
let chart: ChartInstance | null = null

/** 有余额数值的快照（画线只用这些） */
const plotted = computed(() => snapshots.value.filter((item) => item.balance !== null))

/** 时间戳 → `MM-DD HH:mm` */
function formatTime(ts: number): string {
  const date = new Date(ts)
  const pad = (value: number) => String(value).padStart(2, '0')
  return `${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}`
}

/** 销毁图表实例 */
function destroyChart(): void {
  if (chart) {
    chart.destroy()
    chart = null
  }
}

/**
 * 建图。
 *
 * 【为什么用动态 import】
 * chart.js 压缩后约 200KB（gzip 70KB+），而余额历史是低频功能。
 * 静态引入会让每个打开应用的人都为它付下载与解析成本；
 * 动态 import 让它单独成 chunk，只在真正打开抽屉时加载。
 * 顺带解决了「主包超过 500KB」的构建告警。
 */
async function buildChart(): Promise<void> {
  destroyChart()
  const canvas = canvasRef.value
  if (!canvas) return

  const points = plotted.value
  if (points.length === 0) return

  // 按需注册：只挂折线图需要的组件，不引 chart.js/auto 的全量
  const chartjs = await import('chart.js')
  chartjs.Chart.register(
    chartjs.LineController,
    chartjs.LineElement,
    chartjs.PointElement,
    chartjs.LinearScale,
    chartjs.CategoryScale,
    chartjs.Tooltip,
    chartjs.Filler,
    chartjs.Legend,
  )

  // 动态加载期间组件可能已被卸载 / 抽屉已关闭
  if (!canvasRef.value) return

  chart = new chartjs.Chart(canvas, {
    type: 'line',
    data: {
      labels: points.map((item) => formatTime(item.ts)),
      datasets: [
        {
          label: '余额',
          data: points.map((item) => item.balance as number),
          borderColor: theme.colors.accent,
          backgroundColor: theme.colors.accentGlow,
          fill: true,
          tension: 0.25,
          // 单点时没有线段可画，把点放大一点，至少能看见
          pointRadius: points.length === 1 ? 5 : 3,
          pointBackgroundColor: theme.colors.accent,
        },
      ],
    },
    options: {
      responsive: true,
      maintainAspectRatio: false,
      plugins: {
        legend: { display: false },
        tooltip: { intersect: false, mode: 'index' },
      },
      scales: {
        x: {
          ticks: { color: theme.colors.textDim, maxRotation: 0, autoSkipPadding: 12, font: { size: 10 } },
          grid: { color: theme.colors.border },
        },
        y: {
          ticks: { color: theme.colors.textDim, font: { size: 10 } },
          grid: { color: theme.colors.border },
        },
      },
    },
  })
}

/** 载入快照并建图 */
async function load(): Promise<void> {
  const entry = props.entry
  if (!entry) return
  loading.value = true
  errorText.value = ''
  try {
    snapshots.value = await store.loadSnapshots(entry.id)
    await nextTick()
    await buildChart()
  } catch (error) {
    errorText.value = error instanceof ProbeApiError ? error.message : String(error)
    snapshots.value = []
  } finally {
    loading.value = false
  }
}

watch(
  () => [props.show, props.entry?.id] as const,
  ([show]) => {
    if (show) {
      void load()
    } else {
      destroyChart()
    }
  },
  { immediate: true },
)

onUnmounted(destroyChart)

/** 快照的状态标签颜色 */
function statusType(status: string): 'success' | 'warning' | 'error' | 'default' {
  if (status === 'active') return 'success'
  if (status === 'insufficient') return 'warning'
  if (status === 'invalid') return 'error'
  return 'default'
}
</script>

<template>
  <NDrawer :show="show" :width="520" placement="right" @update:show="emit('update:show', $event)">
    <NDrawerContent closable :title="`余额历史 · ${entry?.maskedKey ?? ''}`">
      <div v-if="errorText" class="mb-2 text-[11px] text-ms-danger">{{ errorText }}</div>

      <!-- 图表区 -->
      <div class="mb-3 h-[220px] rounded border border-ms-border bg-ms-bg/40 p-2">
        <div v-if="loading" class="flex h-full items-center justify-center text-[11px] text-ms-text-faint">
          载入中…
        </div>
        <div
          v-else-if="plotted.length === 0"
          class="flex h-full flex-col items-center justify-center gap-2 text-center text-[11px] leading-relaxed text-ms-text-faint"
        >
          <span>还没有可用的余额记录。</span>
          <span>余额查询成功后会在这里画出曲线（没有公开余额接口的平台不会有数据）。</span>
          <span v-if="!store.vault.enabled" class="text-ms-warning">
            未启用「本地保存」时，历史只保留在本次会话内 —— 开启后才会写入本机。
          </span>
        </div>
        <div v-else class="relative h-full">
          <canvas ref="canvasRef" />
          <div
            v-if="plotted.length === 1"
            class="mt-1 text-center text-[10px] text-ms-text-faint"
          >
            只有一个数据点，多检测几次就能看到趋势。
          </div>
        </div>
      </div>

      <!-- 快照列表 -->
      <div class="mb-2 flex items-center gap-2">
        <span class="text-[12px] font-semibold text-ms-text-dim">快照记录</span>
        <NTag size="tiny" :bordered="false">{{ snapshots.length }}</NTag>
      </div>

      <NEmpty v-if="snapshots.length === 0" description="暂无快照" class="py-6" />

      <div v-else class="flex flex-col">
        <div
          v-for="item in [...snapshots].reverse()"
          :key="item.ts"
          class="flex items-center justify-between border-b border-ms-border/60 py-1.5 text-[11px] last:border-b-0"
        >
          <span class="text-ms-text-faint">{{ formatTime(item.ts) }}</span>
          <span class="font-mono text-ms-text-dim">
            {{ item.balance === null ? '—' : `$${item.balance.toFixed(2)}` }}
          </span>
          <NTag size="tiny" :bordered="false" :type="statusType(item.status)">
            {{ item.status }}
          </NTag>
        </div>
      </div>
    </NDrawerContent>
  </NDrawer>
</template>
