<script setup lang="ts">
/**
 * ============================================================================
 * AboutDialog —— 关于弹窗
 * ============================================================================
 *
 * 【职责】
 *   1. 展示应用标识（名称、版本、订阅文案）
 *   2. 展示核心能力列表
 *   3. 提供 GitHub 仓库入口（使用系统浏览器打开）
 *
 * 【交互】
 * 由 AppHeader 通过 v-model:show 控制显隐。
 * ============================================================================
 */

import { NModal, NCard, NButton, NTag } from 'naive-ui'
import { onMounted, ref } from 'vue'
import { openUrl } from '@tauri-apps/plugin-opener'
import { getVersion } from '@tauri-apps/api/app'

import logoUrl from '@/assets/logo-icon.png'

const props = defineProps<{
  show: boolean
}>()

const emit = defineEmits<{
  (e: 'update:show', value: boolean): void
}>()

/** 应用版本（来自 tauri.conf.json，由 Tauri 运行时提供） */
const version = ref('0.1.0')

onMounted(async () => {
  try {
    version.value = await getVersion()
  } catch {
    // 在浏览器中裸跑前端时 Tauri API 不可用，保留默认值即可
  }
})

/** GitHub 仓库地址 */
const REPO_URL = 'https://github.com/Huaxidesu/ModelScout-Desktop'

/** 核心能力列表 */
const FEATURES = [
  '自动探测任意 OpenAI 兼容 API 端点',
  '逐模型检测工具调用 / 附件上传 / 推理能力',
  '解析上下文窗口与输出上限',
  '一键生成配置文件',
]

/** 打开 GitHub 仓库（交给系统浏览器） */
function openRepo(): void {
  openUrl(REPO_URL)
    .then(() => emit('update:show', false))
    .catch(() => {})
}
</script>

<template>
  <NModal
    :show="props.show"
    preset="card"
    style="width: 460px"
    @update:show="(v: boolean) => emit('update:show', v)"
  >
    <NCard :bordered="false" size="small">
      <!-- 头部：logo + 名称 + 版本 -->
      <div class="flex items-center gap-3">
        <img
          :src="logoUrl"
          alt="ModelScout"
          class="h-10 w-10 shrink-0 rounded-lg object-contain"
        />
        <div class="flex flex-col">
          <div class="flex items-center gap-2">
            <span class="text-lg font-semibold text-ms-text">ModelScout</span>
            <NTag size="tiny" :bordered="false">v{{ version }}</NTag>
          </div>
          <div class="text-[12px] text-ms-text-dim">大模型 API 探测与配置生成器</div>
        </div>
      </div>

      <!-- 核心能力 -->
      <div class="mt-4">
        <div class="mb-2 text-[11px] font-medium uppercase tracking-wide text-ms-text-faint">
          核心能力
        </div>
        <ul class="space-y-1.5">
          <li
            v-for="f in FEATURES"
            :key="f"
            class="flex items-start gap-2 text-[12px] text-ms-text-dim"
          >
            <span class="mt-0.5 text-ms-accent">✓</span>
            {{ f }}
          </li>
        </ul>
      </div>

      <!-- 版权与许可 -->
      <div class="mt-4 border-t border-ms-border pt-3 text-[11px] leading-relaxed text-ms-text-faint">
        © 2026 Huaxidesu · 依据 PolyForm Noncommercial 1.0.0 许可发布<br />
        非商业用途
      </div>

      <!-- 底部操作 -->
      <template #footer>
        <div class="flex items-center justify-between">
          <NButton size="small" quaternary @click="emit('update:show', false)">
            关闭
          </NButton>
          <NButton size="small" type="primary" ghost @click="openRepo">
            <span class="mr-1 text-sm leading-none">
              <!-- GitHub Mark 图标 -->
              <svg
                viewBox="0 0 16 16"
                width="14"
                height="14"
                fill="currentColor"
                aria-hidden="true"
              >
                <path
                  d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0 0 16 8c0-4.42-3.58-8-8-8z"
                />
              </svg>
            </span>
            访问 GitHub
          </NButton>
        </div>
      </template>
    </NCard>
  </NModal>
</template>