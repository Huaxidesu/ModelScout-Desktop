<script setup lang="ts">
/**
 * 批量粘贴导入。
 *
 * 提取放在后端：前端只把整段文本原样交给 `key_butler_import`，
 * 提取规则（已知前缀 / 带标签 / Bearer / 十六进制）全在后端一处维护，
 * 避免前后端两套规则漂移。界面上「预计 N 个」只是即时反馈，不参与导入。
 *
 * 平台判定的两种方式：
 *   - 自动识别：按 Key 形态匹配规则表，覆盖绝大多数情况；
 *   - 手动指定厂商：整批统一指定，用于形态无法区分的平台——
 *     DeepSeek / Moonshot / 硅基流动的 Key 均为 `sk-` 开头，自动识别无法区分。
 *     Base URL 留空即用该厂商在规则表里的默认地址。
 */

import { computed, ref, watch } from 'vue'
import {
  NAlert,
  NButton,
  NCard,
  NInput,
  NModal,
  NRadioButton,
  NRadioGroup,
  NSelect,
  NTag,
  useMessage,
} from 'naive-ui'

import { IMPORT_PLATFORMS, useKeyButlerStore } from '@/stores/keyButler'
import { ProbeApiError } from '@/api/commands'
import type { KeyImportMode } from '@/types/contracts'

const props = defineProps<{ show: boolean }>()
const emit = defineEmits<{ (e: 'update:show', value: boolean): void }>()

const store = useKeyButlerStore()
const message = useMessage()

const text = ref('')
const importing = ref(false)

/** 平台判定方式 */
const mode = ref<KeyImportMode>('auto')
/** 手动模式：选定的平台 */
const platform = ref<string | null>(null)
/** 手动模式：Base URL 覆盖；留空 = 用该平台默认值 */
const baseUrl = ref('')

/** 最近一次导入的结果（用于展示 added / duplicates / ignored） */
const lastResult = ref<{ added: number; duplicates: number; ignored: number } | null>(null)

/**
 * 预估候选数量，仅用于即时反馈。
 *
 * 刻意写得宽松（按行 + 按空白切分再筛长度），真实数量以后端为准；
 * 与后端实现完全一致反而会制造「两套规则必须同步」的维护负担。
 */
const estimate = computed(() => {
  const candidates = text.value
    .split(/[\s,，;；]+/)
    .map((token) => token.replace(/^["'`]|["'`]$/g, '').trim())
    .filter((token) => token.length >= 12)
  return new Set(candidates).size
})

/** 关闭时清空，避免下次打开还留着上次的密钥文本 */
watch(
  () => props.show,
  (show) => {
    if (!show) {
      text.value = ''
      baseUrl.value = ''
      lastResult.value = null
    }
  },
)

async function submit(): Promise<void> {
  if (!text.value.trim()) {
    message.warning('请先粘贴内容。')
    return
  }
  if (mode.value === 'manual' && !platform.value) {
    message.warning('手动指定厂商模式下请先选择厂商。')
    return
  }
  importing.value = true
  try {
    const result = await store.importKeys({
      text: text.value,
      mode: mode.value,
      platform: mode.value === 'manual' ? platform.value : null,
      baseUrl: mode.value === 'manual' ? baseUrl.value : null,
    })
    lastResult.value = {
      added: result.added,
      duplicates: result.duplicates,
      ignored: result.ignored,
    }
    if (result.added > 0) {
      message.success(`已导入 ${result.added} 个 Key`)
      text.value = ''
    } else {
      message.warning('没有导入新 Key（可能全是重复的，或文本里没有可识别的 Key）。')
    }
  } catch (error) {
    message.error(error instanceof ProbeApiError ? error.message : String(error))
  } finally {
    importing.value = false
  }
}
</script>

<template>
  <NModal
    :show="show"
    preset="card"
    style="width: 680px"
    @update:show="emit('update:show', $event)"
  >
    <NCard :bordered="false" title="批量粘贴导入" size="small">
      <div class="mb-2 text-[11px] leading-relaxed text-ms-text-dim">
        把一批 Key 整段粘进来即可 —— 每行一个、逗号分隔、夹在聊天记录里都行。
        <br />
        程序会自动去重；识别不了的片段会被忽略。
      </div>

      <!-- 平台判定方式 -->
      <div class="mb-2 flex flex-wrap items-center gap-2">
        <span class="text-[11px] text-ms-text-faint">平台判定</span>
        <NRadioGroup v-model:value="mode" size="small">
          <NRadioButton value="auto">自动识别</NRadioButton>
          <NRadioButton value="manual">手动指定厂商</NRadioButton>
        </NRadioGroup>
        <span v-if="mode === 'auto'" class="text-[10px] text-ms-text-faint">
          按 Key 形态匹配（sk- / gsk_ / AIza …）
        </span>
      </div>

      <!-- 手动模式：厂商 + Base URL -->
      <div v-if="mode === 'manual'" class="mb-2 flex flex-wrap items-center gap-2">
        <NSelect
          v-model:value="platform"
          :options="IMPORT_PLATFORMS"
          size="small"
          placeholder="选择厂商"
          style="width: 200px"
          :consistent-menu-width="false"
        />
        <NInput
          v-model:value="baseUrl"
          size="small"
          placeholder="Base URL（留空则用该厂商默认地址）"
          style="width: 320px"
        />
      </div>

      <NInput
        v-model:value="text"
        type="textarea"
        :rows="8"
        placeholder="例如：&#10;sk-proj-xxxxxxxxxxxxxxxxxxxxxxxx&#10;gsk_yyyyyyyyyyyyyyyyyyyy&#10;apiKey: xai-zzzzzzzzzzzzzzzz"
      />

      <div class="mt-2 flex items-center gap-2 text-[11px] text-ms-text-faint">
        <NTag size="tiny" :bordered="false" type="info">
          预计 {{ estimate }} 个
        </NTag>
        <span>（仅作提示，实际以服务端提取结果为准）</span>
      </div>

      <!-- 上次导入结果 -->
      <NAlert
        v-if="lastResult"
        class="mt-3"
        :bordered="false"
        type="success"
        :title="`新增 ${lastResult.added} 个`"
      >
        <div class="text-[11px]">
          重复跳过 {{ lastResult.duplicates }} 个 · 未识别片段 {{ lastResult.ignored }} 行
        </div>
      </NAlert>

      <div class="mt-3 text-[10px] leading-relaxed text-ms-text-faint">
        默认不落盘：Key 只存在本次会话内存里，关闭应用即清空。
        需要长期保存请用工具栏的「启用本地保存」。
        <br />
        导入后想改平台 / Base URL，可以在列表里勾选多条后点「批量编辑」。
      </div>

      <template #footer>
        <div class="flex justify-end gap-2">
          <NButton size="small" @click="emit('update:show', false)">关闭</NButton>
          <NButton size="small" type="primary" :loading="importing" @click="submit">
            导入
          </NButton>
        </div>
      </template>
    </NCard>
  </NModal>
</template>
