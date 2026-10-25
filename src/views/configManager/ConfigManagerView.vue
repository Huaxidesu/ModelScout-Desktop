<script setup lang="ts">
/**
 * 配置管理页：本机 OpenCode / dsh 模型配置的统一管理入口。
 *
 * 三个能力（对应需求）：
 *   - 一键识别：扫描本机配置文件（scan_tool_configs），解析出 provider 清单；
 *   - 一键管理：删除 provider、绑定 Key 管家密钥（回写配置文件，均带确认）；
 *   - Key 管家联动：把配置里的明文 Key 导入 Key 管家；从 Key 管家选一条
 *     Key 绑到 provider 上（OpenCode 写明文，dsh 写 apiKeyEnv 引用）。
 *
 * 所有写盘动作复用一键填写的基础设施（write_tool_config + 自动备份）。
 */

import { computed, onMounted, ref, watch } from 'vue'
import {
  NAlert,
  NButton,
  NModal,
  NSelect,
  NTag,
  NTooltip,
  useDialog,
  useMessage,
} from 'naive-ui'

import { ProbeApiError } from '@/api/commands'
import { deleteEnvSecret, listEnvSecrets, scanToolConfigs, statusEnvSecret, writeToolConfig } from '@/api/commands'
import {
  parseDshProviders,
  parseOpenCodeProviders,
  rawDshProvider,
  rawOpenCodeProvider,
  suggestEnvVarName,
  type DshScanEntry,
  type OpenCodeScanEntry,
} from '@/generators/configScan'
import { removeOpenCodeProvider, upsertOpenCodeProvider } from '@/generators/opencodeMerge'
import { mergeDshProfileIntoText, removeDshProvider } from '@/generators/dsh'
import { collectEnvVarRefs, missingEnvVars, type EnvRefSource } from '@/utils/envRefs'
import { useKeyButlerStore } from '@/stores/keyButler'
import ProviderEditDialog from './ProviderEditDialog.vue'
import EnvSecretDialog from './EnvSecretDialog.vue'
import RawFileEditorDialog from './RawFileEditorDialog.vue'
import type { EnvSecretInfo, ToolConfigFile } from '@/types/contracts'

const props = defineProps<{
  /** 页签是否处于激活态(激活时刷新环境变量清单,捕捉其他入口的增删) */
  active?: boolean
}>()

const message = useMessage()
const dialog = useDialog()
const keyButler = useKeyButlerStore()

/* ---- 扫描状态 ---- */

/** 各工具扫到的文件 */
const opencodeFiles = ref<ToolConfigFile[]>([])
const dshFiles = ref<ToolConfigFile[]>([])
/** 扫描进行中 */
const scanning = ref(false)
/** 是否扫过（区分「还没扫」与「扫了没有」） */
const hasScanned = ref(false)
/** 解析过程产生的非致命问题 */
const scanErrors = ref<string[]>([])

/** 一个文件 + 它解析出的 provider 清单 */
interface OpenCodeGroup {
  file: ToolConfigFile
  entries: OpenCodeScanEntry[]
}

interface DshGroup {
  file: ToolConfigFile
  entries: DshScanEntry[]
}

const opencodeGroups = computed<OpenCodeGroup[]>(() =>
  opencodeFiles.value.map((file) => ({
    file,
    entries: parseOpenCodeProviders(file.content).entries,
  })),
)

const dshGroups = computed<DshGroup[]>(() =>
  dshFiles.value.map((file) => ({
    file,
    entries: parseDshProviders(file.content).entries,
  })),
)

const totalProviders = computed(
  () =>
    opencodeGroups.value.reduce((sum, g) => sum + g.entries.length, 0) +
    dshGroups.value.reduce((sum, g) => sum + g.entries.length, 0),
)

/** 文件路径缩短显示：只保留「父目录 / 文件名」，完整路径放 title 里 */
function shortenPath(path: string): string {
  const normalized = path.replace(/\\/g, '/')
  const parts = normalized.split('/').filter(Boolean)
  if (parts.length <= 2) return path
  return `…/${parts[parts.length - 2]}/${parts[parts.length - 1]}`
}

/** 一键识别 */
async function scan(): Promise<void> {
  scanning.value = true
  try {
    const result = await scanToolConfigs()
    opencodeFiles.value = result.opencode
    dshFiles.value = result.dsh
    const errors: string[] = []
    for (const file of result.opencode) {
      errors.push(...parseOpenCodeProviders(file.content).errors)
    }
    for (const file of result.dsh) {
      errors.push(...parseDshProviders(file.content).errors)
    }
    scanErrors.value = errors
    hasScanned.value = true
    const total = result.opencode.length + result.dsh.length
    if (total === 0) {
      message.info('没找到 OpenCode / dsh 的配置文件；生成过配置后这里会出现条目。', { duration: 5000 })
    } else {
      message.success(`识别完成：${total} 个配置文件`)
    }
    // Key 管家列表用于「绑定密钥」选择与导入联动（未加载过则顺带拉一次）
    if (keyButler.entries.length === 0 && !keyButler.loading) {
      await keyButler.load().catch(() => undefined)
    }
  } catch (error) {
    message.error(error instanceof ProbeApiError ? error.message : String(error))
  } finally {
    scanning.value = false
  }
}

onMounted(() => {
  // 首次进入自动扫一遍：这个页面的价值就在于「打开就能看到现状」
  void scan()
  // 环境变量清单一并加载（本地命令，无磁盘扫描）
  void loadEnvSecrets()
})

// 页签激活时刷新环境变量清单:变量可能在别处(生成配置的创建提示、
// 配置管理以外的入口)被增删,面板显示的是本组件持有的快照
watch(
  () => props.active,
  (active) => {
    if (active) void loadEnvSecrets()
  },
)

/* ---- 环境变量密钥子模块 ---- */

/** 本机疑似密钥的环境变量（掩码展示） */
const envSecrets = ref<EnvSecretInfo[]>([])
/** 清单加载中 */
const envLoading = ref(false)

/** 加载环境变量密钥清单 */
async function loadEnvSecrets(): Promise<void> {
  envLoading.value = true
  try {
    envSecrets.value = await listEnvSecrets()
  } catch (error) {
    message.error(error instanceof ProbeApiError ? error.message : String(error))
  } finally {
    envLoading.value = false
  }
}

/** 全部 provider 的环境变量引用：变量名 → 引用来源列表 */
const envRefs = computed(() =>
  collectEnvVarRefs(
    opencodeGroups.value.flatMap((g) => g.entries),
    dshGroups.value.flatMap((g) => g.entries),
  ),
)

/** 变量名 → 可读的引用方描述（「OpenCode:shusheng」） */
function refsLabel(name: string): string[] {
  return (envRefs.value.get(name) ?? []).map((r: EnvRefSource) =>
    r.tool === 'opencode' ? `OpenCode:${r.providerKey}` : `dsh:${r.providerKey}`,
  )
}

/** 引用了、且经精确检查确认存在的非清单变量名（不进缺失提醒） */
const refNamesExisting = ref<Set<string>>(new Set())

// 清单按「疑似密钥」命名启发式过滤，配置引用的变量（如 MY_KEY）未必在
// 清单里 —— 对清单外的引用名逐一做精确存在性检查，避免误报缺失。
watch(envRefs, (refs) => {
  const known = new Set(envSecrets.value.map((s) => s.name))
  const toCheck = [...refs.keys()].filter((n) => !known.has(n))
  const existing = new Set<string>()
  void Promise.all(
    toCheck.map(async (n) => {
      try {
        const status = await statusEnvSecret(n)
        if (status.exists) existing.add(n)
      } catch {
        /* 查不了就按不存在处理（下一轮 watch 会再试） */
      }
    }),
  ).then(() => {
    refNamesExisting.value = existing
  })
})

/** 配置引用了、但本机还不存在的变量（需要创建才能生效） */
const missingRefVars = computed(() =>
  missingEnvVars(envRefs.value, [
    ...envSecrets.value.map((s) => s.name),
    ...refNamesExisting.value,
  ]),
)

/** 编辑/新建对话框状态 */
const envDialogShow = ref(false)
/** 锁定的变量名（编辑已有引用时不可改） */
const envDialogLockName = ref(false)
const envDialogInitialName = ref('')
const envDialogTitle = ref('')
/** picker 模式（只选名字）当前未用，保留给后续 */
const envDialogRef = ref<InstanceType<typeof EnvSecretDialog> | null>(null)

function openEnvCreate(name = ''): void {
  envDialogLockName.value = false
  envDialogInitialName.value = name
  envDialogTitle.value = name ? `创建环境变量 ${name}` : '新建环境变量密钥'
  envDialogShow.value = true
}

function openEnvEdit(secret: EnvSecretInfo): void {
  envDialogLockName.value = false
  envDialogInitialName.value = secret.name
  envDialogTitle.value = `覆盖环境变量 ${secret.name}`
  envDialogShow.value = true
}

/** 从 provider 行联动：编辑它引用的环境变量（名字锁定） */
function openEnvEditRef(name: string): void {
  envDialogLockName.value = true
  envDialogInitialName.value = name
  envDialogTitle.value = `编辑环境变量 ${name}`
  envDialogShow.value = true
}

/** 保存成功后的统一提示（区分进程内生效与持久化） */
function onEnvSaved(name: string): void {
  const note = envDialogRef.value?.savedNote
  if (note && !note.persisted && note.note) {
    message.warning(`${name}：${note.note}`, { duration: 8000 })
  } else {
    message.success(
      `${name} 已保存：本应用内立即生效${note?.persisted ? '，系统持久环境已更新（新开的程序可读）' : ''}。`,
      { duration: 6000 },
    )
  }
  void loadEnvSecrets()
}

/* ---- 编辑原始文件 ---- */

const rawEditorShow = ref(false)
const rawEditorPath = ref('')
const rawEditorTool = ref<'opencode' | 'dsh'>('opencode')

function openRawEditor(tool: 'opencode' | 'dsh', path: string): void {
  rawEditorTool.value = tool
  rawEditorPath.value = path
  rawEditorShow.value = true
}

function onRawSaved(): void {
  message.success('原始文件已保存（原文件已备份），正在重新识别…')
  void scan()
}

function confirmEnvDelete(secret: EnvSecretInfo): void {
  const refs = refsLabel(secret.name)
  dialog.warning({
    title: `删除环境变量 ${secret.name}？`,
    content:
      (refs.length > 0 ? `配置里正被 ${refs.join('、')} 引用，删除后这些配置会失效。` : '') +
      '会同时从本应用与 Windows 用户级持久环境中移除，无法恢复。',
    positiveText: '删除',
    negativeText: '取消',
    onPositiveClick: async () => {
      try {
        await deleteEnvSecret(secret.name)
        message.success(`${secret.name} 已删除`)
        await loadEnvSecrets()
      } catch (error) {
        message.error(error instanceof ProbeApiError ? error.message : String(error))
      }
    },
  })
}

/* ---- Key 管家联动：配置 → Key 管家 ---- */

/**
 * 把配置里的明文 Key 导入 Key 管家。
 *
 * 只有 OpenCode 的 provider 可能带明文 Key；dsh 的补丁文件本身不存明文，
 * 按钮不可点并说明原因。
 */
async function addToKeyButler(entry: OpenCodeScanEntry): Promise<void> {
  if (entry.apiKey.mode !== 'plain') return
  const before = new Set(keyButler.entries.map((e) => e.id))
  try {
    const result = await keyButler.importKeys({ text: entry.apiKey.value })
    if (result.added === 0) {
      message.info('这条 Key 已经在 Key 管家里了。')
      return
    }
    // 找到新导入的条目，补上 Base URL（识别平台不一定能推出正确的端点）
    const added = keyButler.entries.filter((e) => !before.has(e.id))
    if (added.length === 1 && entry.baseURL) {
      await keyButler.setConfig(added[0].id, { baseUrl: entry.baseURL })
    }
    message.success(`已加入 Key 管家（${entry.key} → ${added[0]?.maskedKey ?? '新条目'}）`)
  } catch (error) {
    message.error(error instanceof ProbeApiError ? error.message : String(error))
  }
}

/* ---- 编辑 provider（模型参数） ---- */

/** 编辑对话框状态 */
const editShow = ref(false)
const editTarget = ref<{ tool: 'opencode' | 'dsh'; path: string; providerKey: string } | null>(null)

/** 打开编辑（逐模型调整思考/附件/工具/上下文/输出） */
function openEdit(tool: 'opencode' | 'dsh', path: string, providerKey: string): void {
  editTarget.value = { tool, path, providerKey }
  editShow.value = true
}

/** 编辑保存成功后重扫，刷新文件缓存与列表 */
async function onEdited(): Promise<void> {
  await scan()
}

/* ---- Key 管家联动：Key 管家 → 配置（绑定密钥） ---- */

/** 绑定对话框状态 */
const bindShow = ref(false)
const bindTarget = ref<{ tool: 'opencode' | 'dsh'; path: string; providerKey: string } | null>(null)
const bindKeyId = ref<string | null>(null)

/** Key 管家条目的下拉选项 */
const keyOptions = computed(() =>
  keyButler.entries.map((e) => ({
    label: `${e.maskedKey}${e.note ? ` · ${e.note}` : ''}${e.baseUrl ? ` · ${e.baseUrl}` : ''}`,
    value: e.id,
  })),
)

function openBind(tool: 'opencode' | 'dsh', path: string, providerKey: string): void {
  if (keyButler.entries.length === 0) {
    message.warning('Key 管家里还没有 Key，先去导入或从探测页存一条。', { duration: 5000 })
    return
  }
  bindTarget.value = { tool, path, providerKey }
  bindKeyId.value = null
  bindShow.value = true
}

/** 确认绑定：OpenCode 写明文 Key；dsh 写 apiKeyEnv 环境变量引用 */
async function confirmBind(): Promise<void> {
  const target = bindTarget.value
  const keyId = bindKeyId.value
  if (!target || !keyId) return

  const entry = keyButler.entries.find((e) => e.id === keyId)
  if (!entry) return

  try {
    if (target.tool === 'opencode') {
      // 读出明文（瞬态使用，写完即弃）并写进 provider 的密钥字段。
      // 字段名跟随文件世代：v1 在 options.apiKey，v2 在 settings.apiKey。
      const plain = await keyButler.reveal(keyId)
      const raw = rawOpenCodeProvider(fileContent(target.path), target.providerKey)
      if (!raw) throw new Error('provider 已不存在，请重新识别。')
      const holderKey = raw.schema === 'v2' ? 'settings' : 'options'
      raw.value[holderKey] = {
        ...((raw.value[holderKey] as Record<string, unknown> | undefined) ?? {}),
        apiKey: plain,
      }
      const result = upsertOpenCodeProvider(fileContent(target.path), target.providerKey, raw.value)
      await writeToolConfig(target.path, result.text, true)
      message.success(
        `已把 ${entry.maskedKey} 明文写入 ${target.providerKey}（原文件已备份）。⚠️ 配置含明文 Key，请勿外传。`,
        { duration: 6000 },
      )
    } else {
      // dsh：补丁文件不支持明文，写环境变量引用
      const envName = suggestEnvVarName(target.providerKey)
      const raw = rawDshProvider(fileContent(target.path), target.providerKey)
      if (!raw) throw new Error('provider 已不存在，请重新识别。')
      raw.apiKeyEnv = envName
      const result = mergeDshProfileIntoText(fileContent(target.path), target.providerKey, raw)
      await writeToolConfig(target.path, result.text, true)
      message.success(
        `已把 ${target.providerKey} 的密钥引用改为环境变量 ${envName}。dsh 不支持明文 Key，请设置该环境变量或在 dsh 界面保存凭据。`,
        { duration: 8000 },
      )
    }
    bindShow.value = false
    await scan()
  } catch (error) {
    message.error(error instanceof ProbeApiError || error instanceof Error ? error.message : String(error))
  }
}

/** 从扫描缓存里取文件内容 */
function fileContent(path: string): string {
  const oc = opencodeFiles.value.find((f) => f.path === path)
  if (oc) return oc.content
  const dsh = dshFiles.value.find((f) => f.path === path)
  return dsh?.content ?? ''
}

/* ---- 删除 provider ---- */

function confirmRemove(tool: 'opencode' | 'dsh', path: string, providerKey: string): void {
  dialog.warning({
    title: `删除 provider "${providerKey}"？`,
    content: `会从 ${path} 里移除该 provider 及其全部模型配置，其他内容保留；原文件自动备份为 .modelscout.bak。`,
    positiveText: '删除',
    negativeText: '取消',
    onPositiveClick: async () => {
      try {
        const content = fileContent(path)
        const result =
          tool === 'opencode'
            ? removeOpenCodeProvider(content, providerKey)
            : removeDshProvider(content, providerKey)
        if (!result) {
          message.info('该 provider 已不存在，重新识别即可。')
          return
        }
        await writeToolConfig(path, result.text, true)
        message.success(`已删除 ${providerKey}（原文件已备份）`)
        await scan()
      } catch (error) {
        message.error(
          error instanceof ProbeApiError || error instanceof Error ? error.message : String(error),
        )
      }
    },
  })
}
</script>

<template>
  <div class="flex min-h-0 flex-1 flex-col">
    <!-- 工具栏 -->
    <div class="flex shrink-0 flex-wrap items-center gap-3 border-b border-ms-border bg-ms-panel px-4 py-2">
      <span class="ms-panel-title">配置管理</span>
      <NTooltip trigger="hover" style="max-width: 380px">
        <template #trigger>
          <NButton size="small" type="primary" :loading="scanning" @click="scan()">
            {{ scanning ? '识别中…' : '⚡ 一键识别' }}
          </NButton>
        </template>
        扫描本机的 OpenCode（~/.config/opencode/）与 dsh（~/.dsh/profiles/*/cordis.patch.yml）
        配置文件，解析出 provider 与模型清单。
      </NTooltip>
      <span class="text-[11px] text-ms-text-faint">
        管理 OpenCode / dsh 的模型配置，与 Key 管家双向联动；所有写盘自动备份。
      </span>
      <span class="ml-auto text-[11px] text-ms-text-dim" v-if="hasScanned">
        共 {{ totalProviders }} 个 provider
      </span>
    </div>

    <!-- 解析问题提示 -->
    <NAlert
      v-if="scanErrors.length > 0"
      type="warning"
      :bordered="false"
      class="mx-3 mt-2 shrink-0"
      style="border-radius: 0"
    >
      <template #header><span class="text-[11px]">部分内容没能识别（已跳过）</span></template>
      <div class="text-[11px] leading-relaxed">
        <div v-for="err in scanErrors.slice(0, 5)" :key="err">{{ err }}</div>
      </div>
    </NAlert>

    <!-- 列表 -->
    <div class="min-h-0 flex-1 overflow-y-auto px-3 py-2">
      <!-- 空态：一个配置文件都没找到 -->
      <div
        v-if="hasScanned && opencodeFiles.length === 0 && dshFiles.length === 0"
        class="ms-panel ms-scanlines flex h-full flex-col items-center justify-center gap-4"
      >
        <div class="text-4xl opacity-20">🧭</div>
        <div class="text-sm text-ms-text-dim">没有找到配置文件</div>
        <div class="max-w-md text-center text-xs leading-relaxed text-ms-text-faint">
          本机还没有 OpenCode / dsh 的模型配置。<br />
          先在「探测」页跑一次探测，用「一键填写」生成配置；或手动创建配置文件后再回来识别。
        </div>
      </div>

      <!-- 有文件但没有 provider：给出可行动的提示 -->
      <NAlert
        v-if="hasScanned && opencodeFiles.length + dshFiles.length > 0 && totalProviders === 0"
        type="info"
        :bordered="false"
        class="shrink-0"
        style="border-radius: 0"
      >
        <template #header><span class="text-[11px]">找到了配置文件，但里面还没有模型 provider</span></template>
        <div class="text-[11px] leading-relaxed">
          去「探测」页跑一次探测，用「一键填写」把 provider 写进配置；写完回这里点「一键识别」。
        </div>
      </NAlert>

      <!-- OpenCode 分组 -->
      <section v-for="group in opencodeGroups" :key="group.file.path" class="mb-3">
        <div class="mb-1 flex items-center gap-2">
          <NTag size="small" :bordered="false" type="success">OpenCode</NTag>
          <span class="text-[11px] text-ms-text-faint" :title="group.file.path">
            {{ shortenPath(group.file.path) }}
          </span>
          <NButton size="tiny" quaternary @click="openRawEditor('opencode', group.file.path)">
            ✎ 编辑原始文件
          </NButton>
        </div>
        <div class="ms-panel overflow-hidden">
          <div
            v-for="entry in group.entries"
            :key="entry.key"
            class="flex flex-wrap items-center gap-x-3 gap-y-1 border-b border-ms-border px-3 py-2 last:border-b-0 hover:bg-white/[0.02]"
          >
            <span class="min-w-[110px] font-mono text-[12px] text-ms-accent">{{ entry.key }}</span>
            <NTag size="tiny" :bordered="false" :type="entry.schema === 'v2' ? 'success' : 'default'">
              {{ entry.schema }}
            </NTag>
            <span class="text-[11px] text-ms-text-dim">{{ entry.name }}</span>
            <span class="text-[11px] text-ms-text-faint">{{ entry.baseURL || '（无 baseURL）' }}</span>
            <NTag
              size="tiny"
              :bordered="false"
              :type="entry.apiKey.mode === 'plain' ? 'warning' : entry.apiKey.mode === 'env' ? 'info' : 'default'"
            >
              {{
                entry.apiKey.mode === 'plain'
                  ? '明文 Key'
                  : entry.apiKey.mode === 'env'
                    ? `env ${entry.apiKey.envVar}`
                    : '无 Key'
              }}
            </NTag>
            <NTag size="tiny" :bordered="false">{{ entry.modelIds.length }} 个模型</NTag>
            <span class="ml-auto flex items-center gap-1.5">
              <NTooltip v-if="entry.apiKey.mode === 'env'" trigger="hover">
                <template #trigger>
                  <NButton
                    size="tiny"
                    quaternary
                    type="info"
                    @click="openEnvEditRef(entry.apiKey.envVar)"
                  >
                    编辑变量
                  </NButton>
                </template>
                直接修改环境变量 {{ entry.apiKey.envVar }} 的值（本应用内立即生效 + 写入系统持久环境）。
              </NTooltip>
              <NTooltip v-if="entry.apiKey.mode === 'plain'" trigger="hover">
                <template #trigger>
                  <NButton size="tiny" quaternary type="primary" @click="addToKeyButler(entry)">
                    加入 Key 管家
                  </NButton>
                </template>
                把这个 provider 里的明文 Key 导入 Key 管家（自动带上 Base URL）。
              </NTooltip>
              <NTooltip v-if="entry.apiKey.mode !== 'plain'" trigger="hover">
                <template #trigger>
                  <NButton size="tiny" quaternary disabled>加入 Key 管家</NButton>
                </template>
                只有配置里写了明文 Key 的 provider 才能一键导入；环境变量引用/无 Key 的请手动导入。
              </NTooltip>
              <NButton size="tiny" quaternary @click="openEdit('opencode', group.file.path, entry.key)">
                编辑
              </NButton>
              <NButton size="tiny" quaternary @click="openBind('opencode', group.file.path, entry.key)">
                绑定密钥
              </NButton>
              <NButton size="tiny" quaternary type="error" @click="confirmRemove('opencode', group.file.path, entry.key)">
                删除
              </NButton>
            </span>
            <!-- 模型清单 -->
            <div v-if="entry.modelIds.length > 0" class="w-full pl-1 text-[10px] text-ms-text-faint">
              {{ entry.modelIds.join(' · ') }}
            </div>
          </div>
        </div>
      </section>

      <!-- dsh 分组 -->
      <section v-for="group in dshGroups" :key="group.file.path" class="mb-3">
        <div class="mb-1 flex items-center gap-2">
          <NTag size="small" :bordered="false" type="info">DeepSeek Harness</NTag>
          <span class="text-[11px] text-ms-text-faint" :title="group.file.path">
            {{ shortenPath(group.file.path) }}
          </span>
          <NButton size="tiny" quaternary @click="openRawEditor('dsh', group.file.path)">
            ✎ 编辑原始文件
          </NButton>
        </div>
        <div class="ms-panel overflow-hidden">
          <div
            v-for="entry in group.entries"
            :key="entry.key"
            class="flex flex-wrap items-center gap-x-3 gap-y-1 border-b border-ms-border px-3 py-2 last:border-b-0 hover:bg-white/[0.02]"
          >
            <span class="min-w-[110px] font-mono text-[12px] text-ms-accent">{{ entry.key }}</span>
            <span class="text-[11px] text-ms-text-dim">{{ entry.name }}</span>
            <span class="text-[11px] text-ms-text-faint">{{ entry.baseURL || '（无 baseURL）' }}</span>
            <NTag size="tiny" :bordered="false" :type="entry.apiKeyEnv ? 'info' : 'default'">
              {{ entry.apiKeyEnv ? `env ${entry.apiKeyEnv}` : '无密钥引用' }}
            </NTag>
            <NTag size="tiny" :bordered="false">{{ entry.modelIds.length }} 个模型</NTag>
            <span class="ml-auto flex items-center gap-1.5">
              <NTooltip v-if="entry.apiKeyEnv" trigger="hover">
                <template #trigger>
                  <NButton size="tiny" quaternary type="info" @click="openEnvEditRef(entry.apiKeyEnv)">
                    编辑变量
                  </NButton>
                </template>
                直接修改环境变量 {{ entry.apiKeyEnv }} 的值（本应用内立即生效 + 写入系统持久环境）。
              </NTooltip>
              <NTooltip trigger="hover">
                <template #trigger>
                  <NButton size="tiny" quaternary disabled>加入 Key 管家</NButton>
                </template>
                dsh 的补丁文件不存明文 Key（只引用环境变量），无法直接导入。
              </NTooltip>
              <NButton size="tiny" quaternary @click="openEdit('dsh', group.file.path, entry.key)">
                编辑
              </NButton>
              <NButton size="tiny" quaternary @click="openBind('dsh', group.file.path, entry.key)">
                绑定密钥
              </NButton>
              <NButton size="tiny" quaternary type="error" @click="confirmRemove('dsh', group.file.path, entry.key)">
                删除
              </NButton>
            </span>
            <div v-if="entry.modelIds.length > 0" class="w-full pl-1 text-[10px] text-ms-text-faint">
              {{ entry.modelIds.join(' · ') }}
            </div>
          </div>
        </div>
      </section>

      <!-- 环境变量密钥子模块 -->
      <section class="mb-3">
        <div class="mb-1 flex flex-wrap items-center gap-2">
          <NTag size="small" :bordered="false" type="warning">环境变量密钥</NTag>
          <span class="text-[11px] text-ms-text-faint">
            本机疑似密钥的环境变量（值只显示掩码）；配置里的
            <code>{env:…}</code> 引用与此联动
          </span>
          <span class="ml-auto flex items-center gap-1.5">
            <NButton size="tiny" quaternary type="primary" @click="openEnvCreate()">
              新建变量
            </NButton>
            <NButton size="tiny" quaternary :loading="envLoading" @click="loadEnvSecrets()">
              刷新
            </NButton>
          </span>
        </div>

        <!-- 配置引用了但本机没有的变量：联动创建入口 -->
        <NAlert
          v-if="missingRefVars.length > 0"
          type="warning"
          :bordered="false"
          class="mb-1"
          style="border-radius: 0"
        >
          <template #header><span class="text-[11px]">配置引用了 {{ missingRefVars.length }} 个本机不存在的环境变量</span></template>
          <div class="flex flex-wrap items-center gap-2 text-[11px] leading-relaxed">
            <span v-for="name in missingRefVars" :key="name" class="flex items-center gap-1">
              <code class="text-ms-warning">{{ name }}</code>
              <span class="text-ms-text-faint">← {{ refsLabel(name).join('、') || '未知来源' }}</span>
              <NButton size="tiny" quaternary type="warning" @click="openEnvCreate(name)">
                创建
              </NButton>
            </span>
          </div>
        </NAlert>

        <div class="ms-panel overflow-hidden">
          <div
            v-for="secret in envSecrets"
            :key="secret.name"
            class="flex flex-wrap items-center gap-x-3 gap-y-1 border-b border-ms-border px-3 py-2 last:border-b-0 hover:bg-white/[0.02]"
          >
            <span class="min-w-[180px] font-mono text-[12px] text-ms-accent">{{ secret.name }}</span>
            <span class="font-mono text-[11px] text-ms-text-dim">{{ secret.preview }}</span>
            <NTag size="tiny" :bordered="false" :type="secret.persisted ? 'success' : 'default'">
              {{ secret.persisted ? '已持久化' : '仅本程序' }}
            </NTag>
            <span v-if="refsLabel(secret.name).length > 0" class="text-[10px] text-ms-text-faint">
              被 {{ refsLabel(secret.name).join('、') }} 引用
            </span>
            <span class="ml-auto flex items-center gap-1.5">
              <NButton size="tiny" quaternary @click="openEnvEdit(secret)">编辑</NButton>
              <NButton size="tiny" quaternary type="error" @click="confirmEnvDelete(secret)">删除</NButton>
            </span>
          </div>
          <div v-if="envSecrets.length === 0 && !envLoading" class="px-3 py-4 text-center text-[11px] text-ms-text-faint">
            没有发现疑似密钥的环境变量（名字含 API_KEY / APIKEY，或以 _TOKEN / _SECRET 结尾）。
            点「新建变量」可直接创建。
          </div>
          <div v-if="envLoading && envSecrets.length === 0" class="px-3 py-4 text-center text-[11px] text-ms-text-faint">
            正在读取环境变量…
          </div>
        </div>
      </section>

    </div>

    <!-- 编辑 provider(模型参数)对话框 -->
    <ProviderEditDialog
      v-model:show="editShow"
      :tool="editTarget?.tool ?? 'opencode'"
      :path="editTarget?.path ?? ''"
      :provider-key="editTarget?.providerKey ?? ''"
      :content="fileContent(editTarget?.path ?? '')"
      @saved="onEdited"
    />

    <!-- 环境变量密钥 编辑/新建 对话框 -->
    <EnvSecretDialog
      ref="envDialogRef"
      v-model:show="envDialogShow"
      :title="envDialogTitle"
      :initial-name="envDialogInitialName"
      :lock-name="envDialogLockName"
      @saved="onEnvSaved"
    />

    <!-- 原始文件编辑器 -->
    <RawFileEditorDialog
      v-model:show="rawEditorShow"
      :tool="rawEditorTool"
      :path="rawEditorPath"
      @saved="onRawSaved"
    />

    <!-- 绑定密钥对话框 -->
    <NModal
      v-model:show="bindShow"
      preset="card"
      title="绑定 Key 管家密钥"
      style="width: 520px"
      :mask-closable="false"
    >
      <div class="flex flex-col gap-3">
        <div class="text-[11px] leading-relaxed text-ms-text-dim">
          选择一条 Key 管家里的密钥，写入 provider
          <code class="text-ms-accent">{{ bindTarget?.providerKey }}</code>
          ：
          <template v-if="bindTarget?.tool === 'opencode'">
            OpenCode 支持明文 Key，将把明文写入 options.apiKey（原文件自动备份）。
          </template>
          <template v-else>
            dsh 的补丁文件不支持明文 Key，将写入 apiKeyEnv 环境变量引用 ——
            你需要自行设置该环境变量，或在 dsh 的 Settings → Models 页面保存凭据。
          </template>
        </div>
        <NSelect
          v-model:value="bindKeyId"
          size="small"
          filterable
          placeholder="选择 Key 管家里的密钥"
          :options="keyOptions"
        />
      </div>
      <template #footer>
        <div class="flex justify-end gap-2">
          <NButton size="small" quaternary @click="bindShow = false">取消</NButton>
          <NButton size="small" type="primary" :disabled="!bindKeyId" @click="confirmBind()">
            确认绑定
          </NButton>
        </div>
      </template>
    </NModal>
  </div>
</template>
