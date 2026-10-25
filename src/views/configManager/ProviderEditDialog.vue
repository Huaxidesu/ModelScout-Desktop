<script setup lang="ts">
/**
 * 配置管理「编辑 provider」对话框：逐模型调整思考/附件/工具/上下文/输出，
 * 保存时按文件 schema 写回原 provider 并落盘（自动备份）。
 *
 * 只写「动过的字段」的三态语义见 modelOverrides.ts —— 打开时记一份
 * 原始投影，保存时逐字段比对，未改动的字段不重写，避免把「未设置」
 * （跟随工具默认）覆盖成明确值。
 */

import { computed, ref, watch } from 'vue'
import { NButton, NModal, NTag, useMessage } from 'naive-ui'

import { writeToolConfig } from '@/api/commands'
import ModelEditTable from '@/components/ModelEditTable.vue'
import {
  applyDshRows,
  applyOpenCodeRows,
  extractDshRows,
  extractOpenCodeRows,
  type ModelEditRow,
} from '@/generators/modelOverrides'
import { mergeDshProfileIntoText } from '@/generators/dsh'
import { upsertOpenCodeProvider } from '@/generators/opencodeMerge'
import { rawDshProvider, rawOpenCodeProvider } from '@/generators/configScan'

const props = defineProps<{
  show: boolean
  /** 目标工具 */
  tool: 'opencode' | 'dsh'
  /** 配置文件路径（写回目标） */
  path: string
  /** provider 键名 */
  providerKey: string
  /** 文件内容（由配置管理页持有，保存后由父级重扫） */
  content: string
}>()

const emit = defineEmits<{
  (e: 'update:show', value: boolean): void
  (e: 'saved'): void
}>()

const message = useMessage()

/** 当前编辑行（打开时重建） */
const rows = ref<ModelEditRow[]>([])
/** 打开时的原始投影（脏字段比对基准） */
const originals = ref<ModelEditRow[]>([])
/** 文件世代（仅 opencode 有 v1/v2 之分） */
const schema = ref<'v1' | 'v2'>('v1')
const saving = ref(false)

const hasModels = computed(() => rows.value.length > 0)

/** 打开时从配置提取 */
watch(
  () => props.show,
  (show) => {
    if (!show) return
    if (props.tool === 'opencode') {
      const raw = rawOpenCodeProvider(props.content, props.providerKey)
      if (!raw) {
        message.error('provider 已不存在，请重新识别。')
        emit('update:show', false)
        return
      }
      schema.value = raw.schema
      rows.value = extractOpenCodeRows(raw.value, raw.schema)
    } else {
      const raw = rawDshProvider(props.content, props.providerKey)
      if (!raw) {
        message.error('provider 已不存在，请重新识别。')
        emit('update:show', false)
        return
      }
      rows.value = extractDshRows(raw)
    }
    originals.value = JSON.parse(JSON.stringify(rows.value)) as ModelEditRow[]
  },
)

/** 保存：投影写回原始 provider → upsert 到文件 → 落盘 */
async function save(): Promise<void> {
  saving.value = true
  try {
    if (props.tool === 'opencode') {
      const raw = rawOpenCodeProvider(props.content, props.providerKey)
      if (!raw) throw new Error('provider 已不存在，请重新识别。')
      const modified = applyOpenCodeRows(raw.value, rows.value, originals.value, raw.schema)
      const result = upsertOpenCodeProvider(props.content, props.providerKey, modified)
      await writeToolConfig(props.path, result.text, true)
    } else {
      const raw = rawDshProvider(props.content, props.providerKey)
      if (!raw) throw new Error('provider 已不存在，请重新识别。')
      const modified = applyDshRows(raw, rows.value, originals.value)
      const result = mergeDshProfileIntoText(props.content, props.providerKey, modified)
      await writeToolConfig(props.path, result.text, true)
    }
    message.success(`已保存 ${props.providerKey} 的模型参数（原文件已备份）`)
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
    :title="`编辑 provider — ${providerKey}`"
    style="width: min(760px, 94vw)"
    :mask-closable="false"
    @update:show="(v: boolean) => emit('update:show', v)"
  >
    <div class="flex max-h-[70vh] flex-col gap-3">
      <div class="text-[11px] leading-relaxed text-ms-text-dim">
        {{ path }}
        <NTag v-if="tool === 'opencode'" size="tiny" :bordered="false" class="ml-1">
          OpenCode {{ schema }}
        </NTag>
        <NTag v-else size="tiny" :bordered="false" class="ml-1">DeepSeek Harness</NTag>
        —— 保存后写回该文件并自动备份为 .modelscout.bak。
      </div>

      <div class="min-h-0 flex-1 overflow-y-auto pr-1">
        <ModelEditTable
          :rows="rows"
          :show-tools="tool === 'opencode'"
          :show-reasoning="true"
          :v2="tool === 'opencode' && schema === 'v2'"
        />
      </div>
    </div>

    <template #footer>
      <div class="flex justify-end gap-2">
        <NButton size="small" quaternary @click="emit('update:show', false)">取消</NButton>
        <NButton size="small" type="primary" :loading="saving" :disabled="!hasModels" @click="save()">
          保存并写入
        </NButton>
      </div>
    </template>
  </NModal>
</template>
