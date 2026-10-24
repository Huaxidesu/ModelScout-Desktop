<script setup lang="ts">
/**
 * 根组件：装配全局主题（Naive UI 明/暗 + themeOverrides）、消息/对话框 Provider，
 * 以及整体布局骨架（顶栏 / Tab / 输入区 / 控制条 / 进度与结果 / 配置预览）。
 * 挂载时建立事件订阅，卸载时释放。
 *
 * 主题：设置页可切换 默认 / 浅色 / 深色 / GitHub / 自定义（stores/theme.ts
 * 负责 CSS 变量与持久化）；这里按当前颜色的明暗给 Naive UI 配 darkTheme
 * 或亮色（null），并用同一份颜色构建 themeOverrides。
 *
 * 布局策略：整页固定高度，各区域独立滚动，保证长时间探测时界面稳定，
 * 日志区不会将其他区域顶出屏幕。
 */

import { computed, onMounted, onUnmounted, ref } from 'vue'
import {
  NConfigProvider,
  NMessageProvider,
  NDialogProvider,
  NTag,
  darkTheme,
  zhCN,
  dateZhCN,
} from 'naive-ui'

import AppHeader from '@/components/AppHeader.vue'
import AboutDialog from '@/components/AboutDialog.vue'
import SettingsDialog from '@/components/SettingsDialog.vue'
import ConnectionForm from '@/components/ConnectionForm.vue'
import ProbeControl from '@/components/ProbeControl.vue'
import ProgressPanel from '@/components/ProgressPanel.vue'
import ModelTable from '@/components/ModelTable.vue'
import ConfigPreview from '@/components/ConfigPreview.vue'
import KeyButlerView from '@/views/keyButler/KeyButlerView.vue'
import ConfigManagerView from '@/views/configManager/ConfigManagerView.vue'
import { useProbe } from '@/composables/useProbe'
import { useKeyButlerStore } from '@/stores/keyButler'
import { useThemeStore } from '@/stores/theme'
import { buildNaiveThemeOverrides } from '@/theme/naive'
import { keyButlerRecordProbe } from '@/api/commands'
import type { KeyEntry } from '@/types/contracts'

// ---- 主题：设置页切换,这里按当前颜色构建 Naive UI 覆盖 ----
const theme = useThemeStore()

/** themeOverrides 随主题颜色实时重建(切换即时生效) */
const themeOverrides = computed(() => buildNaiveThemeOverrides(theme.colors))

/** Naive UI 明暗:暗色主题用 darkTheme,亮色主题用 null(默认亮色) */
const naiveTheme = computed(() => (theme.scheme === 'dark' ? darkTheme : null))

// ---- 页面布局状态 ----

/** 底部配置预览区是否展开 */
const showConfig = ref(false)

/** 「关于」弹窗是否显示（由 AppHeader 的入口触发） */
const showAbout = ref(false)

/** 「设置」弹窗是否显示（AppHeader 齿轮入口,主题切换等） */
const showSettings = ref(false)

// ---- 顶部 Tab：探测 / 配置管理 / Key 管家（Beta） ----
// 用 v-show 而非 v-if 切换：v-if 会在每次切 Tab 时整体卸载重建探测页组件，
// 丢失滚动位置与展开状态；v-show 仅切换 display。
const activeTab = ref<'probe' | 'config' | 'keyButler'>('probe')

/** Tab 定义（beta 为 true 时在文案后附加 Beta 徽标） */
const TABS = [
  { value: 'probe' as const, label: '探测', beta: false },
  { value: 'config' as const, label: '配置管理', beta: false },
  { value: 'keyButler' as const, label: 'Key 管家', beta: true },
]

/** 一键填写对话框的外部触发器（时间戳；Key 管家「填入配置」后置位） */
const quickFillTrigger = ref(0)

/**
 * 配置预览是否放大到全屏。
 *
 * 与 `showConfig` 分开是两个独立的维度：
 *   showConfig=false            → 完全不显示
 *   showConfig=true, max=false  → 内嵌在结果表下方
 *   showConfig=true, max=true   → 覆盖整个窗口
 */
const configMaximized = ref(false)

/** Key 管家「填入配置」注入模型后的收尾：切页 + 打开预览 + 弹出一键填写 */
function openFillFromKeyButler(): void {
  activeTab.value = 'probe'
  showConfig.value = true
  quickFillTrigger.value = Date.now()
}

/** 组合式函数：内部已注册 onUnmounted 清理，这里只需触发订阅建立 */
const probe = useProbe()

/** Key 管家 store（跨页协调：从 Key 管家发起探测） */
const keyButler = useKeyButlerStore()

/**
 * 用 Key 管家里的某条密钥发起一次完整探测。
 *
 * 流程（对应需求「Key 管家选择密钥发起探测、记录探测结果」）：
 *   1. reveal 拿到明文（瞬态使用，不落任何状态）；
 *   2. 预填探测配置（Base URL 来自 Key 条目，来源切回明文）并切到探测页；
 *   3. 启动探测；结束后把终态与模型数回写到这条 Key 上。
 */
async function launchProbeFromKey(entry: KeyEntry): Promise<void> {
  if (!entry.baseUrl) return
  if (probe.store.isRunning) {
    // 已有探测在跑：不抢占（预填会污染正在进行的上下文），切过去看进度
    activeTab.value = 'probe'
    return
  }
  let plain = ''
  try {
    plain = await keyButler.reveal(entry.id)
  } catch (error) {
    console.error('[ModelScout] 读取 Key 失败：', error)
    return
  }
  if (!plain) return

  const config = probe.store.config
  config.baseUrl = entry.baseUrl
  config.apiKey = plain
  config.apiKeySource = 'plain'
  config.apiKeyEnvVar = ''
  config.apiKeyCredential = ''
  activeTab.value = 'probe'
  showConfig.value = false

  await probe.start()

  // 回写探测结果（done 时带模型数；failed/cancelled 只记状态）
  const status = probe.store.status
  const modelCount = status === 'done' ? probe.store.models.length : null
  try {
    keyButler.entries = await keyButlerRecordProbe(entry.id, status, modelCount)
  } catch (error) {
    console.error('[ModelScout] 回写探测结果失败：', error)
  }
}

onMounted(async () => {
  await probe.setupSubscriptions()
  // 挂载时做一次状态重同步：处理「前端热重载但后端仍在探测」的情况
  await probe.sync()
})

/**
 * Esc 退出全屏。
 *
 * 仅在全屏态生效：内嵌态下按 Esc 不执行任何操作，
 * 避免用户查看日志时误按导致预览被关闭。
 */
function onKeydown(event: KeyboardEvent): void {
  if (event.key === 'Escape' && configMaximized.value) {
    configMaximized.value = false
  }
}

onMounted(() => window.addEventListener('keydown', onKeydown))
onUnmounted(() => window.removeEventListener('keydown', onKeydown))

/** 「生成配置」按钮是否可用 */
const canGenerate = computed(
  () => probe.store.models.length > 0 && probe.store.selectedIds.length > 0,
)

/** 打开配置预览区（由 ModelTable 通过事件触发） */
function openConfigPreview(): void {
  showConfig.value = true
}

/** 切换放大态 */
function toggleConfigMaximize(): void {
  configMaximized.value = !configMaximized.value
}

/** 彻底关闭预览（全屏态下点「关闭」同样走这里） */
function closeConfigPreview(): void {
  configMaximized.value = false
  showConfig.value = false
}

/**
 * 状态机状态 → 中文标签。
 *
 * 与 Rust 的 `ProbeStatus` 枚举一一对应；
 * 使用对象字面量而非 switch，天然覆盖未知状态的回退分支。
 */
const STATUS_LABELS: Record<string, string> = {
  idle: '空闲',
  running: '探测中',
  cancelling: '正在中止',
  done: '已完成',
  cancelled: '已中止',
  failed: '失败',
}

/** 当前状态的显示文案 */
const statusLabel = computed(
  () => STATUS_LABELS[probe.store.status] ?? probe.store.status,
)
</script>

<template>
  <NConfigProvider
    :theme="naiveTheme"
    :theme-overrides="themeOverrides"
    :locale="zhCN"
    :date-locale="dateZhCN"
  >
    <NMessageProvider :max="4" placement="top-right">
      <NDialogProvider>
        <div class="flex h-screen w-screen flex-col bg-ms-bg text-ms-text">
          <!-- ---- 顶栏 ---- -->
          <AppHeader
            class="shrink-0"
            :status="probe.store.status"
            @open-about="showAbout = true"
            @open-settings="showSettings = true"
          />

          <!-- ---- 页面 Tab ---- -->
          <nav
            class="flex shrink-0 items-center gap-5 border-b border-ms-border bg-ms-panel px-4"
          >
            <button
              v-for="tab in TABS"
              :key="tab.value"
              type="button"
              class="relative flex items-center gap-1.5 py-2 text-[13px] transition-colors"
              :class="
                activeTab === tab.value
                  ? 'text-ms-accent'
                  : 'text-ms-text-dim hover:text-ms-text'
              "
              @click="activeTab = tab.value"
            >
              <span>{{ tab.label }}</span>
              <!-- Beta 徽标：warning 色 + 圆角，与文案间距 6px（gap-1.5） -->
              <NTag v-if="tab.beta" size="small" round type="warning" :bordered="false">
                Beta
              </NTag>
              <span
                v-if="activeTab === tab.value"
                class="absolute inset-x-0 -bottom-px h-0.5 bg-ms-accent"
              />
            </button>
          </nav>

          <!-- ---- 主体：探测页（v-show 切换，原因见脚本中 activeTab 注释） ---- -->
          <div
            v-show="activeTab === 'probe'"
            class="flex min-h-0 flex-1 flex-col gap-3 px-4 py-3"
          >
            <!-- ---- 第一段：输入区 ---- -->
            <ConnectionForm class="shrink-0" :probe="probe" />

            <!-- ---- 第二段：控制条（含错误分类汇总） ---- -->
            <ProbeControl class="shrink-0" :probe="probe" />

            <!-- ---- 第三段：进度 / 结果 双栏 ---- -->
            <div class="flex min-h-0 flex-1 gap-3">
              <!-- 左：进度与日志（探测中或已跑过时显示） -->
              <ProgressPanel
                v-if="probe.store.hasRun"
                class="w-[340px] shrink-0"
                :probe="probe"
              />

              <!-- 右：结果表格 / 配置预览 -->
              <div class="flex min-w-0 flex-1 flex-col gap-3">
                <!-- 尚未跑过任何探测：空态引导 -->
                <div
                  v-if="!probe.store.hasRun"
                  class="ms-panel ms-scanlines flex flex-1 flex-col items-center justify-center gap-4"
                >
                  <div class="text-5xl opacity-20">◈</div>
                  <div class="text-sm text-ms-text-dim">
                    填写上方 Base URL 与 API Key，点击「开始探测」
                  </div>
                  <div class="max-w-md text-center text-xs leading-relaxed text-ms-text-faint">
                    建议先点「测试连接」确认端点可达。<br />
                    探测会逐一检查每个模型的工具调用、附件上传、推理能力与上下文上限。
                  </div>
                </div>

                <!-- 结果表格 -->
                <ModelTable
                  v-else
                  class="min-h-0"
                  :style="showConfig ? 'flex: 0 0 46%' : 'flex: 1 1 auto'"
                  :probe="probe"
                  @generate="openConfigPreview"
                />

                <!-- 配置预览（可折叠 / 可放大） -->
                <ConfigPreview
                  v-if="showConfig && !configMaximized"
                  class="min-h-0 flex-1"
                  :probe="probe"
                  :fill-trigger="quickFillTrigger"
                  @close="closeConfigPreview"
                  @toggle-maximize="toggleConfigMaximize"
                />
              </div>
            </div>
          </div>

          <!-- ---- 主体：配置管理 ---- -->
          <ConfigManagerView v-show="activeTab === 'config'" />

          <!-- ---- 主体：Key 管家（Beta） ---- -->
          <!-- v-show 切换以保留列表、过滤条件与抽屉展开状态；
            Key 管家自带底部状态汇总条，全局状态栏仅属于探测页。 -->
          <KeyButlerView
              v-show="activeTab === 'keyButler'"
              @launch-probe="launchProbeFromKey"
              @open-fill="openFillFromKeyButler"
            />

          <!-- ---- 状态栏（仅探测页） ---- -->
          <footer
            v-show="activeTab === 'probe'"
            class="flex shrink-0 items-center justify-between border-t border-ms-border bg-ms-panel px-4 py-1.5 text-[11px] text-ms-text-faint"
          >
            <div class="flex items-center gap-3">
              <span>
                状态：
                <span class="text-ms-text-dim">{{ statusLabel }}</span>
              </span>
              <span v-if="probe.store.total > 0">
                进度：{{ probe.store.completed }}/{{ probe.store.total }}
              </span>
            </div>
            <div class="flex items-center gap-3">
              <span v-if="probe.store.models.length > 0">
                已选 {{ probe.store.selectedIds.length }} / {{ probe.store.models.length }}
              </span>
              <span v-if="canGenerate" class="text-ms-accent">● 可生成配置</span>
              <span v-else>○ 等待探测结果</span>
            </div>
          </footer>
        </div>

        <!-- ---- 配置预览：全屏态 ---- -->
        <!--
          全屏态经 Teleport 挂到 body：脱离原布局的 flex 嵌套，
          避免与 ModelTable 争抢高度及处理各层 overflow；
          z-50 低于 Naive UI 的 message/modal 层级，
          复制提示与导出确认框仍可覆盖在全屏预览之上。
        -->
        <Teleport to="body">
          <div
            v-if="configMaximized"
            class="fixed inset-0 z-50 flex flex-col p-3"
            style="background-color: color-mix(in srgb, var(--color-ms-bg) 92%, transparent)"
          >
            <ConfigPreview
              class="min-h-0 flex-1"
              :probe="probe"
              maximized
              :fill-trigger="quickFillTrigger"
              @close="closeConfigPreview"
              @toggle-maximize="toggleConfigMaximize"
            />
            <div class="shrink-0 pt-2 text-center text-[11px] text-ms-text-faint">
              按 Esc 或点「⤡ 还原」回到内嵌预览
            </div>
          </div>
        </Teleport>

        <!-- ---- 关于 / 设置弹窗 ---- -->
        <AboutDialog v-model:show="showAbout" />
        <SettingsDialog v-model:show="showSettings" />
      </NDialogProvider>
    </NMessageProvider>
  </NConfigProvider>
</template>
