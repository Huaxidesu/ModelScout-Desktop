<script setup lang="ts">
/**
 * 本地保存对话框。四种形态（由 store 的 vault 状态决定）：
 *   1. 未启用：选保存方式（主密码保存 / 无密码保存）；
 *   2. 已启用未解锁：输入主密码解锁；
 *   3. 已启用已解锁：展示状态 + 关闭本地保存（二次确认）；
 *   4. 系统密钥但解不开：说明「这份数据不属于当前 Windows 账户」。
 *
 * 无密码保存依然是加密的：「不用输密码」靠的是把随机数据密钥用
 * Windows DPAPI 绑定当前账户，落盘内容仍是 AES-256-GCM 密文；
 * 不得实现为「写明文 JSON」，界面上也不能给用户「无密码 = 不加密」的暗示。
 *
 * 启用前需要阅读满 10 秒的原因：无密码模式不可迁移——换电脑、重装、
 * 换 Windows 账户后数据将无法解开（没有主密码可输入）。确认按钮
 * 等倒计时结束才可用，确保该代价被用户读到。
 */

import { computed, onUnmounted, ref, watch } from 'vue'
import { NAlert, NButton, NCard, NInput, NModal, NTag, useDialog, useMessage } from 'naive-ui'

import { useKeyButlerStore } from '@/stores/keyButler'
import { ProbeApiError } from '@/api/commands'

const props = defineProps<{ show: boolean }>()
const emit = defineEmits<{ (e: 'update:show', value: boolean): void }>()

const store = useKeyButlerStore()
const message = useMessage()
const dialog = useDialog()

/** 最短主密码长度（与后端 vault::MIN_PASSWORD_LEN 保持一致） */
const MIN_PASSWORD_LEN = 8

/** 无密码保存的强制阅读时长（秒） */
const NOTICE_SECONDS = 10

const password = ref('')
const confirmPassword = ref('')
const busy = ref(false)

/** 十秒注意事项弹窗 */
const showNotice = ref(false)
/** 剩余秒数；> 0 时不允许确认 */
const remaining = ref(NOTICE_SECONDS)
let timer: number | null = null

/** 当前形态 */
const mode = computed<'setup' | 'unlock' | 'manage' | 'orphan'>(() => {
  if (!store.vault.enabled) return 'setup'
  if (!store.vault.unlocked) return store.vault.systemKey ? 'orphan' : 'unlock'
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

/** 倒计时是否已走完 */
const noticeDone = computed(() => remaining.value <= 0)

function stopTimer(): void {
  if (timer !== null) {
    window.clearInterval(timer)
    timer = null
  }
}

/** 打开十秒注意事项并开始倒计时 */
function openNotice(): void {
  showNotice.value = true
  remaining.value = NOTICE_SECONDS
  stopTimer()
  timer = window.setInterval(() => {
    remaining.value -= 1
    if (remaining.value <= 0) stopTimer()
  }, 1000)
}

/** 关闭注意事项（未确认即退出，不留任何副作用） */
function closeNotice(): void {
  stopTimer()
  showNotice.value = false
}

onUnmounted(stopTimer)

/** 关闭整个对话框时清空输入与倒计时，避免密码/状态残留 */
watch(
  () => props.show,
  (show) => {
    if (!show) {
      password.value = ''
      confirmPassword.value = ''
      closeNotice()
    }
  },
)

/** 启用主密码保存 */
async function enable(): Promise<void> {
  if (!canSetup.value) return
  busy.value = true
  try {
    await store.enableVault(password.value)
    message.success('已启用本地保存，Key 已加密写入本机')
    password.value = ''
    confirmPassword.value = ''
  } catch (error) {
    await handleEnableError(error)
  } finally {
    busy.value = false
  }
}

/** 确认启用无密码保存（倒计时结束后才可点） */
async function enableSystemKey(): Promise<void> {
  if (!noticeDone.value) return
  busy.value = true
  try {
    await store.enableVaultSystemKey()
    message.success('已启用无密码保存 —— 数据仍是加密的，只是不需要你输入密码')
    closeNotice()
  } catch (error) {
    if (await handleEnableError(error)) closeNotice()
  } finally {
    busy.value = false
  }
}

/**
 * 处理启用失败。
 *
 * 「本地保存已经启用」单独对待：它几乎总意味着界面状态过期（后端已启用、
 * 前端还停在未启用表单 —— 典型触发是启动时的状态加载晚于用户操作返回）。
 * 此时按已启用收尾并刷新状态，而不是把一条用户无法行动的报错甩给用户。
 *
 * @returns 是否应视为「已启用」收尾（关闭注意事项弹窗）
 */
async function handleEnableError(error: unknown): Promise<boolean> {
  const text = error instanceof ProbeApiError ? error.message : String(error)
  if (text.includes('已经启用')) {
    try {
      await store.refreshVault()
    } catch {
      // 刷新失败也不升级为报错：保持当前状态，让用户重开对话框再看
    }
    if (store.vault.enabled) {
      message.info('本地保存其实已经启用了，界面状态已刷新。')
      return true
    }
  }
  message.error(text)
  return false
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

/** 关闭本地保存（二次确认） */
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
    style="width: 620px"
    @update:show="emit('update:show', $event)"
  >
    <NCard :bordered="false" size="small" title="本地保存">
      <!-- ① 未启用：两种方式并列，让用户自己选 -->
      <template v-if="mode === 'setup'">
        <!-- 方式 A：主密码 -->
        <div class="rounded border border-ms-border p-3">
          <div class="mb-1 flex items-center gap-2">
            <span class="text-[12px] font-semibold text-ms-text">主密码保存</span>
            <NTag size="tiny" :bordered="false" type="success">推荐</NTag>
          </div>
          <div class="mb-2 text-[11px] leading-relaxed text-ms-text-dim">
            Key 由主密码派生密钥（argon2id）加密，再用 AES-256-GCM 写入本机。
            <br />
            <strong class="text-ms-warning">
              主密码无法找回、没有恢复码 —— 忘记主密码等于这份数据无法恢复。
            </strong>
          </div>
          <div class="flex flex-col gap-2">
            <NInput
              v-model:value="password"
              type="password"
              show-password-on="click"
              size="small"
              placeholder="设置主密码（至少 8 位）"
            />
            <NInput
              v-model:value="confirmPassword"
              type="password"
              show-password-on="click"
              size="small"
              placeholder="再输一次确认"
              :status="passwordMismatch ? 'error' : undefined"
            />
            <div v-if="passwordMismatch" class="text-[11px] text-ms-danger">两次输入不一致。</div>
            <div>
              <NButton size="small" type="primary" :disabled="!canSetup" :loading="busy" @click="enable">
                启用并加密
              </NButton>
            </div>
          </div>
        </div>

        <!-- 方式 B：无密码 -->
        <div class="mt-3 rounded border border-ms-border p-3">
          <div class="mb-1 flex items-center gap-2">
            <span class="text-[12px] font-semibold text-ms-text">无密码保存</span>
            <NTag size="tiny" :bordered="false" type="info">免输入</NTag>
          </div>
          <div class="mb-2 text-[11px] leading-relaxed text-ms-text-dim">
            打开应用就能直接读到 Key，不用输任何东西。
            <strong class="text-ms-text">磁盘上依然是加密的</strong> ——
            加密密钥由 Windows 数据保护接口（DPAPI）绑定到当前 Windows 账户。
            <br />
            代价是<strong class="text-ms-warning">不可迁移</strong>：换电脑、重装系统或换
            Windows 账户后，这份数据将无法恢复。
          </div>
          <NButton size="small" :disabled="busy" @click="openNotice">启用无密码保存</NButton>
        </div>
      </template>

      <!-- ② 已启用未解锁（主密码模式） -->
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

      <!-- ④ 系统密钥模式但解不开：换账户 / 换电脑 -->
      <template v-else-if="mode === 'orphan'">
        <NAlert type="error" :bordered="false" title="这份数据不属于当前 Windows 账户">
          <div class="text-[11px] leading-relaxed">
            本机密文库是「无密码保存」创建的，它的加密密钥被绑定到创建时的
            Windows 账户。当前账户无法解开 —— 常见原因是换了电脑、重装了系统，
            或换了 Windows 登录账户。
            <br />
            没有主密码可以输入，因此**无法恢复**。若你确认不再需要它，
            可以关闭本地保存后重新导入 Key。
          </div>
        </NAlert>
      </template>

      <!-- ③ 已启用已解锁 -->
      <template v-else>
        <NAlert
          :type="store.vault.systemKey ? 'info' : 'success'"
          :bordered="false"
          :title="store.vault.systemKey ? '无密码保存已启用' : '主密码保存已启用并已解锁'"
        >
          <div class="text-[11px] leading-relaxed">
            当前共 {{ store.vault.entryCount }} 条 Key 已加密保存在本机。
            之后的导入、删除、改配置都会自动重新加密落盘（每次使用新的随机 nonce）。
            <template v-if="store.vault.systemKey">
              <br />
              加密密钥由 Windows 数据保护接口绑定当前账户，因此打开应用时无需输入密码；
              磁盘上始终只有密文。
            </template>
          </div>
        </NAlert>
      </template>
    </NCard>

    <template #footer>
      <div class="flex justify-end gap-2">
        <NButton v-if="mode === 'manage'" size="small" type="error" ghost @click="disable">
          关闭本地保存
        </NButton>
        <NButton v-if="mode === 'orphan'" size="small" type="error" ghost @click="disable">
          删除这份无法解开的数据
        </NButton>
        <NButton size="small" @click="emit('update:show', false)">关闭</NButton>
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

  <!-- 十秒注意事项：独立一层弹窗，倒计时结束前无法确认 -->
  <NModal
    :show="showNotice"
    preset="card"
    style="width: 560px"
    :mask-closable="false"
    @update:show="(value) => (value ? undefined : closeNotice())"
  >
    <NCard :bordered="false" size="small" title="启用「无密码保存」前请先阅读">
      <NAlert type="warning" :bordered="false">
        <div class="text-[11px] leading-relaxed">
          <p class="mb-1">
            <strong>1. 它依然是加密的。</strong>
            磁盘上不会出现明文 —— Key 仍由 AES-256-GCM 加密，
            加密密钥由 Windows 数据保护接口（DPAPI）绑定到当前 Windows 账户。
          </p>
          <p class="mb-1">
            <strong>2. 它挡不住什么。</strong>
            同一个 Windows 账户下运行的任何程序、以及任何能用你账户登录的人，
            都能解开这份数据。它防的是「文件被拷走 / 备份泄漏 / 换个账户打开」，
            <strong>不防本机恶意软件，也不防别人直接用你的电脑</strong>。
          </p>
          <p class="mb-1">
            <strong>3. 它不可迁移。</strong>
            换电脑、重装系统、更换 Windows 登录账户之后，
            这份数据将<strong>无法恢复</strong> —— 没有主密码可以输入，也没有恢复码。
          </p>
          <p class="mb-0">
            <strong>4. 想更安全就用主密码保存。</strong>
            如果你会把电脑借给别人、或者担心本机安全，
            请改用主密码保存：它要求每次启动输入密码，但换机也能带走。
          </p>
        </div>
      </NAlert>

      <template #footer>
        <div class="flex items-center justify-between gap-2">
          <span class="text-[11px] text-ms-text-faint">
            <template v-if="!noticeDone">请阅读 {{ NOTICE_SECONDS }} 秒（剩余 {{ remaining }} 秒）</template>
            <template v-else>已阅读完毕，可以确认。</template>
          </span>
          <div class="flex gap-2">
            <NButton size="small" @click="closeNotice">取消</NButton>
            <NButton
              size="small"
              type="primary"
              :disabled="!noticeDone"
              :loading="busy"
              @click="enableSystemKey"
            >
              {{ noticeDone ? '我已阅读并了解风险，启用' : `请等待 ${remaining}s` }}
            </NButton>
          </div>
        </div>
      </template>
    </NCard>
  </NModal>
</template>
