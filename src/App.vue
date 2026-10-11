<script setup lang="ts">
/**
 * ============================================================================
 * ModelScout —— 根组件
 * ============================================================================
 *
 * 【职责】
 *   1. 提供 Naive UI 的暗色主题与全局主题覆盖（themeOverrides）
 *   2. 提供消息/对话框 Provider（ElMessage 依赖它）
 *   3. 装配整体布局骨架：顶栏 + 输入区 + 控制条 + 【进度 | 结果】+ 配置预览
 *   4. 在挂载时建立事件订阅，卸载时释放
 *
 * 【布局策略】
 * 整页固定高度，内部各区域独立滚动 —— 这是桌面工具类应用的常见做法，
 * 保证长时间探测时界面不抖动、日志区不把其他区域顶出屏幕。
 * ============================================================================
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
  type GlobalThemeOverrides,
} from 'naive-ui'

import AppHeader from '@/components/AppHeader.vue'
import AboutDialog from '@/components/AboutDialog.vue'
import ConnectionForm from '@/components/ConnectionForm.vue'
import ProbeControl from '@/components/ProbeControl.vue'
import ProgressPanel from '@/components/ProgressPanel.vue'
import ModelTable from '@/components/ModelTable.vue'
import ConfigPreview from '@/components/ConfigPreview.vue'
import KeyButlerView from '@/views/keyButler/KeyButlerView.vue'
import { useProbe } from '@/composables/useProbe'

/* ---------------------------------------------------------------------------
 * 主题覆盖
 * ---------------------------------------------------------------------------
 * 把 style.css 中的设计令牌映射到 Naive UI 的 themeOverrides，
 * 使第三方组件与自定义元素视觉完全统一（避免「拼贴感」）。
 */
const themeOverrides: GlobalThemeOverrides = {
  common: {
    // 品牌色：青绿
    primaryColor: '#14b8a6',
    primaryColorHover: '#2dd4bf',
    primaryColorPressed: '#0d9488',
    primaryColorSuppl: '#14b8a6',

    // 语义色
    successColor: '#22c55e',
    warningColor: '#f59e0b',
    errorColor: '#ef4444',
    infoColor: '#3b82f6',

    // 背景层级
    bodyColor: '#0a0e14',
    cardColor: '#111721',
    modalColor: '#161d29',
    popoverColor: '#161d29',
    tableColor: 'transparent',

    // 边框
    borderColor: '#232c3d',
    dividerColor: '#232c3d',

    // 文字
    textColorBase: '#d6deeb',
    textColor1: '#d6deeb',
    textColor2: '#b4c0d3',
    textColor3: '#8b98ad',
    placeholderColor: '#5c6b82',

    // 圆角与字号
    borderRadius: '6px',
    borderRadiusSmall: '4px',
    fontSize: '13px',
    fontFamily:
      "Inter, -apple-system, BlinkMacSystemFont, 'Segoe UI', 'PingFang SC', 'Microsoft YaHei', sans-serif",
    fontFamilyMono:
      "'JetBrains Mono', 'Cascadia Code', 'Fira Code', Consolas, 'Courier New', monospace",
  },
  Card: {
    color: '#111721',
    borderColor: '#232c3d',
    paddingMedium: '14px 16px',
  },
  DataTable: {
    thColor: '#161d29',
    thTextColor: '#8b98ad',
    tdColor: 'transparent',
    tdColorHover: '#161d29',
    borderColor: '#232c3d',
    thFontWeight: '600',
  },
  Progress: {
    railColor: '#1c2433',
    fillColor: '#14b8a6',
    textColorLineInner: '#0a0e14',
  },
  Tag: {
    borderRadius: '4px',
  },
  Input: {
    color: '#0d131d',
    colorFocus: '#0d131d',
    border: '1px solid #2e3a4f',
    borderFocus: '1px solid #14b8a6',
    borderHover: '1px solid #3d4b63',
  },
  Tabs: {
    tabColor: 'transparent',
    tabTextColor: '#8b98ad',
    tabTextColorActiveLine: '#14b8a6',
    tabTextColorHoverLine: '#2dd4bf',
    barColor: '#14b8a6',
  },
  Log: {
    color: '#0d131d',
    borderColor: '#232c3d',
  },
}

/* ---------------------------------------------------------------------------
 * 页面布局状态
 * ------------------------------------------------------------------------- */

/** 底部配置预览区是否展开 */
const showConfig = ref(false)

/** 「关于」弹窗是否显示（由 AppHeader 的入口触发） */
const showAbout = ref(false)

/* ---------------------------------------------------------------------------
 * 顶部 Tab：探测 / Key 管家（Beta）
 * ---------------------------------------------------------------------------
 * 用 `v-show` 而不是 `v-if` 切换：探测页内部有大量组件（进度面板、结果表、
 * 配置预览），`v-if` 会在每次切 Tab 时把它们整个卸载重建，
 * 既丢滚动位置也丢展开态。`v-show` 只是切换 display。
 */
const activeTab = ref<'probe' | 'keyButler'>('probe')

/** Tab 定义（beta 为 true 时在文案后挂一个 Beta 徽标） */
const TABS = [
  { value: 'probe' as const, label: '探测', beta: false },
  { value: 'keyButler' as const, label: 'Key 管家', beta: true },
]

/**
 * 配置预览是否放大到全屏。
 *
 * 与 `showConfig` 分开是两个独立的维度：
 *   showConfig=false            → 完全不显示
 *   showConfig=true, max=false  → 内嵌在结果表下方
 *   showConfig=true, max=true   → 覆盖整个窗口
 */
const configMaximized = ref(false)

/** 组合式函数：内部已注册 onUnmounted 清理，这里只需触发订阅建立 */
const probe = useProbe()

onMounted(async () => {
  await probe.setupSubscriptions()
  // 挂载时做一次状态重同步：处理「前端热重载但后端仍在探测」的情况
  await probe.sync()
})

/**
 * Esc 退出全屏。
 *
 * 只处理全屏态 —— 内嵌态下按 Esc 不执行任何操作，
 * 否则用户查看日志时误按会直接关闭预览。
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

/** 彻底关闭预览（全屏态下点「关闭」也走这里） */
function closeConfigPreview(): void {
  configMaximized.value = false
  showConfig.value = false
}

/**
 * 状态机状态 → 中文标签。
 *
 * 与 Rust 的 `ProbeStatus` 枚举一一对应。
 * 之所以用对象字面量而不是 switch，是因为它天然覆盖了「未知状态」的回退分支。
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
    :theme="darkTheme"
    :theme-overrides="themeOverrides"
    :locale="zhCN"
    :date-locale="dateZhCN"
  >
    <NMessageProvider :max="4" placement="top-right">
      <NDialogProvider>
        <div class="flex h-screen w-screen flex-col bg-ms-bg text-ms-text">
          <!-- ============ 顶栏 ============ -->
          <AppHeader
            class="shrink-0"
            :status="probe.store.status"
            @open-about="showAbout = true"
          />

          <!-- ============ 页面 Tab ============ -->
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

          <!-- ============ 主体：探测页 ============ -->
          <!--
            用 v-show 而不是 v-if：见脚本里 activeTab 的注释。
            注意这里保留原有的三段式布局结构，只是把最外层从「唯一主体」
            变成「探测 Tab 的内容」。
          -->
          <div
            v-show="activeTab === 'probe'"
            class="flex min-h-0 flex-1 flex-col gap-3 px-4 py-3"
          >
            <!-- ------ 第一段：输入区 ------ -->
            <ConnectionForm class="shrink-0" :probe="probe" />

            <!-- ------ 第二段：控制条（含错误分类汇总） ------ -->
            <ProbeControl class="shrink-0" :probe="probe" />

            <!-- ------ 第三段：进度 / 结果 双栏 ------ -->
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
                  @close="closeConfigPreview"
                  @toggle-maximize="toggleConfigMaximize"
                />
              </div>
            </div>
          </div>

          <!-- ============ 主体：Key 管家（Beta） ============ -->
          <!--
            同样是 v-show：Key 管家的列表、过滤条件、抽屉展开态都值得在
            来回切 Tab 时保留。它自带底部状态汇总条，因此下面那个全局
            状态栏只属于探测页。
          -->
          <KeyButlerView v-show="activeTab === 'keyButler'" />

          <!-- ============ 状态栏（仅探测页） ============ -->
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

        <!-- ============ 配置预览：全屏态 ============ -->
        <!--
          用 Teleport 送到 body，而不是留在原布局里：
          原布局是 flex 嵌套，留在里面就要跟 ModelTable 抢高度，
          还得处理各层 overflow；直接脱离布局用 fixed 覆盖最干净。
          z-50 低于 Naive UI 的 message/modal 层级，所以复制成功的
          提示、导出确认框依然能盖在全屏预览之上。
        -->
        <Teleport to="body">
          <div
            v-if="configMaximized"
            class="fixed inset-0 z-50 flex flex-col p-3"
            style="background-color: rgba(10, 14, 20, 0.92)"
          >
            <ConfigPreview
              class="min-h-0 flex-1"
              :probe="probe"
              maximized
              @close="closeConfigPreview"
              @toggle-maximize="toggleConfigMaximize"
            />
            <div class="shrink-0 pt-2 text-center text-[11px] text-ms-text-faint">
              按 Esc 或点「⤡ 还原」回到内嵌预览
            </div>
          </div>
        </Teleport>

        <!-- ============ 关于弹窗 ============ -->
        <AboutDialog v-model:show="showAbout" />
      </NDialogProvider>
    </NMessageProvider>
  </NConfigProvider>
</template>
