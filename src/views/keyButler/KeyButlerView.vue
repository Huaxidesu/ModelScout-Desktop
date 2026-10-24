<script setup lang="ts">
/**
 * Key 管家页面骨架：装配布局（工具栏 + 分组列表 + 底部状态汇总条）、
 * 建立与释放 `key-butler://*` 事件订阅，并托管四个对话框
 * （导入 / 加密保存 / 编辑 / 历史抽屉）。
 *
 * 订阅放在组件而不是 store 里：取消函数必须在组件卸载时调用，
 * 放在组件里生命周期最清晰；store 只提供纯数据的 applyXxx。
 */

import { computed, onMounted, onUnmounted, ref } from 'vue'
import { NAlert, NButton, NEmpty, useDialog, useMessage } from 'naive-ui'

import { subscribeKeyButler } from '@/api/events'
import { listModels, ProbeApiError } from '@/api/commands'
import { useProbeStore } from '@/stores/probe'
import { provideListTooltip } from '@/composables/useScrollSafeTooltip'
import { useKeyButlerStore } from '@/stores/keyButler'
import type { KeyEntry, ModelBrief, ModelInfo } from '@/types/contracts'

import KeyToolbar from './KeyToolbar.vue'
import KeyGroupSection from './KeyGroupSection.vue'
import KeyImportDialog from './KeyImportDialog.vue'
import KeyBatchEditDialog from './KeyBatchEditDialog.vue'
import VaultDialog from './VaultDialog.vue'
import BalanceEndpointDialog from './BalanceEndpointDialog.vue'
import BalanceHistoryDrawer from './BalanceHistoryDrawer.vue'

const store = useKeyButlerStore()
const message = useMessage()
const dialog = useDialog()

/** 向上抛出跨页动作（由 App.vue 协调：切页 / 打开一键填写） */
const emit = defineEmits<{
  (e: 'launch-probe', entry: KeyEntry): void
  (e: 'open-fill'): void
}>()

/** 「填入配置」进行中（拉模型清单时按钮转圈） */
const fillingConfig = ref(false)

/** ModelBrief（列表接口的轻量条目）→ 全量 ModelInfo（未知项按未探测处理） */
function briefToModelInfo(brief: ModelBrief): ModelInfo {
  const declared = brief.declared
  return {
    id: brief.id,
    displayName: brief.displayName || brief.id,
    ownedBy: brief.ownedBy,
    created: brief.created,
    supportsTools:
      declared?.supportsTools === true ? 'supported'
      : declared?.supportsTools === false ? 'unsupported'
      : 'unknown',
    supportsVision:
      declared?.supportsVision === true ? 'supported'
      : declared?.supportsVision === false ? 'unsupported'
      : 'unknown',
    supportsStreaming: 'unknown',
    isReasoning: declared?.supportsReasoning === true,
    reasoningEvidence: '接口声明（模型清单）',
    contextLimit: declared?.contextLimit ?? 0,
    maxOutputLimit: declared?.maxOutputLimit ?? 0,
    contextLimitSource: 'unknown',
    maxOutputLimitSource: 'unknown',
    capsSource: declared?.contextLimit != null ? '接口声明（模型清单）' : '未探测（来自 Key 管家填入）',
    errors: [],
    elapsedMs: 0,
    finalStage: 'pending',
  }
}

/**
 * 「填入配置」：用这条 Key 拉取模型清单 → 注入探测页的预览上下文 →
 * 打开「一键填写」，让用户选 OpenCode / dsh 写入（复用参数调整与对比预览）。
 */
async function fillConfig(entry: KeyEntry): Promise<void> {
  if (!entry.baseUrl || fillingConfig.value) return
  let plain = ''
  try {
    plain = await store.reveal(entry.id)
  } catch (error) {
    message.error(error instanceof ProbeApiError ? error.message : String(error))
    return
  }
  if (!plain) {
    message.warning('没有取到 Key 内容。')
    return
  }

  fillingConfig.value = true
  try {
    const probeStore = useProbeStore()
    Object.assign(probeStore.config, {
      baseUrl: entry.baseUrl,
      apiKey: plain,
      apiKeySource: 'plain' as const,
      apiKeyEnvVar: '',
      apiKeyCredential: '',
    })
    const result = await listModels(probeStore.config)
    if (result.models.length === 0) {
      message.warning('该端点没有返回任何模型；请检查 Base URL 或 Key 后重试。', { duration: 6000 })
      return
    }
    probeStore.models = result.models.map(briefToModelInfo)
    probeStore.selectedIds = probeStore.models.map((m) => m.id)
    probeStore.hasRun = true
    message.success(
      `已用 ${entry.maskedKey} 获取到 ${result.models.length} 个模型，选择目标工具写入配置`,
      { duration: 5000 },
    )
    emit('open-fill')
  } catch (error) {
    message.error(error instanceof ProbeApiError ? error.message : String(error))
  } finally {
    fillingConfig.value = false
  }
}

/* ---- 对话框状态 ---- */

const showImport = ref(false)
const showVault = ref(false)
const showEdit = ref(false)
const showBatchEdit = ref(false)
const showHistory = ref(false)
const editingEntry = ref<KeyEntry | null>(null)
const historyEntry = ref<KeyEntry | null>(null)

/* ---- 事件订阅 ---- */

/** 取消订阅函数；由 onMounted 赋值 */
let teardown: (() => Promise<void>) | null = null

onMounted(async () => {
  try {
    await store.load()
  } catch (error) {
    message.error(error instanceof ProbeApiError ? error.message : String(error))
  }

  teardown = await subscribeKeyButler({
    onProgress: (payload) => store.applyProgress(payload),
    onEntryDone: (payload) => store.applyEntryDone(payload),
    onFinished: (payload) => {
      store.applyFinished(payload)
      if (payload.status === 'cancelled') {
        message.warning(`检测已停止：完成 ${payload.checked}/${payload.total}`)
      } else {
        const summary = payload.summary
        message.success(
          `检测完成：可用 ${summary.active} · 余额不足 ${summary.insufficient} · 失效 ${summary.invalid} · 查询失败 ${summary.error}`,
        )
      }
    },
  })
})

onUnmounted(() => {
  if (teardown) void teardown()
  teardown = null
})

/* ---- 交互 ---- */

/** 检测单条 */
async function checkOne(entry: KeyEntry): Promise<void> {
  try {
    await store.check([entry.id])
  } catch (error) {
    message.error(error instanceof ProbeApiError ? error.message : String(error))
  }
}

/** 勾选 / 取消勾选单条 */
function toggleSelect(entry: KeyEntry, selected: boolean): void {
  if (selected) {
    if (!store.isSelected(entry.id)) store.toggleSelected(entry.id)
  } else if (store.isSelected(entry.id)) {
    store.toggleSelected(entry.id)
  }
}

/** 打开历史抽屉 */
function openHistory(entry: KeyEntry): void {
  historyEntry.value = entry
  showHistory.value = true
}

/**
 * 用某条 Key 发起探测：交给 App.vue 协调（切页 + 预填 + 启动 + 回写结果）。
 */
function launchProbe(entry: KeyEntry): void {
  if (!entry.baseUrl) {
    message.warning('这条 Key 还没有设置 Base URL，先点「编辑」补一个。', { duration: 5000 })
    return
  }
  emit('launch-probe', entry)
}

/** 打开编辑对话框 */
function openEdit(entry: KeyEntry): void {
  editingEntry.value = entry
  showEdit.value = true
}

/** 删除（二次确认） */
function confirmDelete(entry: KeyEntry): void {
  dialog.warning({
    title: '删除这条 Key？',
    content: `${entry.maskedKey} 及其余额历史都会被删除，无法恢复。`,
    positiveText: '删除',
    negativeText: '取消',
    onPositiveClick: async () => {
      try {
        await store.remove(entry.id)
        message.success('已删除')
      } catch (error) {
        message.error(error instanceof ProbeApiError ? error.message : String(error))
      }
    },
  })
}

/** 汇总条文案 */
const summaryText = computed(() => {
  const { active, insufficient, invalid, error, unchecked } = store.stats
  const parts = [
    `可用 ${active}`,
    `余额不足 ${insufficient}`,
    `已失效 ${invalid}`,
    `查询失败 ${error}`,
  ]
  if (unchecked > 0) parts.push(`未检测 ${unchecked}`)
  return parts.join(' · ')
})

/**
 * 列表行状态 tooltip 的浮层守卫：绑定列表滚动（收起浮层并锁定悬停，
 * 防止浮层滚出列表边缘或反复重开竞速），经 provide 注入 KeyRow。
 * 详见 useScrollSafeTooltip。
 */
const rowTooltips = provideListTooltip()
</script>

<template>
  <div class="flex min-h-0 flex-1 flex-col">
    <!-- 工具栏 -->
    <KeyToolbar
      class="shrink-0"
      @import="showImport = true"
      @vault="showVault = true"
      @batch-edit="showBatchEdit = true"
    />

    <!-- 锁定提示 -->
    <NAlert
      v-if="store.isLocked"
      class="mx-3 mt-2 shrink-0"
      :type="store.vault.systemKey ? 'error' : 'warning'"
      :bordered="false"
      :title="
        store.vault.systemKey
          ? '本机保存的数据无法在当前 Windows 账户下解开'
          : '本地保存已启用，但当前未解锁'
      "
    >
      <div class="flex items-center justify-between text-[11px]">
        <span v-if="store.vault.systemKey">
          这份数据是「无密码保存」创建的，密钥绑定到创建时的 Windows 账户 ——
          换账户或换电脑后就解不开了，没有密码可以输入。
        </span>
        <span v-else>解锁后才能看到 Key 列表并继续导入 / 检测。</span>
        <NButton
          size="tiny"
          :type="store.vault.systemKey ? 'error' : 'warning'"
          ghost
          @click="showVault = true"
        >
          {{ store.vault.systemKey ? '查看说明' : '输入主密码解锁' }}
        </NButton>
      </div>
    </NAlert>

    <!-- 列表 -->
    <!--
      @scroll / @mousemove：滚动时收起行 tooltip 并锁定悬停，
      鼠标真正移动后才恢复；机制见脚本中 rowTooltips 的注释。
    -->
    <div
      class="min-h-0 flex-1 overflow-y-auto px-3 py-2"
      @scroll="rowTooltips.closeOnScroll"
      @mousemove="rowTooltips.unlockHover"
    >
      <!-- 空态 -->
      <div
        v-if="store.entries.length === 0 && !store.loading"
        class="ms-panel ms-scanlines flex h-full flex-col items-center justify-center gap-4"
      >
        <div class="text-4xl opacity-20">🔑</div>
        <div class="text-sm text-ms-text-dim">还没有 Key</div>
        <div class="max-w-md text-center text-xs leading-relaxed text-ms-text-faint">
          点「批量粘贴」把一批 Key 整段粘进来：默认按形态自动识别平台，
          也可以在导入时手动指定厂商（Moonshot、硅基流动这类 Key 形态和
          通用中转一样，只能手动指定）。<br />
          默认不落盘 —— Key 只存在本次会话内存里，关闭应用即清空。
        </div>
        <NButton type="primary" size="medium" @click="showImport = true">批量粘贴</NButton>
      </div>

      <!-- 分组 -->
      <template v-else>
        <KeyGroupSection
          v-for="group in store.grouped"
          :key="group.platform"
          :platform="group.platform"
          :label="group.label"
          :entries="group.entries"
          :balance-sum="group.balanceSum"
          :currency="group.currency"
          :locked="store.isLocked"
          :checking="store.checking"
          :selected-ids="store.selectedIds"
          @check="checkOne"
          @history="openHistory"
          @edit="openEdit"
          @delete="confirmDelete"
          @select="toggleSelect"
          @select-group="store.setSelected"
          @probe="launchProbe"
          @fill-config="fillConfig"
        />

        <NEmpty
          v-if="store.grouped.length === 0"
          description="没有符合当前过滤条件的 Key"
          class="py-16"
        />
      </template>
    </div>

    <!-- 状态汇总条 -->
    <footer
      class="flex shrink-0 items-center justify-between border-t border-ms-border bg-ms-panel px-3 py-1.5 text-[11px] text-ms-text-faint"
    >
      <span>{{ summaryText }}</span>
      <span v-if="store.entries.length > 0">
        共 {{ store.entries.length }} 条
        <span v-if="store.filtered.length !== store.entries.length">
          （当前筛选 {{ store.filtered.length }}）
        </span>
      </span>
    </footer>

    <!-- 对话框 -->
    <KeyImportDialog v-model:show="showImport" />
    <KeyBatchEditDialog v-model:show="showBatchEdit" :ids="store.selectedIds" />
    <VaultDialog v-model:show="showVault" />
    <BalanceEndpointDialog v-model:show="showEdit" :entry="editingEntry" />
    <BalanceHistoryDrawer v-model:show="showHistory" :entry="historyEntry" />
  </div>
</template>
