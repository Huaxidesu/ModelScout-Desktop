<script setup lang="ts">
/**
 * 配置管理「编辑条目注释」对话框。
 *
 * 改的是 provider 键上方那行 ModelScout 标记注释
 * （`// 名字 · N 个模型 · ModelScout 日期`）。留空 = 删除该行；
 * 保存时若发现历史堆积的多条标记注释，会一并清掉只留一条。
 *
 * 严格 JSON（`.json`）不能写注释，这类文件直接禁用保存并说明原因。
 */

import { computed, ref, watch } from 'vue'
import { NAlert, NButton, NInput, NModal, useMessage } from 'naive-ui'

import { writeToolConfig } from '@/api/commands'
import { setProviderComment } from '@/generators/opencodeComments'

const props = defineProps<{
  show: boolean
  /** 配置文件路径（写回目标） */
  path: string
  /** provider 键名 */
  providerKey: string
  /** 文件当前内容（由配置管理页持有） */
  content: string
  /** 当前注释正文；null = 文件里没有标记注释 */
  current: string | null
  /** 「填入自动文案」用的默认文本 */
  autoText: string
}>()

const emit = defineEmits<{
  (e: 'update:show', value: boolean): void
  (e: 'saved'): void
}>()

const message = useMessage()

/** 编辑草稿 */
const draft = ref('')
const saving = ref(false)

/** 严格 JSON 文件写 `//` 会直接解析失败，因此只允许 .jsonc 等带注释的格式 */
const commentable = computed(() => !/\.json$/i.test(props.path.trim()))

watch(
  () => props.show,
  (show) => {
    if (show) draft.value = props.current ?? ''
  },
)

/** 用自动文案覆盖草稿（日期取当天，等于重新生成一次） */
function fillAuto(): void {
  draft.value = props.autoText
}

async function save(): Promise<void> {
  saving.value = true
  try {
    const next = setProviderComment(props.content, props.providerKey, draft.value)
    if (next === null) throw new Error('provider 已不存在，请重新识别。')
    if (next === props.content) {
      message.info('注释没有变化')
      emit('update:show', false)
      return
    }
    await writeToolConfig(props.path, next, true)
    message.success(draft.value.trim() ? '注释已更新（原文件已备份）' : '注释已删除（原文件已备份）')
    emit('saved')
    emit('update:show', false)
  } catch (error) {
    message.error(error instanceof Error ? error.message : String(error))
  } finally {
    saving.value = false
  }
}
</script>

<template>
  <NModal
    :show="show"
    preset="card"
    :title="`编辑注释 — ${providerKey}`"
    style="width: min(620px, 94vw)"
    :mask-closable="false"
    @update:show="(v: boolean) => emit('update:show', v)"
  >
    <div class="flex flex-col gap-3">
      <div class="text-[11px] leading-relaxed text-ms-text-dim">
        {{ path }} —— 注释写在
        <span class="font-mono">{{ providerKey }}</span>
        这一行的上方，只影响阅读，不影响配置行为。留空保存 = 删除该行注释。
      </div>

      <NAlert v-if="!commentable" type="warning" :bordered="false">
        该文件是严格 JSON（<span class="font-mono">.json</span>），写入
        <span class="font-mono">//</span>
        注释会让文件无法解析。请先把它改名为
        <span class="font-mono">.jsonc</span>（opencode 同样识别）再来编辑。
      </NAlert>

      <NInput
        v-model:value="draft"
        type="textarea"
        :autosize="{ minRows: 2, maxRows: 4 }"
        :disabled="!commentable"
        placeholder="例如：主力网关 · 2026-10-29 换成 V4.1"
      />

      <div class="flex items-center gap-2 text-[10px] text-ms-text-faint">
        <NButton size="tiny" quaternary :disabled="!commentable" @click="fillAuto()">
          填入自动文案
        </NButton>
        <span class="truncate" :title="autoText">{{ autoText }}</span>
      </div>
    </div>

    <template #footer>
      <div class="flex justify-end gap-2">
        <NButton size="small" quaternary @click="emit('update:show', false)">取消</NButton>
        <NButton
          size="small"
          type="primary"
          :loading="saving"
          :disabled="!commentable"
          @click="save()"
        >
          保存并写入
        </NButton>
      </div>
    </template>
  </NModal>
</template>
