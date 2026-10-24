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
  NInput,
  NModal,
  NSpin,
  NTag,
  useMessage,
} from 'naive-ui'

import { inspectToolConfig, writeToolConfig } from '@/api/commands'
import { mergeOpenCodeIntoText } from '@/generators/opencodeMerge'
import { mergeDshIntoText } from '@/generators/dsh'
import type { GenerateContext } from '@/generators/opencode'
import { diffLines, type DiffRow } from '@/utils/diffLines'
import { highlightLine } from '@/utils/highlight'
import type { ToolConfigFile } from '@/types/contracts'

const props = defineProps<{
  /** 是否显示（v-model:show） */
  show: boolean
  /** 目标工具：OpenCode 或 DeepSeek Harness */
  tool: 'opencode' | 'dsh'
  /** 生成上下文（与配置预览区共用同一份推断与用户输入） */
  context: GenerateContext
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

/** 密钥相关的提示（两种工具行为不同） */
const keyNotice = computed(() => {
  if (!plainKeyFilled.value) return ''
  if (props.tool === 'dsh') {
    return 'dsh 的补丁文件不支持写明文 API Key，本次写入以 apiKeyEnv 引用环境变量；请设置该变量，或在 dsh 的 Settings → Models 页面保存凭据。'
  }
  return '即将写入的文件包含明文 API Key，请勿分享或提交到公开仓库。'
})

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
    if (props.tool === 'opencode') {
      const result = mergeOpenCodeIntoText(existing, props.context)
      mergedText.value = result.text
      mergeModeLabel.value =
        result.mode === 'create'
          ? '目标文件没有模型配置，将直接写入完整配置'
          : result.mode === 'splice'
            ? '已把 provider 片段并入现有配置（原有内容与注释保留）'
            : '现有文件结构特殊，改为整体重写（其他配置项保留）'
      mergeWarnings.value = result.warnings
    } else {
      const result = mergeDshIntoText(existing, props.context)
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
watch(
  () => props.show,
  (show) => {
    if (show) {
      file.value = null
      mergedText.value = ''
      mergeModeLabel.value = ''
      mergeWarnings.value = []
      fatal.value = ''
      targetPath.value = ''
      void reload()
    }
  },
)

watch(
  () => props.context,
  () => {
    if (props.show) computeMerge()
  },
  { deep: true },
)

// ---- 写入 ----

async function confirmWrite(): Promise<void> {
  if (!canWrite.value) return
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
    style="width: min(1200px, 96vw); height: min(860px, 92vh)"
    :mask-closable="false"
    @update:show="(v: boolean) => emit('update:show', v)"
  >
    <div class="flex h-full min-h-0 flex-col gap-3">
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
        <NTag v-if="pathDirty" size="small" :bordered="false" type="error">
          路径已修改，请点「重新读取」
        </NTag>
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

      <!-- 左右对比 -->
      <div class="ms-code-block ms-code min-h-0 flex-1 overflow-auto" style="border-top: 1px solid var(--color-ms-border)">
        <NSpin :show="loading" size="small">
          <div class="min-w-[720px]">
            <!-- 列头 -->
            <div
              class="sticky top-0 z-[1] grid border-b border-ms-border bg-ms-panel-2/95 text-[10px] text-ms-text-faint"
              style="grid-template-columns: 3rem 1fr 3rem 1fr"
            >
              <div class="px-2 py-1">行</div>
              <div class="border-l border-ms-border px-2 py-1">原文件（左）</div>
              <div class="border-l border-ms-border px-2 py-1">行</div>
              <div class="border-l border-ms-border px-2 py-1">填写后（右）</div>
            </div>
            <div
              v-for="(row, index) in diffRows"
              :key="index"
              class="grid"
              style="grid-template-columns: 3rem 1fr 3rem 1fr"
            >
              <div
                class="select-none px-2 text-right leading-5"
                :class="row.type === 'del' ? 'bg-red-500/10 text-red-300/70' : 'text-ms-text-faint opacity-50'"
              >
                {{ row.leftNo ?? '' }}
              </div>
              <div
                class="whitespace-pre leading-5"
                :class="row.type === 'del' ? 'bg-red-500/10' : ''"
                v-html="row.left !== null ? highlightLine(row.left) : ''"
              />
              <div
                class="select-none border-l border-ms-border px-2 text-right leading-5"
                :class="row.type === 'add' ? 'bg-emerald-500/10 text-emerald-300/70' : 'text-ms-text-faint opacity-50'"
              >
                {{ row.rightNo ?? '' }}
              </div>
              <div
                class="whitespace-pre leading-5"
                :class="row.type === 'add' ? 'bg-emerald-500/10' : ''"
                v-html="row.right !== null ? highlightLine(row.right) : ''"
              />
            </div>
          </div>
        </NSpin>
      </div>

      <!-- 底部 -->
      <div class="flex shrink-0 items-center justify-between">
        <div class="text-[11px] text-ms-text-faint">
          <template v-if="diffRows.length > 0">
            对比：{{ stats.total }} 行 · <span class="text-emerald-400">+{{ stats.added }}</span>
            · <span class="text-red-400">-{{ stats.removed }}</span>
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
</template>

<style scoped>
/* 固定高度的卡片：让内容区撑满并约束滚动 */
.ms-quickfill :deep(.n-card__content) {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
}

.ms-quickfill :deep(.n-spin-content) {
  min-width: 100%;
}

/* 左右对比行的字号（等宽字体由全局 .ms-code 提供） */
.ms-code {
  font-size: 11.5px;
  line-height: 1.5;
}

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
