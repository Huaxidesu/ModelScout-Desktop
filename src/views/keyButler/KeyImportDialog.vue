<script setup lang="ts">
/**
 * ============================================================================
 * Key 管家 —— 批量粘贴导入
 * ============================================================================
 *
 * 【为什么提取放在后端】
 * 前端只把整段文本原样交给 `key_butler_import`，提取规则（已知前缀 /
 * 带标签 / Bearer / 十六进制）全在后端一处维护 —— 避免「前端能识别、
 * 后端不认」或反过来这种两套规则漂移的问题。
 * 界面上那个「预计 N 个」只是**给用户的即时反馈**，不参与真正导入。
 * ============================================================================
 */

import { computed, ref, watch } from 'vue'
import { NAlert, NButton, NCard, NInput, NModal, NTag, useMessage } from 'naive-ui'

import { useKeyButlerStore } from '@/stores/keyButler'
import { ProbeApiError } from '@/api/commands'

const props = defineProps<{ show: boolean }>()
const emit = defineEmits<{ (e: 'update:show', value: boolean): void }>()

const store = useKeyButlerStore()
const message = useMessage()

const text = ref('')
const importing = ref(false)

/** 最近一次导入的结果（用于展示 added / duplicates / ignored） */
const lastResult = ref<{ added: number; duplicates: number; ignored: number } | null>(null)

/**
 * 预估候选数量 —— **仅用于即时反馈**。
 *
 * 这里刻意写得宽松（按行 + 按空白切分再筛长度），真实数量以后端为准；
 * 写成和后端一模一样反而会制造「两套规则必须同步」的维护负担。
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
      lastResult.value = null
    }
  },
)

async function submit(): Promise<void> {
  if (!text.value.trim()) {
    message.warning('请先粘贴内容。')
    return
  }
  importing.value = true
  try {
    const result = await store.importKeys(text.value)
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
        程序会按前缀识别平台、自动去重；识别不了的片段会被忽略。
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
