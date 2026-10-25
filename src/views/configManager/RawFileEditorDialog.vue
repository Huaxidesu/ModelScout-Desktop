<script setup lang="ts">
/**
 * 原始文件编辑器（配置管理「编辑原始文件」）。
 *
 * 打开时按路径重新读取磁盘内容（不用父组件的扫描缓存，避免编辑到旧数据），
 * textarea 自由改写；保存前按扩展名做语法校验（JSONC/YAML），不合法则
 * 阻止并显示错误行；写入走 writeToolConfig（自动 .modelscout.bak 备份）。
 */

import { computed, ref, watch } from 'vue'
import { NAlert, NButton, NInput, NModal, NTag } from 'naive-ui'

import { inspectToolConfig, writeToolConfig, ProbeApiError } from '@/api/commands'
import { validateConfigText } from '@/utils/configValidate'

const props = defineProps<{
  show: boolean
  /** 文件所属工具（决定默认路径推断与校验语义） */
  tool: 'opencode' | 'dsh'
  /** 文件绝对路径 */
  path: string
}>()

const emit = defineEmits<{
  (e: 'update:show', value: boolean): void
  /** 保存成功（父组件应重新扫描） */
  (e: 'saved', path: string): void
}>()

const text = ref('')
const loading = ref(false)
const saving = ref(false)
const errorText = ref('')
const exists = ref(true)

watch(
  () => props.show,
  async (show) => {
    if (!show) return
    loading.value = true
    errorText.value = ''
    text.value = ''
    try {
      const file = await inspectToolConfig(props.tool, props.path)
      exists.value = file.exists
      text.value = file.content
    } catch (err) {
      errorText.value = err instanceof ProbeApiError || err instanceof Error ? err.message : String(err)
    } finally {
      loading.value = false
    }
  },
)

/** 语法校验结果（实时） */
const syntaxError = computed(() => (exists.value ? validateConfigText(props.path, text.value) : null))

const dirty = computed(() => text.value !== '')

/** 保存可用:有改动、无语法错误、不在保存中 */
const canSave = computed(() => dirty.value && !saving.value && !loading.value && syntaxError.value === null)

async function save(): Promise<void> {
  const err = validateConfigText(props.path, text.value)
  if (err) {
    errorText.value = err
    return
  }
  saving.value = true
  errorText.value = ''
  try {
    const result = await writeToolConfig(props.path, text.value, true)
    emit('saved', result.path)
    emit('update:show', false)
  } catch (err2) {
    errorText.value = err2 instanceof ProbeApiError || err2 instanceof Error ? err2.message : String(err2)
  } finally {
    saving.value = false
  }
}

function close(): void {
  emit('update:show', false)
}
</script>

<template>
  <NModal
    :show="props.show"
    preset="card"
    title="编辑原始文件"
    style="width: min(960px, 96vw)"
    :mask-closable="false"
    @update:show="(v: boolean) => emit('update:show', v)"
  >
    <div class="flex flex-col gap-2">
      <div class="flex flex-wrap items-center gap-2">
        <span class="font-mono text-[11px] text-ms-text-faint" :title="props.path">
          {{ props.path }}
        </span>
        <NTag size="tiny" :bordered="false" :type="exists ? 'default' : 'warning'">
          {{ exists ? '已存在' : '文件不存在' }}
        </NTag>
        <NTag v-if="dirty" size="tiny" :bordered="false" type="info">已修改</NTag>
        <span class="ml-auto text-[10px] text-ms-text-faint">{{ text.split('\n').length }} 行</span>
      </div>

      <NAlert v-if="errorText" type="error" :bordered="false" style="border-radius: 0">
        <div class="text-[11px]">{{ errorText }}</div>
      </NAlert>
      <NAlert v-else-if="syntaxError" type="warning" :bordered="false" style="border-radius: 0">
        <div class="text-[11px]">{{ syntaxError }}（修正后才能保存）</div>
      </NAlert>

      <NInput
        v-model:value="text"
        type="textarea"
        class="ms-code"
        :autosize="false"
        :loading="loading"
        placeholder="文件内容"
        spellcheck="false"
        style="height: min(56vh, 560px)"
      />

      <div class="text-[11px] leading-relaxed text-ms-text-faint">
        直接编辑原始文件的全部内容。保存前按扩展名做语法校验
        （JSONC 容注释 / YAML）；覆盖前自动备份为
        <code>.modelscout.bak</code>。保存后建议点「一键识别」刷新 provider 清单。
      </div>
    </div>

    <template #footer>
      <div class="flex justify-end gap-2">
        <NButton size="small" quaternary @click="close()">取消</NButton>
        <NButton size="small" type="primary" :loading="saving" :disabled="!canSave" @click="save()">
          保存并写入（自动备份）
        </NButton>
      </div>
    </template>
  </NModal>
</template>

