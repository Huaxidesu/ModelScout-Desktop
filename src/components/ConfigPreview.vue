<script setup lang="ts">
/**
 * ============================================================================
 * ConfigPreview —— 配置预览与一键复制
 * ============================================================================
 *
 * 【职责】
 *   1. 双 Tab 切换：OpenCode / DeepSeekHardness
 *   2. 每个 Tab 内可切换输出形态（JSONC / 严格 JSON / TOML）
 *   3. 只读代码预览 + 行号 + 一键复制
 *   4. 导出为文件（写入应用数据目录的 exports/ 子目录）
 *
 * 【剪贴板策略（决策日志 D-05）】
 * Tauri 任务书第七节第 5 条要求：不确定 navigator.clipboard 在 WebView
 * 中是否可用时，改用 tauri-plugin-clipboard-manager。
 *
 * 本项目采用**三级降级**策略，最大化兼容性：
 *   1. 优先尝试 tauri-plugin-clipboard-manager（原生，最可靠）
 *   2. 失败则回退 navigator.clipboard（需 HTTPS 或 localhost 上下文）
 *   3. 再失败则回退 document.execCommand('copy')（最古老但最广）
 * 三级都失败时，提示用户手动全选复制，并自动帮用户选中代码区。
 * ============================================================================
 */

import { computed, ref, watch } from 'vue'
import {
  NButton,
  NTabs,
  NTabPane,
  NSelect,
  NTag,
  NAlert,
  NInput,
  NTooltip,
  useDialog,
  useMessage,
} from 'naive-ui'
import { writeText } from '@tauri-apps/plugin-clipboard-manager'

import { useProbe } from '@/composables/useProbe'
import { saveConfigFile } from '@/api/commands'
import {
  renderOpenCodeJsonc,
  renderOpenCodeJson,
  renderOpenCodeProviderFragment,
  inferProviderPackage,
} from '@/generators/opencode'
import {
  renderHardnessJsonc,
  renderHardnessJson,
  renderHardnessToml,
  FORMAT_ASSUMPTIONS,
} from '@/generators/deepseekHardness'
import {
  type ApiKeyMode,
  hasApiKey,
  inferEnvVarName,
  inferProviderKey,
  inferProviderName,
  sanitizeProviderKey,
} from '@/generators/naming'

import type { ModelInfo } from '@/types/contracts'

const props = defineProps<{
  probe: ReturnType<typeof useProbe>
  /** 是否处于全屏放大态（由 App.vue 控制） */
  maximized?: boolean
}>()

const emit = defineEmits<{
  (e: 'close'): void
  (e: 'toggle-maximize'): void
}>()

const message = useMessage()
const dialog = useDialog()

/** 当前激活的 Tab */
const activeTab = ref<'opencode' | 'hardness'>('opencode')

/** 各 Tab 的输出形态 */
const opencodeFormat = ref<'jsonc' | 'json' | 'fragment'>('jsonc')
const hardnessFormat = ref<'jsonc' | 'json' | 'toml'>('jsonc')

/** 复制按钮的瞬时状态（用于给出视觉反馈） */
const copied = ref<'opencode' | 'hardness' | null>(null)

/** 代码区 DOM 引用（降级复制时用于全选） */
const opencodeRef = ref<HTMLElement | null>(null)
const hardnessRef = ref<HTMLElement | null>(null)

/* ---------------------------------------------------------------------------
 * 命名与密钥（生成时的可选项）
 * ---------------------------------------------------------------------------
 *
 * 【设计：推断值当默认，用户输入当覆盖】
 * `xxxOverride === null` 表示「跟随推断」，一旦用户动过输入框就固定下来。
 * 这样 Base URL 一改，键名会自动跟着变（用户没手动改过的话），
 * 而用户改过之后自动推断不会再覆盖其输入。
 */

/** provider 键名的人工覆盖值；null = 跟随 Base URL 推断 */
const providerKeyOverride = ref<string | null>(null)
/** provider 显示名的人工覆盖值；null = 跟随键名推断 */
const providerNameOverride = ref<string | null>(null)
/** 环境变量名的人工覆盖值；null = 跟随键名推断 */
const envVarOverride = ref<string | null>(null)

/** 密钥写入方式 */
const apiKeyMode = ref<ApiKeyMode>('plain')

const baseUrl = computed(() => props.probe.store.config.baseUrl)
const apiKey = computed(() => props.probe.store.config.apiKey ?? '')

/** provider 键名输入框的原始值（**不做实时规整**，否则用户打不出 `my-key`） */
const providerKeyInput = computed<string>({
  get: () => providerKeyOverride.value ?? inferProviderKey(baseUrl.value),
  set: (value: string) => {
    providerKeyOverride.value = value
  },
})

/**
 * 真正写进配置的键名 —— 已规整成合法形态。
 *
 * 规整放在**这里**而不是输入框的 setter：用户打字过程中
 * `my-` 这种中间态必须被原样保留，否则连字符会被规整掉，无法输入
 * `my-key`。规整只在生成时发生，用户看到的是自己输入的内容。
 */
const providerKey = computed<string>(
  () => sanitizeProviderKey(providerKeyInput.value) || inferProviderKey(baseUrl.value),
)

/** provider 显示名（输入框双向绑定） */
const providerName = computed<string>({
  get: () => (providerNameOverride.value ?? '').trim() || inferProviderName(providerKey.value),
  set: (value: string) => {
    providerNameOverride.value = value
  },
})

/** 环境变量名（仅 env 模式使用） */
const envVarName = computed<string>({
  get: () => (envVarOverride.value ?? '').trim() || inferEnvVarName(providerKey.value),
  set: (value: string) => {
    envVarOverride.value = value
  },
})

/**
 * 键名变化时，让「还是自动推断状态」的从属字段跟着更新。
 *
 * 判断依据：某个从属字段的覆盖值恰好等于「用旧键名推断出来的值」，
 * 说明它本来就是自动填的，不是用户特意写的 → 重新推断。
 */
watch(providerKey, (next, prev) => {
  if (next === prev) return
  if (providerNameOverride.value === null || providerNameOverride.value === inferProviderName(prev)) {
    providerNameOverride.value = null
  }
  if (envVarOverride.value === null || envVarOverride.value === inferEnvVarName(prev)) {
    envVarOverride.value = null
  }
})

/** 是否已填写 API Key（决定能不能生成「复制即可用」的配置） */
const keyFilled = computed(() => hasApiKey(apiKey.value))

/** 密钥的脱敏展示，避免在界面上二次暴露完整 Key */
const maskedKey = computed(() => {
  const key = apiKey.value.trim()
  if (!key) return ''
  if (key.length <= 10) return `${key.slice(0, 3)}***`
  return `${key.slice(0, 5)}…${key.slice(-4)}`
})

/* ---------------------------------------------------------------------------
 * 生成上下文
 * ------------------------------------------------------------------------- */

/** 生成器共用的上下文 */
const context = computed(() => ({
  baseUrl: baseUrl.value,
  models: props.probe.store.selectedModels,
  providerKey: providerKey.value,
  providerName: providerName.value,
  apiKey: apiKey.value,
  apiKeyMode: apiKeyMode.value,
  apiKeyEnv: envVarName.value,
}))

/* ---------------------------------------------------------------------------
 * 生成结果
 * ------------------------------------------------------------------------- */

/** OpenCode 配置文本 */
const opencodeText = computed(() => {
  if (context.value.models.length === 0) {
    return '// 尚未选择任何模型。请在上方结果表格中勾选要导出的模型。'
  }
  switch (opencodeFormat.value) {
    case 'json':
      return renderOpenCodeJson(context.value)
    case 'fragment':
      return renderOpenCodeProviderFragment(context.value)
    default:
      return renderOpenCodeJsonc(context.value)
  }
})

/** OpenCode 当前形态对应的建议文件名 */
const opencodeFileName = computed(() => {
  switch (opencodeFormat.value) {
    case 'json':
      return 'opencode.json'
    case 'fragment':
      return 'opencode-provider.jsonc'
    default:
      return 'opencode.jsonc'
  }
})

/** DeepSeekHardness 配置文本 */
const hardnessText = computed(() => {
  if (context.value.models.length === 0) {
    return '// 尚未选择任何模型。请在上方结果表格中勾选要导出的模型。'
  }
  switch (hardnessFormat.value) {
    case 'json':
      return renderHardnessJson(context.value)
    case 'toml':
      return renderHardnessToml(context.value)
    default:
      return renderHardnessJsonc(context.value)
  }
})

/** 当前激活 Tab 对应的文本 */
const activeText = computed(() =>
  activeTab.value === 'opencode' ? opencodeText.value : hardnessText.value,
)

/** 当前激活 Tab 对应的建议文件名 */
const activeFileName = computed(() => {
  if (activeTab.value === 'opencode') {
    return opencodeFileName.value
  }
  switch (hardnessFormat.value) {
    case 'json':
      return 'deepseek-hardness.json'
    case 'toml':
      return 'deepseek-hardness.toml'
    default:
      return FORMAT_ASSUMPTIONS.defaultFileName
  }
})

/** 当前激活 Tab 对应的代码区引用 */
const activeRef = computed(() =>
  activeTab.value === 'opencode' ? opencodeRef.value : hardnessRef.value,
)

/* ---------------------------------------------------------------------------
 * 统计信息
 * ------------------------------------------------------------------------- */

/** 每个生成器的统计（模型数、字符数、行数） */
const stats = computed(() => {
  const count = context.value.models.length
  return {
    models: count,
    opencodeChars: opencodeText.value.length,
    opencodeLines: opencodeText.value.split('\n').length,
    hardnessChars: hardnessText.value.length,
    hardnessLines: hardnessText.value.split('\n').length,
  }
})

/** 被选中模型里有多少上限是未知的（需要提醒用户） */
const unknownLimitCount = computed(
  () =>
    context.value.models.filter(
      (m: ModelInfo) => m.contextLimit === 0 || m.maxOutputLimit === 0,
    ).length,
)

/* ---------------------------------------------------------------------------
 * 复制（三级降级）
 * ------------------------------------------------------------------------- */

/**
 * 执行复制，返回是否成功。
 *
 * 三级降级顺序：原生插件 → navigator.clipboard → execCommand。
 */
async function performCopy(text: string): Promise<boolean> {
  // ---- 第 1 级：Tauri 官方剪贴板插件 ----
  try {
    await writeText(text)
    return true
  } catch (err) {
    console.warn('[ModelScout] 剪贴板插件写入失败，尝试降级方案：', err)
  }

  // ---- 第 2 级：Web Clipboard API ----
  try {
    if (navigator.clipboard && window.isSecureContext) {
      await navigator.clipboard.writeText(text)
      return true
    }
  } catch (err) {
    console.warn('[ModelScout] navigator.clipboard 失败，尝试降级方案：', err)
  }

  // ---- 第 3 级：execCommand（已废弃但兼容性最广）----
  try {
    const textarea = document.createElement('textarea')
    textarea.value = text
    // 移出视口，避免滚动跳动
    textarea.style.position = 'fixed'
    textarea.style.top = '-9999px'
    textarea.style.left = '-9999px'
    textarea.setAttribute('readonly', '')
    document.body.appendChild(textarea)
    textarea.select()
    const ok = document.execCommand('copy')
    document.body.removeChild(textarea)
    return ok
  } catch (err) {
    console.error('[ModelScout] 所有剪贴板方案均失败：', err)
    return false
  }
}

/** 复制当前 Tab 的配置 */
async function copyActive(): Promise<void> {
  const text = activeText.value
  if (!text.trim()) {
    message.warning('没有可复制的内容。')
    return
  }

  const ok = await performCopy(text)
  if (ok) {
    copied.value = activeTab.value
    message.success(`已复制 ${activeFileName.value}（${text.length} 字符）`)
    // 1.5 秒后恢复按钮文案
    setTimeout(() => {
      copied.value = null
    }, 1500)
  } else {
    // 全部失败：选中代码区，供用户手动 Ctrl+C
    selectActiveCode()
    message.error('剪贴板不可用，已为你选中代码内容，请按 Ctrl+C 手动复制。')
  }
}

/** 选中当前 Tab 的代码区内容 */
function selectActiveCode(): void {
  const el = activeRef.value
  if (!el) return
  const selection = window.getSelection()
  if (!selection) return
  const range = document.createRange()
  range.selectNodeContents(el)
  selection.removeAllRanges()
  selection.addRange(range)
}

/* ---------------------------------------------------------------------------
 * 导出为文件
 * ------------------------------------------------------------------------- */

const exporting = ref(false)

/** 把当前配置写入磁盘 */
async function exportActive(): Promise<void> {
  // 明文密钥落盘前必须让用户知情 —— 复制到剪贴板是「用过就散」，
  // 而写成文件会长期留在磁盘上，风险等级不同。
  if (apiKeyMode.value === 'plain' && keyFilled.value) {
    const confirmed = await new Promise<boolean>((resolve) => {
      dialog.warning({
        title: '导出文件包含明文 API Key',
        content: `即将导出的 ${activeFileName.value} 里含有你的 API Key（${maskedKey.value}）。文件会保存在磁盘上，请注意不要把它分享给他人或提交到公开仓库。`,
        positiveText: '仍然导出',
        negativeText: '改用环境变量方式',
        onPositiveClick: () => resolve(true),
        onNegativeClick: () => {
          apiKeyMode.value = 'env'
          resolve(false)
        },
        onClose: () => resolve(false),
        onMaskClick: () => resolve(false),
      })
    })
    if (!confirmed) return
  }

  exporting.value = true
  try {
    const result = await saveConfigFile({
      fileName: activeFileName.value,
      content: activeText.value,
    })
    message.success(`已导出到：${result.path}`, { duration: 8000 })
  } catch (err) {
    const text = err instanceof Error ? err.message : String(err)
    message.error(`导出失败：${text}`)
  } finally {
    exporting.value = false
  }
}

/* ---------------------------------------------------------------------------
 * 行号渲染
 * ------------------------------------------------------------------------- */

/**
 * 把文本拆成带行号的行数组。
 *
 * 【为什么前端算行号而不是用第三方编辑器组件】
 * 此处的需求是「只读预览 + 复制」，引入 CodeMirror/Monaco 会显著
 * 增大打包体积（Monaco 约 2MB+）。手写行号 + `<pre>` 只需几十行，
 * 且渲染性能对几百行的配置文件完全够用。
 */
function numberedLines(text: string): Array<{ no: number; content: string }> {
  return text.split('\n').map((content, index) => ({
    no: index + 1,
    content,
  }))
}

const opencodeLines = computed(() => numberedLines(opencodeText.value))
const hardnessLines = computed(() => numberedLines(hardnessText.value))

/** 语法高亮：给简单的 token 着色，提升可读性 */
function highlight(line: string): string {
  // 先做 HTML 转义，防止内容里的 < > 破坏结构
  let escaped = line
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')

  // 注释（// 或 #）整体着色，且不再处理内部 token
  const commentMatch = escaped.match(/^(\s*)(\/\/|#)(.*)$/)
  if (commentMatch) {
    return `${commentMatch[1]}<span class="tok-comment">${commentMatch[2]}${commentMatch[3]}</span>`
  }

  // 字符串（双引号）
  escaped = escaped.replace(
    /(&quot;|")([^"]*)(&quot;|")/g,
    '<span class="tok-string">"$2"</span>',
  )
  // 键名：以 " 开头且后面紧跟冒号的
  escaped = escaped.replace(
    /<span class="tok-string">"([^"]+)"<\/span>(\s*:)/g,
    '<span class="tok-key">"$1"</span>$2',
  )
  // 数字
  escaped = escaped.replace(
    /(\s|\[|,|:)(\s*)(\d+(?:\.\d+)?)(\s*)(,|\]|\}|$)/g,
    '$1$2<span class="tok-number">$3</span>$4$5',
  )
  // 布尔与 null
  escaped = escaped.replace(
    /\b(true|false|null)\b/g,
    '<span class="tok-bool">$1</span>',
  )

  return escaped
}

/* ---------------------------------------------------------------------------
 * 预览内容变化提示
 * ------------------------------------------------------------------------- */

/**
 * 当选中的模型集合变化时，如果预览区已打开，给一个轻提示。
 *
 * 这解决一个实际困扰：用户在表格里改了勾选，
 * 但没意识到预览区内容**已经自动跟着变了**（属预期行为，但需提示用户）。
 */
watch(
  () => props.probe.store.selectedIds.length,
  (next, prev) => {
    if (prev !== undefined && next !== prev) {
      message.info(`已同步更新配置内容（当前 ${next} 个模型）`, { duration: 2000 })
    }
  },
)
</script>

<template>
  <section class="ms-panel flex min-h-0 flex-col overflow-hidden">
    <!-- ---------- 头部 ---------- -->
    <div class="flex shrink-0 items-center justify-between border-b border-ms-border px-3 py-2">
      <div class="flex items-center gap-2">
        <span class="ms-panel-title">配置预览</span>
        <NTag size="tiny" :bordered="false" type="info">
          {{ stats.models }} 个模型
        </NTag>
        <NTag v-if="unknownLimitCount > 0" size="tiny" :bordered="false" type="warning">
          {{ unknownLimitCount }} 个模型上限未知
        </NTag>
      </div>

      <div class="flex items-center gap-2">
        <!-- 形态切换 -->
        <NSelect
          v-if="activeTab === 'opencode'"
          v-model:value="opencodeFormat"
          :options="[
            { label: 'JSONC（完整配置）', value: 'jsonc' },
            { label: 'JSON（完整·严格）', value: 'json' },
            { label: '片段（粘贴进已有配置）', value: 'fragment' },
          ]"
          size="tiny"
          style="width: 178px"
        />
        <NSelect
          v-else
          v-model:value="hardnessFormat"
          :options="[
            { label: 'JSONC（带注释）', value: 'jsonc' },
            { label: 'JSON（严格）', value: 'json' },
            { label: 'TOML', value: 'toml' },
          ]"
          size="tiny"
          style="width: 150px"
        />

        <NButton size="tiny" :loading="exporting" @click="exportActive()">
          导出文件
        </NButton>
        <NButton
          size="tiny"
          type="primary"
          :disabled="context.models.length === 0"
          @click="copyActive()"
        >
          {{ copied === activeTab ? '✓ 已复制' : '一键复制' }}
        </NButton>

        <NTooltip trigger="hover">
          <template #trigger>
            <NButton size="tiny" quaternary @click="emit('toggle-maximize')">
              {{ maximized ? '⤡ 还原' : '⛶ 放大' }}
            </NButton>
          </template>
          {{ maximized ? '还原为内嵌预览（Esc）' : '放大到整个窗口，便于通读与编辑' }}
        </NTooltip>

        <NButton size="tiny" quaternary @click="emit('close')">
          {{ maximized ? '关闭' : '收起' }}
        </NButton>
      </div>
    </div>

    <!-- ---------- provider 命名与密钥 ---------- -->
    <div
      class="flex shrink-0 flex-wrap items-center gap-x-4 gap-y-2 border-b border-ms-border px-3 py-2"
    >
      <div class="flex items-center gap-1.5">
        <span class="text-[11px] text-ms-text-faint">provider 键名</span>
        <NInput
          v-model:value="providerKeyInput"
          size="tiny"
          placeholder="deepseek-local"
          style="width: 148px"
        />
      </div>

      <div class="flex items-center gap-1.5">
        <span class="text-[11px] text-ms-text-faint">显示名</span>
        <NInput
          v-model:value="providerName"
          size="tiny"
          placeholder="DeepSeek Local"
          style="width: 158px"
        />
      </div>

      <div class="flex items-center gap-1.5">
        <span class="text-[11px] text-ms-text-faint">密钥写入</span>
        <NSelect
          v-model:value="apiKeyMode"
          size="tiny"
          style="width: 132px"
          :options="[
            { label: '明文（可直接用）', value: 'plain' },
            { label: '环境变量引用', value: 'env' },
          ]"
        />
        <NInput
          v-if="apiKeyMode === 'env'"
          v-model:value="envVarName"
          size="tiny"
          placeholder="DEEPSEEK_LOCAL_API_KEY"
          style="width: 210px"
        />
        <NTooltip v-else trigger="hover">
          <template #trigger>
            <NTag size="tiny" :bordered="false" :type="keyFilled ? 'success' : 'warning'">
              {{ keyFilled ? maskedKey : '连接表单里未填 Key' }}
            </NTag>
          </template>
          密钥取自上方连接表单的 API Key 输入框，此处只控制它以什么形式写进配置。
        </NTooltip>
      </div>
    </div>

    <!-- ---------- 提醒条 ---------- -->
    <NAlert
      v-if="context.models.length > 0"
      type="warning"
      :bordered="false"
      class="shrink-0"
      style="border-radius: 0"
    >
      <template #header>
        <span class="text-[11px]">配置格式需人工核对</span>
      </template>
      <div class="text-[11px] leading-relaxed">
        <span v-if="activeTab === 'opencode'">
          字段已按 OpenCode 官方 schema（
          <code class="text-ms-accent">opencode.ai/config.json</code>）核对；provider 通过
          <code class="text-ms-accent">{{ inferProviderPackage(context.baseUrl) }}</code>
          加载。
          <span v-if="apiKeyMode === 'plain' && keyFilled" class="text-ms-warning">
            ⚠️ 当前含明文 API Key，请勿分享或提交到公开仓库。
          </span>
        </span>
        <span v-else>
          本模板基于公开信息整理。
          <span v-if="apiKeyMode === 'plain' && keyFilled" class="text-ms-warning">
            当前含明文 API Key；</span
          >
          若配置要入库，建议把「密钥写入」改为环境变量方式（
          <code class="text-ms-accent">{{ envVarName }}</code>）。
        </span>
      </div>
    </NAlert>

    <!-- ---------- Tab + 代码区 ---------- -->
    <NTabs
      v-model:value="activeTab"
      type="line"
      size="small"
      class="min-h-0 flex-1"
      pane-class="min-h-0"
      style="--n-tab-padding: 0 12px"
    >
      <!-- ========== OpenCode ========== -->
      <NTabPane name="opencode" tab="OpenCode" class="min-h-0">
        <div class="flex h-full flex-col">
          <div class="shrink-0 border-b border-ms-border px-3 py-1 text-[10px] text-ms-text-faint">
            {{ stats.opencodeLines }} 行 · {{ stats.opencodeChars }} 字符 ·
            建议文件名 {{ opencodeFileName }}
          </div>
          <div ref="opencodeRef" class="ms-code-block ms-code min-h-0 flex-1 overflow-auto px-3 py-2">
            <div
              v-for="line in opencodeLines"
              :key="line.no"
              class="flex hover:bg-white/[0.02]"
            >
              <span class="mr-3 w-8 shrink-0 select-none text-right text-ms-text-faint opacity-50">
                {{ line.no }}
              </span>
              <span class="whitespace-pre" v-html="highlight(line.content)" />
            </div>
          </div>
        </div>
      </NTabPane>

      <!-- ========== DeepSeekHardness ========== -->
      <NTabPane name="hardness" tab="DeepSeekHardness" class="min-h-0">
        <div class="flex h-full flex-col">
          <div class="shrink-0 border-b border-ms-border px-3 py-1 text-[10px] text-ms-text-faint">
            {{ stats.hardnessLines }} 行 · {{ stats.hardnessChars }} 字符 ·
            建议文件名 {{ activeTab === 'hardness' ? activeFileName : FORMAT_ASSUMPTIONS.defaultFileName }}
          </div>
          <div ref="hardnessRef" class="ms-code-block ms-code min-h-0 flex-1 overflow-auto px-3 py-2">
            <div
              v-for="line in hardnessLines"
              :key="line.no"
              class="flex hover:bg-white/[0.02]"
            >
              <span class="mr-3 w-8 shrink-0 select-none text-right text-ms-text-faint opacity-50">
                {{ line.no }}
              </span>
              <span class="whitespace-pre" v-html="highlight(line.content)" />
            </div>
          </div>
        </div>
      </NTabPane>
    </NTabs>
  </section>
</template>

<style scoped>
/* 简易语法高亮配色：与整体暗色极客风一致 */
.ms-code :deep(.tok-comment) {
  color: #5c6b82;
  font-style: italic;
}

.ms-code :deep(.tok-key) {
  color: #7dd3fc;
}

.ms-code :deep(.tok-string) {
  color: #86efac;
}

.ms-code :deep(.tok-number) {
  color: #fcd34d;
}

.ms-code :deep(.tok-bool) {
  color: #c4b5fd;
}
</style>
