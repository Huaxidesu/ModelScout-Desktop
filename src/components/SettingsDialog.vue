<script setup lang="ts">
/**
 * 设置弹窗:主题切换(默认 / 浅色 / 深色 / GitHub / 自定义)。
 *
 * 选中的主题立即生效并持久化(stores/theme.ts);「自定义」展开全部 20 个
 * 颜色令牌的取色器,初始值取自「默认」配色,可整体一键恢复。
 * 由 AppHeader 的齿轮入口经 v-model:show 控制显隐。
 */

import { computed } from 'vue'
import { NButton, NColorPicker, NModal } from 'naive-ui'

import { PRESETS, tokenAllowsAlpha, type ThemeColors, type ThemeId } from '@/theme/presets'
import { useThemeStore } from '@/stores/theme'

const props = defineProps<{
  show: boolean
}>()

const emit = defineEmits<{
  (e: 'update:show', value: boolean): void
}>()

const theme = useThemeStore()

/** 主题选择卡片的数据(预设 + 自定义) */
interface ThemeOption {
  id: ThemeId
  label: string
  description: string
  /** 预览色点:背景 / 面板 / 文字 / 强调色 */
  swatches: string[]
  schemeLabel: string
}

const options = computed<ThemeOption[]>(() => [
  ...PRESETS.map((p) => ({
    id: p.id,
    label: p.label,
    description: p.description,
    swatches: [p.colors.bg, p.colors.panel, p.colors.text, p.colors.accent],
    schemeLabel: p.scheme === 'dark' ? '暗色' : '亮色',
  })),
  {
    id: 'custom',
    label: '自定义',
    description: '所有颜色都可改,初始值取自「默认」配色',
    swatches: [
      theme.customColors.bg,
      theme.customColors.panel,
      theme.customColors.text,
      theme.customColors.accent,
    ],
    schemeLabel: theme.scheme === 'dark' ? '暗色' : '亮色',
  },
])

/** 自定义取色器的分组与中文标签 */
const TOKEN_GROUPS: { title: string; tokens: { key: keyof ThemeColors; label: string }[] }[] = [
  {
    title: '背景层级',
    tokens: [
      { key: 'bg', label: '页面背景' },
      { key: 'panel', label: '面板背景' },
      { key: 'panel2', label: '次级面板' },
      { key: 'elevated', label: '浮层 / 轨道' },
      { key: 'inputBg', label: '输入框背景' },
    ],
  },
  {
    title: '边框',
    tokens: [
      { key: 'border', label: '常规边框' },
      { key: 'borderStrong', label: '强调边框' },
      { key: 'borderHover', label: '悬停边框' },
    ],
  },
  {
    title: '文字',
    tokens: [
      { key: 'text', label: '主文字' },
      { key: 'textDim', label: '次要文字' },
      { key: 'textFaint', label: '弱化文字' },
    ],
  },
  {
    title: '品牌与语义色',
    tokens: [
      { key: 'accent', label: '强调色' },
      { key: 'accentHover', label: '强调 · 悬停' },
      { key: 'accentDim', label: '强调 · 按下' },
      { key: 'accentGlow', label: '光晕 / 选中背景' },
      { key: 'success', label: '成功' },
      { key: 'warning', label: '警告' },
      { key: 'danger', label: '危险' },
      { key: 'info', label: '信息' },
      { key: 'purple', label: '紫色标记' },
    ],
  },
]

function close(): void {
  emit('update:show', false)
}
</script>

<template>
  <NModal
    :show="props.show"
    preset="card"
    title="设置"
    style="width: min(640px, 94vw)"
    :mask-closable="true"
    @update:show="(v: boolean) => emit('update:show', v)"
  >
    <div class="flex max-h-[72vh] flex-col gap-4 overflow-y-auto">
      <!-- 主题选择 -->
      <section class="flex flex-col gap-2">
        <span class="text-[11px] font-semibold tracking-wide text-ms-text-dim">主题</span>
        <div class="grid grid-cols-2 gap-2">
          <button
            v-for="opt in options"
            :key="opt.id"
            type="button"
            class="flex flex-col gap-1.5 rounded-md border p-2.5 text-left transition-colors"
            :class="
              theme.themeId === opt.id
                ? 'border-ms-accent bg-ms-accent/10'
                : 'border-ms-border hover:border-ms-border-hover'
            "
            @click="theme.setTheme(opt.id)"
          >
            <div class="flex items-center justify-between">
              <span class="text-[13px] font-medium text-ms-text">{{ opt.label }}</span>
              <span class="text-[10px] text-ms-text-faint">{{ opt.schemeLabel }}</span>
            </div>
            <!-- 预览色点:背景 / 面板 / 文字 / 强调 -->
            <div class="flex items-center gap-1">
              <span
                v-for="(color, i) in opt.swatches"
                :key="i"
                class="h-4 w-4 rounded-sm border border-ms-border"
                :style="{ backgroundColor: color }"
              />
            </div>
            <span class="text-[11px] leading-snug text-ms-text-faint">{{ opt.description }}</span>
          </button>
        </div>
      </section>

      <!-- 自定义取色器 -->
      <section v-if="theme.themeId === 'custom'" class="flex flex-col gap-3">
        <div class="flex items-center justify-between">
          <span class="text-[11px] font-semibold tracking-wide text-ms-text-dim">自定义颜色</span>
          <NButton size="tiny" quaternary @click="theme.resetCustomColors()">
            恢复默认配色
          </NButton>
        </div>
        <div
          v-for="group in TOKEN_GROUPS"
          :key="group.title"
          class="flex flex-col gap-1.5 rounded-md border border-ms-border bg-ms-panel-2 p-2.5"
        >
          <span class="text-[10px] tracking-wide text-ms-text-faint">{{ group.title }}</span>
          <div class="grid grid-cols-2 gap-x-4 gap-y-1">
            <div
              v-for="token in group.tokens"
              :key="token.key"
              class="flex items-center justify-between gap-2"
            >
              <span class="text-xs text-ms-text-dim">{{ token.label }}</span>
              <NColorPicker
                size="small"
                class="w-[110px]"
                :value="theme.customColors[token.key]"
                :show-alpha="tokenAllowsAlpha(token.key)"
                @update:value="(v: string) => theme.setCustomColor(token.key, v)"
              />
            </div>
          </div>
        </div>
        <span class="text-[11px] leading-relaxed text-ms-text-faint">
          明暗模式按「页面背景」的亮度自动判断(影响滚动条与窗口标题栏);
          修改任意颜色即切到自定义主题,自动保存,重启后保持。
        </span>
      </section>
    </div>

    <template #footer>
      <div class="flex justify-end">
        <NButton size="small" type="primary" @click="close()">完成</NButton>
      </div>
    </template>
  </NModal>
</template>
