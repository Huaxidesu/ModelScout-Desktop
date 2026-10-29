<script setup lang="ts">
/**
 * 一键填写对话框：读取目标工具的本机配置文件 → 生成合并结果 →
 * 左右对比预览（VSCode 并排 diff 风格）→ 用户确认后写入。
 *
 * 两条主路径（与需求对应）：
 *   - 用户还没有模型配置（文件不存在 / 空文件 / `{}` 骨架）：
 *     右侧直接是完整配置，确认后写入新文件（含自动建目录）；
 *   - 已有配置：把 provider 片段并入现有文件（OpenCode 做保注释的
 *     原文拼接，dsh 做 YAML 对象合并），其余内容保留。
 *
 * 安全设计：
 *   - 写入前永远先展示左右对比，确认按钮才会触盘；
 *   - 覆盖已有文件时后端自动生成 `.modelscout.bak` 备份；
 *   - 目标路径可改（支持 `~`），改完点「重新读取」再预览。
 */

import { computed, ref, watch } from 'vue'
import {
  NAlert,
  NButton,
  NCheckbox,
  NInput,
  NModal,
  NTag,
  NTooltip,
  useMessage,
} from 'naive-ui'

import { inspectToolConfig, statusEnvSecret, setEnvSecret, writeToolConfig } from '@/api/commands'
import { mergeOpenCodeIntoText } from '@/generators/opencodeMerge'
import { mergeDshIntoText } from '@/generators/dsh'
import { detectOpenCodeVersion } from '@/generators/configScan'
import type { GenerateContext } from '@/generators/opencode'
import { diffLines, type DiffRow } from '@/utils/diffLines'
import { highlightLine } from '@/utils/highlight'
import ModelEditTable from '@/components/ModelEditTable.vue'
import {
  applyRowsToProbeModels,
  rowsFromProbeModels,
  v2ExtrasFromRows,
  type ModelEditRow,
} from '@/generators/modelOverrides'
import type { ToolConfigFile } from '@/types/contracts'

const props = defineProps<{
  /** 是否显示（v-model:show） */
  show: boolean
  /** 目标工具：OpenCode 或 DeepSeek Harness */
  tool: 'opencode' | 'dsh'
  /** 生成上下文（与配置预览区共用同一份推断与用户输入） */
  context: GenerateContext
  /**
   * 新建文件时使用的 OpenCode schema 世代（来自配置预览区的 V1/V2 选择）。
   * 已有文件永远跟随文件本身的世代，此选项不生效。
   */
  preferredVersion?: 'v1' | 'v2'
}>()

const emit = defineEmits<{
  (e: 'update:show', value: boolean): void
  /** 写入成功后触发，参数为实际写入路径 */
  (e: 'written', path: string): void
}>()

const message = useMessage()

// ---- 状态 ----

/** 正在读取目标文件 */
const loading = ref(false)
/** 正在写入 */
const writing = ref(false)
/** inspect 的原始结果（null = 尚未读取） */
const file = ref<ToolConfigFile | null>(null)
/** 目标路径（用户可改） */
const targetPath = ref('')
/** 合并结果文本（空串 = 未生成/出错） */
const mergedText = ref('')
/** 合并模式的展示文案 */
const mergeModeLabel = ref('')
/** 需要展示的警告（合并器给出） */
const mergeWarnings = ref<string[]>([])
/** 读取/合并失败的用户可读错误（非空 = 阻止写入） */
const fatal = ref('')

/** 目标文件识别出的 OpenCode schema 世代（null = 文件不存在或没识别到） */
const targetSchema = ref<'v1' | 'v2' | null>(null)

/**
 * 本次合并实际生效的世代：已有文件跟随文件；新建文件跟随配置预览区的
 * V1/V2 选择（preferredVersion，缺省 v1）。v2 时参数调整区会带 v2 独有列。
 */
const effectiveSchema = computed<'v1' | 'v2'>(() => {
  if (props.tool !== 'opencode') return 'v1'
  if (file.value?.exists && targetSchema.value) return targetSchema.value
  return props.preferredVersion ?? 'v1'
})

// ---- 派生 ----

const toolLabel = computed(() => (props.tool === 'opencode' ? 'OpenCode' : 'DeepSeek Harness (dsh)'))

/** 左侧展示的原文（文件不存在时为空，diff 全绿） */
const leftText = computed(() => (file.value?.exists ? file.value.content : ''))

/** 左右对比行 */
const diffRows = computed<DiffRow[]>(() => diffLines(leftText.value, mergedText.value))

/** 统计 */
const stats = computed(() => {
  const rows = diffRows.value
  return {
    added: rows.filter((r) => r.type === 'add').length,
    removed: rows.filter((r) => r.type === 'del').length,
    total: rows.length,
  }
})

/** 是否已填写明文密钥（用于警告文案） */
const plainKeyFilled = computed(
  () => props.context.apiKeyMode !== 'env' && Boolean(props.context.apiKey?.trim()),
)

/* ---- 模型参数调整（写入前的最后一道人工修正） ---- */

/** 参数调整区是否展开 */
const overridesOpen = ref(false)
/** 当前编辑行（context 变化时重建；未展开过不初始化） */
const overrideRows = ref<ModelEditRow[]>([])

/** 打开参数调整区时按当前 context 建行 */
function openOverrides(): void {
  overrideRows.value = rowsFromProbeModels(props.context.models)
  // v2 目标：给推理模型预填常规强度档位（可增删；清空 = 不写该字段）
  if (props.tool === 'opencode' && effectiveSchema.value === 'v2') {
    for (const row of overrideRows.value) {
      if (row.reasoning === true) row.effort = ['low', 'medium', 'high']
    }
  }
  overridesOpen.value = true
}

/**
 * 生效的编辑行：未展开参数调整时退回探测默认行。
 * 默认行的输出上限是留空的，因此「默认不填 output」在用户没打开参数表时同样成立。
 */
const effectiveRows = computed<ModelEditRow[]>(() =>
  overrideRows.value.length > 0 ? overrideRows.value : rowsFromProbeModels(props.context.models),
)

/** 应用覆盖后的生成上下文（合并/预览全部走它） */
const effectiveContext = computed<GenerateContext>(() => ({
  ...props.context,
  models: applyRowsToProbeModels(props.context.models, effectiveRows.value),
}))
/* ---- 基础信息注释（写在 provider 条目上方的一行说明） ---- */

/** 是否生成基础信息注释（默认开） */
const providerCommentOn = ref(true)

/** 目标文件是否支持注释（JSONC / YAML） */
const commentableTarget = computed(() => /\.(jsonc|ya?ml)$/i.test(targetPath.value.trim()))

/** 目标是否为严格 JSON（注释不可用，提供一键转 JSONC） */
const jsonTarget = computed(() => /\.json$/i.test(targetPath.value.trim()))

/** 基础信息注释内容（显示名 · 模型数 · 日期） */
const providerComment = computed(
  () =>
    `${props.context.providerName || props.context.providerKey || 'Provider'} · ${props.context.models.length} 个模型 · ModelScout ${new Date().toISOString().slice(0, 10)}`,
)

/** 一键把目标路径换成 .jsonc（原 .json 不修改；两个文件并存时 opencode 优先读 .json，需自行取舍） */
function convertTargetToJsonc(): void {
  const next = targetPath.value.trim().replace(/\.json$/i, '.jsonc')
  if (next === targetPath.value.trim()) return
  targetPath.value = next
  message.info('目标已改为同名 .jsonc；原 .json 文件不会被修改。', { duration: 6000 })
  void reload()
}

const keyNotice = computed(() => {
  if (!plainKeyFilled.value) return ''
  if (props.tool === 'dsh') {
    return 'dsh 的补丁文件不支持写明文 API Key，本次写入以 apiKeyEnv 引用环境变量；请设置该变量，或在 dsh 的 Settings → Models 页面保存凭据。'
  }
  return '即将写入的文件包含明文 API Key，请勿分享或提交到公开仓库。'
})

/** 对比区滚动容器的引用（顶部「↓ 到底部确认」把对比滚到最后） */
const diffScrollRef = ref<HTMLElement | null>(null)

/**
 * 把对比内容滚到最底部。
 *
 * 「确认写入」按钮固定在对话框底部、始终可见；这个按钮滚的是对比区
 * 本身 —— 长配置时直接看完最后一行差异，紧接着就是底部的确认栏。
 */
function scrollToConfirm(): void {
  const el = diffScrollRef.value
  if (!el) return
  el.scrollTo({ top: el.scrollHeight, behavior: 'smooth' })
}

/** 路径被改过但还没重新读取（预览对应的是旧路径的内容） */
const pathDirty = computed(
  () => file.value !== null && targetPath.value.trim() !== file.value.path,
)

/** 写入按钮是否可用 */
const canWrite = computed(
  () =>
    !fatal.value &&
    !pathDirty.value &&
    mergedText.value.trim() !== '' &&
    targetPath.value.trim() !== '' &&
    !loading.value,
)

// ---- 读取与合并 ----

/** 调后端读取目标文件并生成合并预览 */
async function reload(): Promise<void> {
  fatal.value = ''
  loading.value = true
  try {
    const custom = targetPath.value.trim() || undefined
    const result = await inspectToolConfig(props.tool, custom)
    file.value = result
    targetPath.value = result.path
    targetSchema.value = result.exists ? detectOpenCodeVersion(result.content) : null
    computeMerge()
  } catch (err) {
    file.value = null
    mergedText.value = ''
    fatal.value = err instanceof Error ? err.message : String(err)
  } finally {
    loading.value = false
  }
}

/** 根据当前文件内容 + 生成上下文计算合并结果 */
function computeMerge(): void {
  mergeWarnings.value = []
  if (!file.value) return
  try {
    const existing = file.value.exists ? file.value.content : ''
    const comment =
      providerCommentOn.value && commentableTarget.value ? providerComment.value : undefined
    if (props.tool === 'opencode') {
      const result = mergeOpenCodeIntoText(existing, effectiveContext.value, {
        providerComment: comment,
        // v2 独有参数（强度档位/预算/温度/输入上限）只在参数区展开时参与
        v2Extras: overridesOpen.value ? v2ExtrasFromRows(overrideRows.value) : undefined,
        // 已有文件跟随文件世代（merge 内部自行判断）；新建文件用所选版本
        createVersion: file.value?.exists ? undefined : (props.preferredVersion ?? 'v1'),
      })
      mergedText.value = result.text
      mergeModeLabel.value =
        result.mode === 'create'
          ? `目标文件没有模型配置，将直接写入完整配置（${effectiveSchema.value === 'v2' ? 'V2 格式' : 'V1 格式'}）`
          : result.mode === 'splice'
            ? `已把 provider 片段并入现有配置（原有内容与注释保留${targetSchema.value === 'v2' ? '，跟随文件的 V2 格式' : ''}）`
            : '现有文件结构特殊，改为整体重写（其他配置项保留）'
      mergeWarnings.value = result.warnings
    } else {
      const result = mergeDshIntoText(existing, effectiveContext.value, {
        providerComment: comment,
      })
      mergedText.value = result.text
      mergeModeLabel.value =
        result.mode === 'create'
          ? '目标文件没有补丁内容，将直接写入完整配置'
          : '已把 provider 并入现有补丁（其他插件条目与字段保留）'
      mergeWarnings.value = result.warnings
    }
    fatal.value = ''
  } catch (err) {
    mergedText.value = ''
    fatal.value = err instanceof Error ? err.message : String(err)
  }
}

// 打开时重置并读取；上下文（模型选择/键名等）变化时重算合并，文件内容不重读
//
// targetPath 特意**保留上次会话的值**：用户在本次会话里已明确选过目标文件，
// 再次打开时静默回落到默认路径，会把内容写到（默认的）真实配置里 —— 曾因此
// 发生测试误写。只有用户清空路径输入框才会回到默认推断。
watch(
  () => props.show,
  (show) => {
    if (show) {
      file.value = null
      mergedText.value = ''
      mergeModeLabel.value = ''
      mergeWarnings.value = []
      fatal.value = ''
      overridesOpen.value = false
      overrideRows.value = []
      void reload()
    }
  },
)

// 上下文或参数调整变化都重算合并；文件内容不重读
watch(
  effectiveContext,
  () => {
    if (props.show) computeMerge()
  },
  { deep: true },
)

// ---- 写入 ----

async function confirmWrite(): Promise<void> {
  if (!canWrite.value) return
  // 写入前兜底：合并结果里引用的环境变量若有本机不存在的，先弹窗
  // 问是否顺手创建（可跳过直接写）——否则写出的配置跑不起来。
  const missing = await findMissingEnvRefs()
  if (missing.length > 0) {
    pendingMissing.value = missing
    missingValues.value = {}
    missingPromptShow.value = true
    return
  }
  await doWrite()
}

/**
 * 收集合并结果里引用、但本机不存在的环境变量名。
 *
 * OpenCode 的引用形态是 `{env:NAME}`；dsh 写的是 `apiKeyEnv: NAME` 键，
 * 从生成上下文里取。查询失败按存在处理（不阻塞写入）。
 */
async function findMissingEnvRefs(): Promise<string[]> {
  // 只扫代码行:注释里的 {env:…}(如文档示例)不是真实引用
  const codeLines = mergedText.value
    .split('\n')
    .filter((line) => !/^\s*\/\//.test(line))
    .join('\n')
  const refs = new Set<string>(
    [...codeLines.matchAll(/\{env:([A-Za-z_][A-Za-z0-9_]*)\}/g)].map((m) => m[1]),
  )
  if (props.tool === 'dsh' && props.context.apiKeyMode === 'env' && props.context.apiKeyEnv?.trim()) {
    refs.add(props.context.apiKeyEnv.trim())
  }
  const missing: string[] = []
  for (const name of refs) {
    try {
      const status = await statusEnvSecret(name)
      if (!status.exists) missing.push(name)
    } catch {
      /* 查询失败视为存在 */
    }
  }
  return missing
}

/** 缺失变量提示对话框状态 */
const missingPromptShow = ref(false)
const pendingMissing = ref<string[]>([])
const missingValues = ref<Record<string, string>>({})

/** 用户选择「仍要写入」：不创建，直接落盘 */
async function writeWithoutCreating(): Promise<void> {
  missingPromptShow.value = false
  await doWrite()
}

/** 用户选择「创建并写入」：先创建填了值的变量，再落盘 */
async function createThenWrite(): Promise<void> {
  const created: string[] = []
  for (const name of pendingMissing.value) {
    const value = (missingValues.value[name] ?? '').trim()
    if (!value) continue
    try {
      await setEnvSecret(name, value)
      created.push(name)
    } catch (err) {
      message.error(`创建 ${name} 失败：${err instanceof Error ? err.message : String(err)}`)
      return
    }
  }
  missingPromptShow.value = false
  if (created.length > 0) {
    const skipped = pendingMissing.value.filter((n) => !(missingValues.value[n] ?? '').trim())
    message.success(
      `已创建 ${created.join('、')}（本应用内立即生效${created.length > 0 ? '' : ''}）` +
        (skipped.length > 0 ? `；${skipped.join('、')} 未创建（留空跳过）` : ''),
      { duration: 7000 },
    )
  }
  await doWrite()
}

/** 实际落盘（原 confirmWrite 主体） */
async function doWrite(): Promise<void> {
  writing.value = true
  try {
    const result = await writeToolConfig(targetPath.value.trim(), mergedText.value, true)
    const backupNote = result.backupPath ? `（原文件已备份为 ${result.backupPath}）` : ''
    message.success(`已写入：${result.path} ${backupNote}`, { duration: 8000 })
    emit('written', result.path)
    emit('update:show', false)
  } catch (err) {
    message.error(`写入失败：${err instanceof Error ? err.message : String(err)}`)
  } finally {
    writing.value = false
  }
}

function close(): void {
  emit('update:show', false)
}
</script>

<template>
  <NModal
    :show="show"
    preset="card"
    :title="`一键填写 — ${toolLabel} 配置`"
    class="ms-quickfill"
    :bordered="false"
    size="huge"
    style="width: min(1200px, 96vw)"
    :mask-closable="false"
    @update:show="(v: boolean) => emit('update:show', v)"
  >
    <div class="flex h-full min-h-0 flex-1 flex-col gap-2">
      <!-- 目标路径 -->
      <div class="flex shrink-0 flex-wrap items-center gap-2">
        <span class="text-[11px] text-ms-text-faint">目标文件</span>
        <NInput
          v-model:value="targetPath"
          size="small"
          class="min-w-[420px] flex-1"
          placeholder="目标配置文件的绝对路径（支持 ~）"
          spellcheck="false"
          @keyup.enter="reload()"
        />
        <NButton size="small" :loading="loading" @click="reload()">重新读取</NButton>
        <NTag v-if="file" size="small" :bordered="false" :type="file.exists ? 'success' : 'warning'">
          {{ file.exists ? '文件已存在' : '文件不存在，将新建' }}
        </NTag>
        <NTooltip trigger="hover" style="max-width: 260px">
          <template #trigger>
            <NButton size="small" quaternary type="primary" @click="scrollToConfirm()">
              ↓ 到底部确认
            </NButton>
          </template>
          一键把对比内容滚到最底部。「确认填写」按钮固定在对话框最下方,随时可点。
        </NTooltip>
        <NTag v-if="pathDirty" size="small" :bordered="false" type="error">
          路径已修改，请点「重新读取」
        </NTag>
        <NTooltip v-if="jsonTarget" trigger="hover" style="max-width: 320px">
          <template #trigger>
            <NButton
              size="small"
              disabled
              style="color: #0a0e14; background-color: #0a0e14; border-color: #0a0e14"
            >
              注释不可用（JSON）
            </NButton>
          </template>
            目标是严格 JSON：不允许注释，基础信息（名字 / 模型数）无法以注释写入。
            点「转 JSONC」把目标换成同名 .jsonc（原文件不动）。
        </NTooltip>
        <NButton v-if="jsonTarget" size="small" quaternary @click="convertTargetToJsonc()">
          转 JSONC
        </NButton>
        <NCheckbox
          v-else-if="commentableTarget"
          size="small"
          :checked="providerCommentOn"
          @update:checked="providerCommentOn = $event"
        >
          基础信息注释
        </NCheckbox>
      </div>

      <!-- 状态与警告 -->
      <div v-if="fatal" class="shrink-0">
        <NAlert type="error" :bordered="false" style="border-radius: 0">
          <template #header><span class="text-[11px]">无法自动合并</span></template>
          <div class="text-[11px] leading-relaxed">{{ fatal }}</div>
        </NAlert>
      </div>
      <div v-else class="flex shrink-0 flex-col gap-1.5">
        <NAlert v-if="mergeModeLabel" type="info" :bordered="false" style="border-radius: 0">
          <template #header><span class="text-[11px]">{{ mergeModeLabel }}</span></template>
          <div class="flex flex-col gap-1 text-[11px] leading-relaxed">
            <span v-if="!file?.exists">
              目录不存在时会自动创建；确认前请核对右侧内容与路径。
            </span>
            <span v-else>
              覆盖前会自动把原文件备份为
              <code class="text-ms-accent">&lt;原文件名&gt;.modelscout.bak</code>。
            </span>
            <span v-if="keyNotice" class="text-ms-warning">⚠️ {{ keyNotice }}</span>
            <span v-for="w in mergeWarnings" :key="w" class="text-ms-warning">⚠️ {{ w }}</span>
          </div>
        </NAlert>
      </div>

      <!-- 模型参数调整(写入前的人工修正,右侧预览实时跟随) -->
      <div class="shrink-0">
        <NButton v-if="!overridesOpen" size="tiny" quaternary @click="openOverrides()">
          ⚙ 调整模型参数（思考 / 附件 / 工具 / 上下文 / 输出）
        </NButton>
        <template v-else>
          <div class="mb-1 flex items-center justify-between">
            <span class="text-[11px] text-ms-text-dim">
              模型参数调整 —— 右侧预览与最终写入都使用这里的值
              <NTag
                v-if="tool === 'opencode' && effectiveSchema === 'v2'"
                size="tiny"
                :bordered="false"
                type="info"
                class="ml-1"
              >
                V2：强度档位 / 思考预算 / 温度 / 输入上限可调
              </NTag>
            </span>
            <NButton
              size="tiny"
              quaternary
              @click="overridesOpen = false; overrideRows = []"
            >
              收起并还原
            </NButton>
          </div>
          <div class="max-h-[30vh] overflow-y-auto">
            <ModelEditTable
              :rows="overrideRows"
              :show-tools="tool === 'opencode'"
              :show-reasoning="true"
              :v2="tool === 'opencode' && effectiveSchema === 'v2'"
            />
          </div>
        </template>
      </div>

      <!-- 左右对比：双面板（纯 flex，无 grid / 无 sticky） -->
      <!--
        结构：列头（静态，不参与滚动）+ 共享纵向滚动区；每侧面板独立横向
        滚动（长行互不挤兑，与 VSCode 相同）。行是普通 flex 块、行背景为
        内联 rgba：此前 grid + sticky + 双向滚动的组合会触发 WebView2 的
        合成层重绘缺陷（滚动后行背景只留首屏），本结构不含任何触发条件。
        两面板渲染同一 DiffRow 序列（空侧占位），行高一致 → 天然对齐。
      -->
      <div class="flex min-h-0 flex-1 flex-col">
        <!-- 列头 -->
        <div class="flex shrink-0 border-b border-ms-border text-[10px] text-ms-text-faint" style="background-color: var(--color-ms-panel-2)">
          <div class="flex min-w-0 flex-1 items-center gap-2 border-r border-ms-border px-2 py-1">
            <span class="w-7 shrink-0 text-right">行</span>
            <span class="truncate">原文件（左）</span>
          </div>
          <div class="flex min-w-0 flex-1 items-center gap-2 px-2 py-1">
            <span class="w-7 shrink-0 text-right">行</span>
            <span class="truncate">填写后（右）</span>
          </div>
        </div>

        <!-- 共享纵向滚动 + 双面板 -->
        <div ref="diffScrollRef" class="min-h-0 flex-1 overflow-y-auto">
          <div v-if="loading && diffRows.length === 0" class="px-3 py-8 text-center text-[11px] text-ms-text-faint">
            正在读取目标文件…
          </div>
          <div v-else-if="diffRows.length === 0" class="px-3 py-8 text-center text-[11px] text-ms-text-faint">
            没有可展示的内容
          </div>
          <div v-else class="flex items-start">
            <!-- 左面板 -->
            <div class="min-w-0 flex-1 overflow-x-auto border-r border-ms-border">
              <div
                v-for="(row, index) in diffRows"
                :key="index"
                class="flex w-max items-start"
                :style="row.type === 'del' ? 'min-width: 100%; background-image: linear-gradient(rgba(239, 68, 68, 0.14), rgba(239, 68, 68, 0.14))' : 'min-width: 100%'"
              >
                <span
                  class="w-10 shrink-0 select-none px-2 text-right leading-5"
                  :class="row.type === 'del' ? 'ms-diff-del-no' : 'ms-diff-no'"
                >{{ row.leftNo ?? '' }}</span>
                <span
                  class="whitespace-pre px-2 leading-5"
                  v-html="row.left !== null ? highlightLine(row.left) : ''"
                />
              </div>
            </div>
            <!-- 右面板 -->
            <div class="min-w-0 flex-1 overflow-x-auto">
              <div
                v-for="(row, index) in diffRows"
                :key="index"
                class="flex w-max items-start"
                :style="row.type === 'add' ? 'min-width: 100%; background-image: linear-gradient(rgba(16, 185, 129, 0.14), rgba(16, 185, 129, 0.14))' : 'min-width: 100%'"
              >
                <span
                  class="w-10 shrink-0 select-none px-2 text-right leading-5"
                  :class="row.type === 'add' ? 'ms-diff-add-no' : 'ms-diff-no'"
                >{{ row.rightNo ?? '' }}</span>
                <span
                  class="whitespace-pre px-2 leading-5"
                  v-html="row.right !== null ? highlightLine(row.right) : ''"
                />
              </div>
            </div>
          </div>
        </div>
      </div>

      <!-- 底部（确认区固定可见；顶部的「↓ 到底部确认」把对比内容滚到最后） -->
      <div class="flex shrink-0 items-center justify-between gap-3 border-t border-ms-border pt-2">
        <div class="text-[11px] text-ms-text-faint">
          <template v-if="diffRows.length > 0">
            对比：{{ stats.total }} 行 · <span class="ms-diff-stat-add">+{{ stats.added }}</span>
            · <span class="ms-diff-stat-del">-{{ stats.removed }}</span>
          </template>
          <template v-else-if="!loading && !fatal">没有可展示的内容</template>
        </div>
        <div class="flex items-center gap-2">
          <NButton size="small" quaternary @click="close()">取消</NButton>
          <NButton
            size="small"
            type="primary"
            :loading="writing"
            :disabled="!canWrite"
            @click="confirmWrite()"
          >
            确认写入
          </NButton>
        </div>
      </div>
    </div>
  </NModal>

  <!-- 写入前兜底：引用的环境变量本机不存在 → 是否顺手创建 -->
  <NModal
    :show="missingPromptShow"
    preset="card"
    title="引用的环境变量不存在"
    style="width: 480px"
    :mask-closable="false"
    @update:show="(v: boolean) => (missingPromptShow = v)"
  >
    <div class="flex flex-col gap-3">
      <div class="text-[11px] leading-relaxed text-ms-text-dim">
        即将写入的配置引用了以下环境变量，但本机找不到它们。现在填值即可顺手创建
        （本应用内立即生效，Windows 会写入系统持久环境，<strong>新开的</strong>程序才能读到）；
        也可以跳过，之后在配置管理 → 环境变量密钥里再建。
      </div>
      <div
        v-for="name in pendingMissing"
        :key="name"
        class="flex flex-col gap-1 rounded border border-ms-border bg-ms-panel-2 p-2.5"
      >
        <div class="flex items-center justify-between">
          <span class="font-mono text-[12px] text-ms-accent">{{ name }}</span>
          <NTag size="tiny" :bordered="false" type="warning">本机不存在</NTag>
        </div>
        <NInput
          :value="missingValues[name] ?? ''"
          type="password"
          show-password-on="click"
          placeholder="密钥值（留空 = 跳过这个变量，直接写入配置）"
          spellcheck="false"
          @update:value="(v: string) => (missingValues[name] = v)"
        />
      </div>
    </div>
    <template #footer>
      <div class="flex items-center justify-between gap-2">
        <NButton size="small" quaternary @click="missingPromptShow = false">取消写入</NButton>
        <div class="flex items-center gap-2">
          <NButton size="small" quaternary @click="writeWithoutCreating()">仍要写入</NButton>
          <NButton size="small" type="primary" @click="createThenWrite()">创建并写入</NButton>
        </div>
      </div>
    </template>
  </NModal>
</template>

<style>
/*
 * 全局样式（不能 scoped）：NModal 会 Teleport 到 body,组件的 scoped
 * data-v 属性到不了卡片内部 —— 此前对话框因此失去高度约束,内容把
 * 卡片撑到上万像素高,滚动发生在外层遮罩上,既让「确认按钮要滚很久」,
 * 也触发 WebView2 在遮罩滚动下的行背景丢失。
 */
.ms-quickfill.n-card {
  height: min(860px, 92vh);
  display: flex;
  flex-direction: column;
}
/* 注意类名是 n-card-content（连字符），不是 n-card__content */
.ms-quickfill .n-card-content,
.ms-quickfill .n-card__content {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}
</style>

<style scoped>

/* 左右对比行的字号（等宽字体由全局 .ms-code 提供） */
.ms-code {
  font-size: 11.5px;
  line-height: 1.5;
}

/* 语法高亮配色见 style.css（随主题明暗切换） */
</style>
