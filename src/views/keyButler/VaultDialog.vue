<script setup lang="ts">
/**
 * ============================================================================
 * Key 管家 —— 加密保存对话框
 * ============================================================================
 *
 * 三种形态（由 store 的 vault 状态决定）：
 *   ① 未启用 → 说明加密方式与「主密码不可找回」，设置主密码（两次确认）
 *   ② 已启用未解锁 → 输入主密码解锁
 *   ③ 已启用已解锁 → 展示状态 + 关闭本地保存（二次确认）
 *
 * 【为什么主密码要强调「不可找回」】
 * 密钥由主密码派生（argon2id），程序里**没有**任何后门或恢复码。
 * 忘记主密码就等于密文库报废 —— 这句话必须在用户设置密码之前说清楚，
 * 而不是等他忘了才在文档里看到。
 * ============================================================================
 */

import { computed, ref, watch } from 'vue'
import { NAlert, NButton, NCard, NInput, NModal, useDialog, useMessage } from 'naive-ui'

import { useKeyButlerStore } from '@/stores/keyButler'
import { ProbeApiError } from '@/api/commands'

const props = defineProps<{ show: boolean }>()
const emit = defineEmits<{ (e: 'update:show', value: boolean): void }>()

const store = useKeyButlerStore()
const message = useMessage()
const dialog = useDialog()

/** 最短主密码长度（与后端 vault::MIN_PASSWORD_LEN 保持一致） */
const MIN_PASSWORD_LEN = 8

const password = ref('')
const confirmPassword = ref('')
const busy = ref(false)

/** 当前形态 */
const mode = computed<'setup' | 'unlock' | 'manage'>(() => {
  if (!store.vault.enabled) return 'setup'
  if (!store.vault.unlocked) return 'unlock'
  return 'manage'
})

/** 两次输入是否一致 */
const passwordMismatch = computed(
  () => confirmPassword.value.length > 0 && password.value !== confirmPassword.value,
)

/** 设置密码按钮是否可用 */
const canSetup = computed(
  () => password.value.length >= MIN_PASSWORD_LEN && password.value === confirmPassword.value,
)

/** 关闭时清空输入，避免密码残留在界面上 */
watch(
  () => props.show,
  (show) => {
    if (!show) {
      password.value = ''
      confirmPassword.value = ''
    }
  },
)

/** 启用加密保存 */
async function enable(): Promise<void> {
  if (!canSetup.value) return
  busy.value = true
  try {
    await store.enableVault(password.value)
    message.success('已启用加密保存，Key 已加密写入本机')
    password.value = ''
    confirmPassword.value = ''
  } catch (error) {
    message.error(error instanceof ProbeApiError ? error.message : String(error))
  } finally {
    busy.value = false
  }
}

/** 解锁 */
async function unlock(): Promise<void> {
  if (!password.value) return
  busy.value = true
  try {
    await store.unlockVault(password.value)
    message.success('已解锁')
    password.value = ''
  } catch (error) {
    message.error(error instanceof ProbeApiError ? error.message : String(error))
  } finally {
    busy.value = false
  }
}

/** 关闭加密保存（二次确认） */
function disable(): void {
  dialog.warning({
    title: '关闭本地保存？',
    content:
      '本机保存的密文文件会被删除。当前会话里已经导入的 Key 仍然可用，但关闭应用后就再也找不回来了。',
    positiveText: '仍然关闭',
    negativeText: '取消',
    onPositiveClick: async () => {
      try {
        await store.disableVault()
        message.success('已关闭本地保存，本机密文已删除')
      } catch (error) {
        message.error(error instanceof ProbeApiError ? error.message : String(error))
      }
    },
  })
}
</script>

<template>
  <NModal
    :show="show"
    preset="card"
    style="width: 560px"
    @update:show="emit('update:show', $event)"
  >
    <NCard :bordered="false" size="small" title="本地加密保存">
      <!-- ① 未启用：先讲清楚风险再让用户设密码 -->
      <template v-if="mode === 'setup'">
        <NAlert type="warning" :bordered="false" title="默认不保存，启用后需牢记主密码">
          <div class="text-[11px] leading-relaxed">
            启用后，Key 会用你设置的主密码派生密钥（argon2id）加密，
            再用 AES-256-GCM 写入本机应用数据目录。
            <br />
            <strong class="text-ms-warning">
              主密码无法找回、没有恢复码 —— 忘记主密码等于这份数据无法恢复。
            </strong>
          </div>
        </NAlert>

        <div class="mt-3 flex flex-col gap-2">
          <NInput
            v-model:value="password"
            type="password"
            show-password-on="click"
            placeholder="设置主密码（至少 8 位）"
          />
          <NInput
            v-model:value="confirmPassword"
            type="password"
            show-password-on="click"
            placeholder="再输一次确认"
            :status="passwordMismatch ? 'error' : undefined"
          />
          <div v-if="passwordMismatch" class="text-[11px] text-ms-danger">两次输入不一致。</div>
        </div>
      </template>

      <!-- ② 已启用未解锁 -->
      <template v-else-if="mode === 'unlock'">
        <NAlert type="info" :bordered="false" title="密文库已存在，请输入主密码解锁">
          <div class="text-[11px] leading-relaxed">
            解锁后才能看到 Key 列表并继续导入 / 检测。
          </div>
        </NAlert>

        <div class="mt-3">
          <NInput
            v-model:value="password"
            type="password"
            show-password-on="click"
            placeholder="主密码"
            @keyup.enter="unlock"
          />
        </div>
      </template>

      <!-- ③ 已启用已解锁 -->
      <template v-else>
        <NAlert type="success" :bordered="false" title="加密保存已启用并已解锁">
          <div class="text-[11px] leading-relaxed">
            当前共 {{ store.vault.entryCount }} 条 Key 已加密保存在本机。
            之后的导入、删除、改配置都会自动重新加密落盘（每次使用新的随机 nonce）。
          </div>
        </NAlert>
      </template>
    </NCard>

    <template #footer>
      <div class="flex justify-end gap-2">
        <NButton v-if="mode === 'manage'" size="small" type="error" ghost @click="disable">
          关闭本地保存
        </NButton>
        <NButton size="small" @click="emit('update:show', false)">关闭</NButton>
        <NButton
          v-if="mode === 'setup'"
          size="small"
          type="primary"
          :disabled="!canSetup"
          :loading="busy"
          @click="enable"
        >
          启用并加密
        </NButton>
        <NButton
          v-if="mode === 'unlock'"
          size="small"
          type="primary"
          :disabled="!password"
          :loading="busy"
          @click="unlock"
        >
          解锁
        </NButton>
      </div>
    </template>
  </NModal>
</template>
