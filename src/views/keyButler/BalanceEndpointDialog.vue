<script setup lang="ts">
/**
 * ============================================================================
 * Key 管家 —— 单条 Key 的配置编辑
 * ============================================================================
 *
 * 三块内容：备注、Base URL、余额接口适配器。
 *
 * 【为什么 Base URL 是可编辑的】
 * 前缀表只能给一个「平台默认值」，而同一批 Key 完全可能来自不同中转站。
 * 有效性检测与余额查询都依赖它，所以必须逐条可改。
 *
 * 【extraHeaders 的输入形式】
 * 界面上用「每行一条 `Name: Value`」的文本域，而不是键值对表格 ——
 * 自定义头是低频需求，表格控件反而更重。提交前在本地解析成对象。
 * ============================================================================
 */

import { computed, ref, watch } from 'vue'
import { NAlert, NButton, NCard, NInput, NInputNumber, NModal, NSelect, useMessage } from 'naive-ui'

import type { BalanceEndpoint, BalanceEndpointKind, HttpMethod, KeyEntry } from '@/types/contracts'
import { BALANCE_ENDPOINT_OPTIONS, useKeyButlerStore } from '@/stores/keyButler'
import { ProbeApiError } from '@/api/commands'

const props = defineProps<{
  show: boolean
  entry: KeyEntry | null
}>()
const emit = defineEmits<{ (e: 'update:show', value: boolean): void }>()

const store = useKeyButlerStore()
const message = useMessage()

const saving = ref(false)

const note = ref('')
const baseUrl = ref('')
const kind = ref<BalanceEndpointKind>('none')
const urlTemplate = ref('')
const method = ref<HttpMethod>('GET')
const headersText = ref('')
const totalPath = ref('')
const usedPath = ref('')
const balancePath = ref('')
const unitDivisor = ref(1)

/**
 * 适配器选项。
 *
 * 直接复用 store 里的共享常量 —— 单条编辑与批量编辑必须给出同一套选项，
 * 各自维护一份迟早会漏掉新适配器（`builtin-deepseek` 就是这么漏掉的）。
 */
const KIND_OPTIONS = BALANCE_ENDPOINT_OPTIONS

const METHOD_OPTIONS: Array<{ label: string; value: HttpMethod }> = [
  { label: 'GET', value: 'GET' },
  { label: 'POST', value: 'POST' },
]

/** 打开时用当前条目填充表单 */
watch(
  () => [props.show, props.entry] as const,
  ([show, entry]) => {
    if (!show || !entry) return
    note.value = entry.note
    baseUrl.value = entry.baseUrl ?? ''
    kind.value = entry.balanceEndpoint
    const custom = entry.customEndpoint
    urlTemplate.value = custom?.urlTemplate ?? '{base}/v1/dashboard/billing/subscription'
    method.value = custom?.method ?? 'GET'
    headersText.value = custom
      ? Object.entries(custom.extraHeaders)
          .map(([name, value]) => `${name}: ${value}`)
          .join('\n')
      : ''
    totalPath.value = custom?.totalPath ?? ''
    usedPath.value = custom?.usedPath ?? ''
    balancePath.value = custom?.balancePath ?? ''
    unitDivisor.value = custom?.unitDivisor ?? 1
  },
  { immediate: true },
)

/** 每行 `Name: Value` → 对象 */
function parseHeaders(): Record<string, string> {
  const result: Record<string, string> = {}
  for (const line of headersText.value.split('\n')) {
    const trimmed = line.trim()
    if (!trimmed) continue
    const index = trimmed.indexOf(':')
    if (index <= 0) continue
    const name = trimmed.slice(0, index).trim()
    const value = trimmed.slice(index + 1).trim()
    if (name) result[name] = value
  }
  return result
}

/** 空字符串归一为 null（后端也做同样归一，这里是为了少传无意义的值） */
function nullable(value: string): string | null {
  const trimmed = value.trim()
  return trimmed.length > 0 ? trimmed : null
}

const savingDisabled = computed(() => !props.entry)

async function save(): Promise<void> {
  const entry = props.entry
  if (!entry) return

  if (kind.value === 'custom' && !urlTemplate.value.trim()) {
    message.warning('自定义余额接口必须填 URL 模板。')
    return
  }

  const customEndpoint: BalanceEndpoint | null =
    kind.value === 'custom'
      ? {
          urlTemplate: urlTemplate.value.trim(),
          method: method.value,
          extraHeaders: parseHeaders(),
          totalPath: nullable(totalPath.value),
          usedPath: nullable(usedPath.value),
          balancePath: nullable(balancePath.value),
          unitDivisor: unitDivisor.value > 0 ? unitDivisor.value : 1,
        }
      : null

  saving.value = true
  try {
    await store.updateNote(entry.id, note.value)
    await store.setConfig(entry.id, {
      baseUrl: nullable(baseUrl.value),
      balanceEndpoint: kind.value,
      customEndpoint,
    })
    message.success('已保存')
    emit('update:show', false)
  } catch (error) {
    message.error(error instanceof ProbeApiError ? error.message : String(error))
  } finally {
    saving.value = false
  }
}
</script>

<template>
  <NModal
    :show="show"
    preset="card"
    style="width: 620px"
    @update:show="emit('update:show', $event)"
  >
    <NCard :bordered="false" size="small" :title="`编辑：${entry?.maskedKey ?? ''}`">
      <div class="flex flex-col gap-3">
        <div>
          <div class="mb-1 text-[11px] text-ms-text-dim">备注</div>
          <NInput v-model:value="note" size="small" placeholder="例如：测试账号 / 同事借的" />
        </div>

        <div>
          <div class="mb-1 text-[11px] text-ms-text-dim">Base URL</div>
          <NInput
            v-model:value="baseUrl"
            size="small"
            placeholder="https://relay.example.com/v1"
          />
          <div class="mt-1 text-[10px] text-ms-text-faint">
            有效性检测与余额查询都用它。留空则该条无法检测。
          </div>
        </div>

        <div>
          <div class="mb-1 text-[11px] text-ms-text-dim">余额接口</div>
          <!-- consistent-menu-width=false：菜单按内容自适应宽度，避免长选项被截断 -->
          <NSelect
            v-model:value="kind"
            :options="KIND_OPTIONS"
            size="small"
            :consistent-menu-width="false"
          />

          <div v-if="kind === 'custom'" class="mt-2 flex flex-col gap-2 rounded border border-ms-border bg-ms-bg/40 p-2">
            <NAlert type="info" :bordered="false" :show-icon="false">
              <div class="text-[10px] leading-relaxed">
                <code class="text-ms-accent">{base}</code> 会展开为不含版本段的主机地址
                （如 <code class="text-ms-accent">https://relay.example.com</code>）；
                需要完整地址时用 <code class="text-ms-accent">{baseUrl}</code>。
              </div>
            </NAlert>

            <NInput
              v-model:value="urlTemplate"
              size="small"
              placeholder="{base}/v1/dashboard/billing/subscription"
            />
            <div class="flex items-center gap-2">
              <NSelect
                v-model:value="method"
                :options="METHOD_OPTIONS"
                size="small"
                style="width: 96px"
                :consistent-menu-width="false"
              />
              <span class="text-[10px] text-ms-text-faint">额外请求头（每行一条 Name: Value）</span>
            </div>
            <NInput
              v-model:value="headersText"
              type="textarea"
              :rows="2"
              size="small"
              placeholder="X-Custom-Auth: abc"
            />
            <div class="flex items-center gap-2">
              <NInput v-model:value="totalPath" size="small" placeholder="总额度路径 hard_limit_usd" />
              <NInput v-model:value="usedPath" size="small" placeholder="已用路径 total_usage" />
            </div>
            <div class="flex items-center gap-2">
              <NInput v-model:value="balancePath" size="small" placeholder="直接余额路径（可选）" />
              <div class="flex shrink-0 items-center gap-1">
                <span class="text-[10px] text-ms-text-faint">除数</span>
                <NInputNumber
                  v-model:value="unitDivisor"
                  size="small"
                  :min="0.000001"
                  :show-button="false"
                  style="width: 96px"
                />
              </div>
            </div>
            <div class="text-[10px] leading-relaxed text-ms-text-faint">
              「直接余额路径」填了就用它；否则用「总额度 − 已用」。除数用于美分换算（美分填 100）。
            </div>
          </div>
        </div>
      </div>
    </NCard>

    <template #footer>
      <div class="flex justify-end gap-2">
        <NButton size="small" @click="emit('update:show', false)">取消</NButton>
        <NButton
          size="small"
          type="primary"
          :loading="saving"
          :disabled="savingDisabled"
          @click="save"
        >
          保存
        </NButton>
      </div>
    </template>
  </NModal>
</template>
