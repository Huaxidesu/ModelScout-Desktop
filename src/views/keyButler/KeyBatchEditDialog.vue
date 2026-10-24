<script setup lang="ts">
/**
 * 批量编辑：一次修改多条 Key 的平台 / Base URL / 余额接口 / 备注。
 *
 * 每一项前的「修改」勾选框：批量补丁语义为「未勾选的项一律不动」。
 * 若改为「留空即清空」，用户只想改平台时会顺带抹掉所有 Base URL，
 * 批量操作中此类误伤代价极高，因此要求显式勾选。
 *
 * 「套用该平台默认端点」默认不勾：多数批量编辑只是纠正标签
 * （例如「这批其实是 DeepSeek」），用户手填过的 Base URL 必须保留；
 * 仅在明确要整批换成该平台官方配置时才勾选。
 */

import { computed, ref, watch } from 'vue'
import { NAlert, NButton, NCard, NCheckbox, NInput, NModal, NSelect, useMessage } from 'naive-ui'

import {
  BALANCE_ENDPOINT_OPTIONS,
  IMPORT_PLATFORMS,
  platformLabel,
  useKeyButlerStore,
} from '@/stores/keyButler'
import { ProbeApiError } from '@/api/commands'
import type { BalanceEndpointKind, KeyBatchPatch } from '@/types/contracts'

const props = defineProps<{
  show: boolean
  /** 待修改的 Key id 列表 */
  ids: string[]
}>()

const emit = defineEmits<{ (e: 'update:show', value: boolean): void }>()

const store = useKeyButlerStore()
const message = useMessage()

/* ---- 表单状态 ---- */

const editPlatform = ref(false)
const platform = ref<string | null>(null)
const applyPlatformDefaults = ref(false)

const editBaseUrl = ref(false)
const baseUrl = ref('')

const editEndpoint = ref(false)
const balanceEndpoint = ref<BalanceEndpointKind | null>(null)

const editNote = ref(false)
const note = ref('')

const submitting = ref(false)

/** 打开时重置，避免上次的选择串到这一次 */
watch(
  () => props.show,
  (show) => {
    if (!show) return
    editPlatform.value = false
    platform.value = null
    applyPlatformDefaults.value = false
    editBaseUrl.value = false
    baseUrl.value = ''
    editEndpoint.value = false
    balanceEndpoint.value = null
    editNote.value = false
    note.value = ''
  },
)

/** 是否有任何一项被勾选 */
const hasChange = computed(
  () => editPlatform.value || editBaseUrl.value || editEndpoint.value || editNote.value,
)

/** 变更摘要（让用户在点「应用」前能确认一遍） */
const summary = computed(() => {
  const parts: string[] = []
  if (editPlatform.value && platform.value) {
    parts.push(`平台 → ${platformLabel(platform.value)}${applyPlatformDefaults.value ? '（含默认端点）' : ''}`)
  }
  if (editBaseUrl.value) {
    parts.push(`Base URL → ${baseUrl.value.trim() || '（清空）'}`)
  }
  if (editEndpoint.value && balanceEndpoint.value) {
    parts.push(`余额接口 → ${BALANCE_ENDPOINT_OPTIONS.find((o) => o.value === balanceEndpoint.value)?.label ?? balanceEndpoint.value}`)
  }
  if (editNote.value) {
    parts.push(`备注 → ${note.value.trim() || '（清空）'}`)
  }
  return parts
})

async function submit(): Promise<void> {
  if (!hasChange.value) {
    message.warning('请至少勾选一项要修改的内容。')
    return
  }
  if (editPlatform.value && !platform.value) {
    message.warning('勾选了「平台」但还没选择平台。')
    return
  }
  if (editEndpoint.value && !balanceEndpoint.value) {
    message.warning('勾选了「余额接口」但还没选择适配器。')
    return
  }

  // 只把「勾选了」的项放进补丁：未勾选的键在契约里代表「不修改」
  const patch: KeyBatchPatch = {}
  if (editPlatform.value) {
    patch.platform = platform.value
    patch.applyPlatformDefaults = applyPlatformDefaults.value
  }
  if (editBaseUrl.value) patch.baseUrl = baseUrl.value
  if (editEndpoint.value) patch.balanceEndpoint = balanceEndpoint.value
  if (editNote.value) patch.note = note.value

  submitting.value = true
  try {
    const count = await store.updateMany(props.ids, patch)
    message.success(`已修改 ${count} 条 Key`)
    emit('update:show', false)
  } catch (error) {
    message.error(error instanceof ProbeApiError ? error.message : String(error))
  } finally {
    submitting.value = false
  }
}
</script>

<template>
  <NModal
    :show="show"
    preset="card"
    style="width: 560px"
    @update:show="emit('update:show', $event)"
  >
    <NCard :bordered="false" title="批量编辑" size="small">
      <NAlert class="mb-3" :bordered="false" type="info">
        <span class="text-[11px]">
          将修改已选中的 <b>{{ ids.length }}</b> 条 Key。只有勾选的项目才会被改动，其余保持原样。
        </span>
      </NAlert>

      <!-- 平台 -->
      <div class="mb-3 flex flex-wrap items-center gap-2">
        <NCheckbox v-model:checked="editPlatform" size="small" class="w-[72px] shrink-0">
          平台
        </NCheckbox>
        <NSelect
          v-model:value="platform"
          :options="IMPORT_PLATFORMS"
          size="small"
          :disabled="!editPlatform"
          placeholder="选择平台"
          style="width: 220px"
          :consistent-menu-width="false"
        />
      </div>

      <!-- 套用默认端点 -->
      <div class="mb-3 pl-[72px]">
        <NCheckbox
          v-model:checked="applyPlatformDefaults"
          size="small"
          :disabled="!editPlatform"
        >
          <span class="text-[11px]">同时套用该平台默认的 Base URL 与余额接口</span>
        </NCheckbox>
        <div class="mt-1 text-[10px] leading-relaxed text-ms-text-faint">
          不勾选时只改平台标签，你手填过的 Base URL 会被保留。
        </div>
      </div>

      <!-- Base URL -->
      <div class="mb-3 flex flex-wrap items-center gap-2">
        <NCheckbox v-model:checked="editBaseUrl" size="small" class="w-[72px] shrink-0">
          Base URL
        </NCheckbox>
        <NInput
          v-model:value="baseUrl"
          size="small"
          :disabled="!editBaseUrl"
          placeholder="留空表示清空该字段"
          style="width: 340px"
        />
      </div>

      <!-- 余额接口 -->
      <div class="mb-3 flex flex-wrap items-center gap-2">
        <NCheckbox v-model:checked="editEndpoint" size="small" class="w-[72px] shrink-0">
          余额接口
        </NCheckbox>
        <NSelect
          v-model:value="balanceEndpoint"
          :options="BALANCE_ENDPOINT_OPTIONS"
          size="small"
          :disabled="!editEndpoint"
          placeholder="选择余额适配器"
          style="width: 220px"
          :consistent-menu-width="false"
        />
      </div>

      <!-- 备注 -->
      <div class="mb-3 flex flex-wrap items-center gap-2">
        <NCheckbox v-model:checked="editNote" size="small" class="w-[72px] shrink-0">
          备注
        </NCheckbox>
        <NInput
          v-model:value="note"
          size="small"
          :disabled="!editNote"
          placeholder="留空表示清空备注"
          style="width: 340px"
        />
      </div>

      <!-- 变更摘要 -->
      <div v-if="summary.length > 0" class="rounded border border-ms-border bg-ms-panel-2 px-3 py-2">
        <div class="mb-1 text-[10px] text-ms-text-faint">将要执行：</div>
        <ul class="ml-4 list-disc text-[11px] text-ms-text-dim">
          <li v-for="item in summary" :key="item">{{ item }}</li>
        </ul>
      </div>

      <template #footer>
        <div class="flex justify-end gap-2">
          <NButton size="small" @click="emit('update:show', false)">取消</NButton>
          <NButton
            size="small"
            type="primary"
            :loading="submitting"
            :disabled="!hasChange"
            @click="submit"
          >
            应用到 {{ ids.length }} 条
          </NButton>
        </div>
      </template>
    </NCard>
  </NModal>
</template>
