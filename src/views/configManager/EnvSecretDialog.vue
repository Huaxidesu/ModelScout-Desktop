<script setup lang="ts">
/**
 * 环境变量密钥编辑对话框（配置管理子模块与生成配置共用）。
 *
 * 两种用法：
 *   - 编辑/新建：输入变量名 + 值 → setEnvSecret（本应用内立即生效 +
 *     Windows 用户级持久化，新开的程序才能读到持久值）；
 *   - 名称选择：`picker` 模式只取变量名（用于生成配置的「编辑变量名」），
 *     下拉里带出本机已有的疑似密钥变量（掩码预览），也可直接输入新名字。
 *
 * 敏感处理：值输入框默认遮蔽（可切换明文），编辑已有变量时不回显旧值
 * （后端就不返回完整值），保存需要重新输入完整值。
 */

import { computed, ref, watch } from 'vue'
import { NAlert, NAutoComplete, NButton, NInput, NModal, NTag } from 'naive-ui'

import { listEnvSecrets, setEnvSecret, ProbeApiError } from '@/api/commands'
import type { EnvSecretInfo } from '@/types/contracts'

const props = defineProps<{
  show: boolean
  /** 对话框标题 */
  title?: string
  /** 只选名字不填值（生成配置「编辑变量名」模式） */
  picker?: boolean
  /** 初始/锁定的变量名（联动场景从 provider 带过来） */
  initialName?: string
  /** 锁定变量名（编辑已有引用时不可改名） */
  lockName?: boolean
  /** 仅允许从本机已有变量里选（picker 模式可选约束，默认 false） */
  existingOnly?: boolean
}>()

const emit = defineEmits<{
  (e: 'update:show', value: boolean): void
  /** 保存成功（picker 模式 = 确认名字） */
  (e: 'saved', name: string): void
}>()

const nameInput = ref('')
const valueInput = ref('')
const saving = ref(false)
const errorText = ref('')
/** 本机已有的疑似密钥变量（打开时拉一次，供自动补全与展示） */
const machineSecrets = ref<EnvSecretInfo[]>([])
/** 最近一次保存结果（父组件在 saved 事件后读取） */
const savedNote = ref<import('@/types/contracts').EnvSecretWriteResult | null>(null)

watch(
  () => props.show,
  (show) => {
    if (!show) return
    nameInput.value = props.initialName ?? ''
    valueInput.value = ''
    errorText.value = ''
    // 拉本机变量清单（失败不阻塞手输）
    listEnvSecrets()
      .then((items) => {
        machineSecrets.value = items
      })
      .catch(() => {
        machineSecrets.value = []
      })
  },
  { immediate: true },
)

const NAME_RE = /^[A-Za-z_][A-Za-z0-9_]*$/

/** 名字自动补全候选（本机已有变量） */
const nameOptions = computed(() =>
  machineSecrets.value.map((s) => ({
    label: `${s.name}（${s.preview}）`,
    value: s.name,
  })),
)

/** 选中/输入的名字是否是本机已有变量 */
const nameExists = computed(() =>
  machineSecrets.value.some((s) => s.name === nameInput.value.trim()),
)

/** 能否保存 */
const canSave = computed(() => {
  if (saving.value) return false
  if (!NAME_RE.test(nameInput.value.trim())) return false
  if (props.picker) return true
  return valueInput.value.length > 0
})

async function save(): Promise<void> {
  const name = nameInput.value.trim()
  if (!NAME_RE.test(name)) {
    errorText.value = '变量名须以字母或下划线开头，只含字母、数字、下划线。'
    return
  }
  if (props.picker) {
    emit('saved', name)
    emit('update:show', false)
    return
  }
  if (!valueInput.value) {
    errorText.value = '请输入密钥值。'
    return
  }
  saving.value = true
  errorText.value = ''
  try {
    const result = await setEnvSecret(name, valueInput.value)
    savedNote.value = result
    emit('saved', name)
    emit('update:show', false)
  } catch (error) {
    errorText.value = error instanceof ProbeApiError || error instanceof Error ? error.message : String(error)
  } finally {
    saving.value = false
  }
}

defineExpose({ savedNote })

function close(): void {
  emit('update:show', false)
}
</script>

<template>
  <NModal
    :show="props.show"
    preset="card"
    :title="props.title ?? (props.picker ? '环境变量名' : '环境变量密钥')"
    style="width: 460px"
    :mask-closable="false"
    @update:show="(v: boolean) => emit('update:show', v)"
  >
    <div class="flex flex-col gap-3">
      <NAlert v-if="machineSecrets.length > 0" type="info" :bordered="false" style="border-radius: 0">
        <div class="text-[11px] leading-relaxed">
          本机检测到 {{ machineSecrets.length }} 个疑似密钥的环境变量，输入名字时可自动补全。
        </div>
      </NAlert>

      <div class="flex flex-col gap-1">
        <span class="text-[11px] text-ms-text-dim">变量名</span>
        <NAutoComplete
          v-model:value="nameInput"
          :options="nameOptions"
          :disabled="props.lockName"
          placeholder="如 OPENAI_API_KEY"
          spellcheck="false"
          :input-props="{ spellcheck: 'false' }"
          @keyup.enter="save()"
        />
        <div class="flex items-center gap-1.5">
          <NTag v-if="nameExists" size="tiny" :bordered="false" type="info">本机已有（保存将覆盖其值）</NTag>
          <NTag v-if="props.lockName" size="tiny" :bordered="false">引用锁定</NTag>
        </div>
      </div>

      <div v-if="!props.picker" class="flex flex-col gap-1">
        <span class="text-[11px] text-ms-text-dim">密钥值</span>
        <NInput
          v-model:value="valueInput"
          type="password"
          show-password-on="click"
          placeholder="sk-...（不会回显已有值，保存即覆盖）"
          spellcheck="false"
          @keyup.enter="save()"
        />
      </div>

      <NAlert v-if="errorText" type="error" :bordered="false" style="border-radius: 0">
        <div class="text-[11px]">{{ errorText }}</div>
      </NAlert>

      <div class="text-[11px] leading-relaxed text-ms-text-faint">
        <template v-if="!props.picker">
          保存后<strong>本应用内立即生效</strong>（探测 / 连通测试直接可用）；
          Windows 会同时写入用户级持久环境，<strong>新开的</strong>终端与程序
          （opencode / dsh）重启后才能读到，已在运行的不会热更新。
        </template>
        <template v-else>
          填写的变量名将以 <code>{env:变量名}</code> 形式写入配置；
          目标程序须能在自己的环境里读到该变量。
        </template>
      </div>
    </div>

    <template #footer>
      <div class="flex justify-end gap-2">
        <NButton size="small" quaternary @click="close()">取消</NButton>
        <NButton size="small" type="primary" :loading="saving" :disabled="!canSave" @click="save()">
          {{ props.picker ? '确定' : '保存' }}
        </NButton>
      </div>
    </template>
  </NModal>
</template>
