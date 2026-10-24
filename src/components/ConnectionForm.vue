<script setup lang="ts">
/**
 * 连接配置输入区：采集 Base URL、API Key、并发数、超时与上限探测开关，
 * 提供「测试连接」预检与「获取模型列表」按钮，并支持挑选探测范围。
 *
 * API Key 支持三种来源：明文输入（password 型遮蔽，可切换显示）、
 * 环境变量（只填变量名，由后端读取）、Windows 凭据管理器（只填凭据名，由后端读取）。
 * 无论哪种方式，Key 都只保存在前端内存（Pinia store）中，不写 localStorage、不写文件；
 * 非明文模式下明文不会经过前端渲染层。
 */

import { computed, ref } from 'vue'
import {
  NButton,
  NInput,
  NInputGroup,
  NInputNumber,
  NSwitch,
  NSelect,
  NTooltip,
  NModal,
  NCard,
  NCheckbox,
  NCheckboxGroup,
  NSpace,
  NEmpty,
  NTag,
  useMessage,
} from 'naive-ui'

import { useProbe } from '@/composables/useProbe'
import { extractConnection, maskSecret, type ExtractResult } from '@/utils/parseConnection'
import type { ApiKeySource } from '@/types/contracts'

const props = defineProps<{
  /** 由 App.vue 传入的探测组合式对象 */
  probe: ReturnType<typeof useProbe>
}>()

const message = useMessage()

/** 是否显示 API Key 明文 */
const revealKey = ref(false)

/** 模型挑选弹窗是否打开 */
const pickerOpen = ref(false)

/** 弹窗中的临时勾选集 */
const pickedIds = ref<string[]>([])

/** 当前是否禁用输入（探测中不允许改配置） */
const disabled = computed(() => props.probe.store.isRunning)

// ---- 密钥来源 ----
// 三种提供方式均只把「明文」交给后端，非明文模式下前端界面不接触密钥内容。
// 实际读取在后端 `ProbeConfig::resolve_api_key()` 完成。
const KEY_SOURCE_OPTIONS = [
  { label: '明文', value: 'plain' },
  { label: '环境变量', value: 'env' },
  { label: '凭据管理器', value: 'credential' },
]

/** 当前密钥来源（用于模板分支） */
const keySource = computed<ApiKeySource>(() => props.probe.store.config.apiKeySource)

/** 切换密钥来源 */
function onKeySourceChange(value: string | number | null): void {
  if (typeof value !== 'string') return
  props.probe.store.config.apiKeySource = value as ApiKeySource
}

// ---- 常用端点预设 ----
// 常用 OpenAI 兼容端点，用于一键填充；用户仍可自由编辑。
const PRESETS = [
  { label: 'OpenAI', value: 'https://api.openai.com/v1' },
  { label: 'DeepSeek', value: 'https://api.deepseek.com/v1' },
  { label: '阿里通义千问 (DashScope)', value: 'https://dashscope.aliyuncs.com/compatible-mode/v1' },
  { label: '月之暗面 Moonshot', value: 'https://api.moonshot.cn/v1' },
  { label: '智谱 GLM', value: 'https://open.bigmodel.cn/api/paas/v4' },
  { label: '硅基流动 SiliconFlow', value: 'https://api.siliconflow.cn/v1' },
  { label: '字节火山方舟', value: 'https://ark.cn-beijing.volces.com/api/v3' },
  { label: '本地 Ollama', value: 'http://localhost:11434/v1' },
  { label: '本地 LM Studio', value: 'http://localhost:1234/v1' },
  { label: '本地 vLLM', value: 'http://localhost:8000/v1' },
]

/** 应用一个预设 */
function applyPreset(value: string): void {
  props.probe.store.config.baseUrl = value
}

// ---- 模型挑选 ----

/** 打开挑选弹窗，默认全不选（让用户主动挑） */
function openPicker(): void {
  pickedIds.value = [...(props.probe.store.config.onlyModels ?? [])]
  pickerOpen.value = true
}

/** 确认挑选 */
function confirmPicker(): void {
  props.probe.store.config.onlyModels = [...pickedIds.value]
  pickerOpen.value = false
}

/** 挑选=全部：清空白名单，表示探测全部模型 */
function clearPicker(): void {
  pickedIds.value = []
  props.probe.store.config.onlyModels = []
  pickerOpen.value = false
}

/** 白名单的展示文案 */
const scopeLabel = computed(() => {
  const list = props.probe.store.config.onlyModels
  if (!list || list.length === 0) return '全部模型'
  return `已选 ${list.length} 个模型`
})

// ---- 从文本识别连接信息 ----
// 端点与密钥通常来自聊天记录、服务商文档或现成配置文件，此处接收整段文本，
// 提取 Base URL 与 API Key 并填入。
//
// 粘贴后直接填入而不先要求确认：填入目标是两个可见输入框，内容对错一目了然，
// 重填成本低；仅在什么都没识别到时才提示。

const recognizeOpen = ref(false)
const recognizeText = ref('')
const recognizeResult = ref<ExtractResult | null>(null)

/** 打开识别弹窗（每次都是干净状态） */
function openRecognize(): void {
  recognizeText.value = ''
  recognizeResult.value = null
  recognizeOpen.value = true
}

/** 两个字段是否都识别到了 */
const recognizeComplete = computed(
  () => Boolean(recognizeResult.value?.baseUrl && recognizeResult.value?.apiKey),
)

/**
 * 执行识别并填入连接配置。
 *
 * @param notify 是否弹出结果提示（手动点按钮时为 true）
 */
function runRecognize(notify = true): void {
  const result = extractConnection(recognizeText.value)
  recognizeResult.value = result

  const filled: string[] = []
  if (result.baseUrl) {
    props.probe.store.config.baseUrl = result.baseUrl
    filled.push('Base URL')
  }
  if (result.apiKey) {
    props.probe.store.config.apiKey = result.apiKey
    // 识别到的是明文密钥：把来源切回「明文」。
    // 否则用户处在「环境变量 / 凭据管理器」模式时，会以为填了却根本没生效。
    props.probe.store.config.apiKeySource = 'plain'
    filled.push('API Key')
  }

  if (!notify) return

  if (filled.length === 0) {
    message.warning('没识别出可用的链接或密钥，请检查文本内容。', { duration: 5000 })
  } else {
    message.success(`已填入 ${filled.join(' 与 ')}`, { duration: 3000 })
  }
}

/**
 * 粘贴后自动识别。
 *
 * 用 `setTimeout(0)` 而不是立即执行：`paste` 事件触发时，粘贴的内容
 * 还没进入 `v-model`（那要等随后的 `input` 事件），
 * 此刻读取 `recognizeText` 拿到的是旧值。
 *
 * 不 `preventDefault`：让浏览器按默认行为完成粘贴，
 * 输入框中展示的即用户粘贴的内容。
 */
function onRecognizePaste(): void {
  window.setTimeout(() => runRecognize(true), 0)
}

/** 清空输入与结果 */
function clearRecognize(): void {
  recognizeText.value = ''
  recognizeResult.value = null
}
</script>

<template>
  <section class="ms-panel px-4 py-3">
    <!-- 标题行 -->
    <div class="mb-3 flex items-center justify-between">
      <span class="ms-panel-title">连接配置</span>
      <span class="text-[11px] text-ms-text-faint">
        API Key 仅保存在本机内存中，不会写入磁盘
      </span>
    </div>

    <div class="grid grid-cols-12 gap-3">
      <!-- ---------- Base URL ---------- -->
      <div class="col-span-5">
        <label class="mb-1 block text-[11px] text-ms-text-dim">Base URL</label>
        <NInputGroup>
          <NInput
            v-model:value="probe.store.config.baseUrl"
            :disabled="disabled"
            placeholder="https://api.openai.com/v1"
            clearable
          />
          <!--
            consistent-menu-width：Naive 的 NSelect 默认让下拉菜单与触发器等宽
            （内部把 Popover 的 width 直接设成触发器的像素宽），而这里触发器只有
            130px，长预设名（「阿里通义千问 (DashScope)」）会被截成「阿里通义…」。
            关掉它之后菜单改为按内容自适应宽度，同时保留「不小于触发器」的下限。
          -->
          <NSelect
            :options="PRESETS"
            :disabled="disabled"
            placeholder="预设"
            style="width: 130px"
            :consistent-menu-width="false"
            @update:value="applyPreset"
          />
        </NInputGroup>
      </div>

      <!-- ---------- API Key ---------- -->
      <div class="col-span-4">
        <label class="mb-1 flex items-center gap-1 text-[11px] text-ms-text-dim">
          API Key
          <NTooltip trigger="hover" style="max-width: 320px">
            <template #trigger>
              <span class="cursor-help text-ms-text-faint">ⓘ</span>
            </template>
            三种提供方式：<br />
            1. 明文 —— 直接粘贴 Key；<br />
            2. 环境变量 —— 只填变量名，后端读取该变量的值；<br />
            3. 凭据管理器 —— 只填凭据名，后端从 Windows 凭据管理器读取。<br />
            后两种方式下明文不会出现在界面上，也不会写入磁盘。
          </NTooltip>
        </label>

        <NInputGroup>
          <!-- 密钥来源：明文 / 环境变量 / Windows 凭据管理器 -->
          <NSelect
            :value="probe.store.config.apiKeySource"
            :options="KEY_SOURCE_OPTIONS"
            :disabled="disabled"
            style="width: 112px"
            :consistent-menu-width="false"
            @update:value="onKeySourceChange"
          />

          <!--
            明文输入：
              默认 type=password，内容以圆点遮蔽。
              小眼睛图标用自定义 suffix 实现，常驻显示（不随内容有无、聚焦与否、
              当前是否明文而变化）。不使用 Naive 的 show-password-on，原因：
                1. 明文态下 type 变为 text，Naive 自带图标会消失，无法常驻；
                2. 把 @click 挂在 NInput 上会导致点输入框任意位置都切换明文。
              自定义 suffix 只把切换绑定在按钮本身，行为可控。
          -->
          <NInput
            v-if="keySource === 'plain'"
            v-model:value="probe.store.config.apiKey"
            :type="revealKey ? 'text' : 'password'"
            :disabled="disabled"
            placeholder="sk-..."
            clearable
          >
            <template #suffix>
              <button
                type="button"
                class="ms-eye-toggle"
                :aria-label="revealKey ? '隐藏 API Key' : '显示 API Key'"
                :title="revealKey ? '隐藏' : '显示'"
                :disabled="disabled"
                @click="revealKey = !revealKey"
              >
                <!-- 明文态：显示「眼睛划掉」图标，点击可隐藏 -->
                <svg
                  v-if="revealKey"
                  viewBox="0 0 24 24"
                  width="15"
                  height="15"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="2"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                  aria-hidden="true"
                >
                  <path d="M17.94 17.94A10.07 10.07 0 0 1 12 20c-7 0-11-8-11-8a18.45 18.45 0 0 1 5.06-5.94M9.9 4.24A9.12 9.12 0 0 1 12 4c7 0 11 8 11 8a18.5 18.5 0 0 1-2.16 3.19m-6.72-1.07a3 3 0 1 1-4.24-4.24" />
                  <line x1="1" y1="1" x2="23" y2="23" />
                </svg>
                <!-- 遮蔽态：显示「眼睛」图标，点击可显示 -->
                <svg
                  v-else
                  viewBox="0 0 24 24"
                  width="15"
                  height="15"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="2"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                  aria-hidden="true"
                >
                  <path d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z" />
                  <circle cx="12" cy="12" r="3" />
                </svg>
              </button>
            </template>
          </NInput>

          <!-- 环境变量名：后端读取该变量的值作为 Key -->
          <NInput
            v-else-if="keySource === 'env'"
            v-model:value="probe.store.config.apiKeyEnvVar"
            :disabled="disabled"
            placeholder="OPENAI_API_KEY"
            clearable
          />

          <!-- 凭据名：后端从 Windows 凭据管理器读取 -->
          <NInput
            v-else
            v-model:value="probe.store.config.apiKeyCredential"
            :disabled="disabled"
            placeholder="凭据名称"
            clearable
          />
        </NInputGroup>

        <!-- 非明文来源的说明：明确「明文不会经过本界面」 -->
        <div v-if="keySource !== 'plain'" class="mt-1 text-[10px] leading-relaxed text-ms-text-faint">
          {{
            keySource === 'env'
              ? '由后端读取该环境变量的值；请先在系统中配置好它。'
              : '由后端从「凭据管理器 → Windows 凭据」的通用凭据中读取。'
          }}
        </div>
      </div>

      <!-- ---------- 并发数 ---------- -->
      <div class="col-span-1">
        <label class="mb-1 flex items-center gap-1 text-[11px] text-ms-text-dim">
          并发
          <NTooltip trigger="hover">
            <template #trigger>
              <span class="cursor-help text-ms-text-faint">ⓘ</span>
            </template>
            同时进行的探测任务数（1-10）。默认 3，可避免触发服务端限流。
            若频繁出现 429 错误，请降到 1-2。
          </NTooltip>
        </label>
        <NInputNumber
          v-model:value="probe.store.config.concurrency"
          :min="1"
          :max="10"
          :disabled="disabled"
          :show-button="false"
        />
      </div>

      <!-- ---------- 超时 ---------- -->
      <div class="col-span-1">
        <label class="mb-1 flex items-center gap-1 text-[11px] text-ms-text-dim">
          超时
          <NTooltip trigger="hover">
            <template #trigger>
              <span class="cursor-help text-ms-text-faint">ⓘ</span>
            </template>
            单次 HTTP 请求超时（秒），范围 5-300。默认 30 秒。
          </NTooltip>
        </label>
        <NInputNumber
          v-model:value="probe.store.config.timeoutSecs"
          :min="5"
          :max="300"
          :disabled="disabled"
          :show-button="false"
        />
      </div>

      <!-- ---------- 上限探测开关 ---------- -->
      <div class="col-span-1">
        <label class="mb-1 flex items-center gap-1 text-[11px] text-ms-text-dim">
          上限
          <NTooltip trigger="hover">
            <template #trigger>
              <span class="cursor-help text-ms-text-faint">ⓘ</span>
            </template>
            开启后，未命中本地字典的模型会发送 max_tokens=999999 的探针，
            从服务端报错中反推上下文与输出上限。
            这会略微增加耗时与额度消耗，但能拿到更完整的参数。
          </NTooltip>
        </label>
        <NSwitch
          v-model:value="probe.store.config.probeLimits"
          :disabled="disabled"
          size="small"
        />
      </div>
    </div>

    <!-- ---------- 第二行：操作按钮 ---------- -->
    <div class="mt-3 flex items-center gap-2">
      <NTooltip trigger="hover">
        <template #trigger>
          <NButton size="small" type="info" ghost :disabled="disabled" @click="openRecognize()">
            ⤵ 从文本识别
          </NButton>
        </template>
        把带 Base URL 与 API Key 的一段文字整段粘进来，自动识别并填入
      </NTooltip>

      <NButton
        size="small"
        :loading="probe.pinging.value"
        :disabled="disabled"
        @click="probe.ping()"
      >
        测试连接
      </NButton>

      <NButton
        size="small"
        :loading="probe.loadingList.value"
        :disabled="disabled"
        @click="probe.previewModels()"
      >
        获取模型列表
      </NButton>

      <NButton
        size="small"
        type="primary"
        ghost
        :disabled="disabled || probe.discovered.value.length === 0"
        @click="openPicker()"
      >
        探测范围：{{ scopeLabel }}
      </NButton>

      <div class="flex-1" />

      <!-- 预检结果摘要 -->
      <div
        v-if="probe.pingResult.value"
        class="flex items-center gap-2 text-[11px]"
      >
        <template v-if="probe.pingResult.value.ok">
          <span class="text-ms-success">● 连接正常</span>
          <span class="text-ms-text-faint">
            发现 {{ probe.pingResult.value.modelCount }} 个模型 ·
            {{ probe.pingResult.value.elapsedMs }}ms
          </span>
        </template>
        <template v-else>
          <span class="text-ms-danger">● 连接失败</span>
          <span class="max-w-[420px] truncate text-ms-text-faint">
            {{ probe.pingResult.value.error?.message }}
          </span>
        </template>
      </div>
    </div>

    <!-- ---- 从文本识别 ---- -->
    <NModal v-model:show="recognizeOpen" preset="card" style="width: 700px">
      <NCard :bordered="false" title="从文本识别连接信息" size="small">
        <div class="mb-2 text-[11px] leading-relaxed text-ms-text-dim">
          把带 Base URL 与 API Key 的文字整段粘进下面 ——
          聊天记录、服务商文档、现成的配置文件都行。
          <strong class="text-ms-accent">粘贴后会自动识别并填入上方的连接配置。</strong>
        </div>

        <NInput
          v-model:value="recognizeText"
          type="textarea"
          :rows="7"
          placeholder="例如：&#10;Base URL: https://discovery-api.intern-ai.org.cn/v1&#10;API Key: sk-xxxxxxxxxxxxxxxxxxxx"
          @paste="onRecognizePaste"
        />

        <!-- ---------- 识别结果 ---------- -->
        <div
          v-if="recognizeResult"
          class="mt-3 rounded border border-ms-border bg-ms-bg/40 p-3 text-xs"
        >
          <div class="mb-2 flex items-center gap-2">
            <span class="font-medium text-ms-text-dim">识别结果</span>
            <NTag
              size="tiny"
              :bordered="false"
              :type="recognizeComplete ? 'success' : 'warning'"
            >
              {{ recognizeComplete ? '两项都已填入' : '只识别到部分字段' }}
            </NTag>
          </div>

          <div class="flex items-start gap-2">
            <span :class="recognizeResult.baseUrl ? 'text-ms-success' : 'text-ms-text-faint'">
              {{ recognizeResult.baseUrl ? '✓' : '○' }}
            </span>
            <span class="w-[68px] shrink-0 text-ms-text-faint">Base URL</span>
            <span class="break-all font-mono">
              {{ recognizeResult.baseUrl || '未识别到' }}
            </span>
          </div>

          <div class="mt-1 flex items-start gap-2">
            <span :class="recognizeResult.apiKey ? 'text-ms-success' : 'text-ms-text-faint'">
              {{ recognizeResult.apiKey ? '✓' : '○' }}
            </span>
            <span class="w-[68px] shrink-0 text-ms-text-faint">API Key</span>
            <span class="break-all font-mono">
              {{ recognizeResult.apiKey ? maskSecret(recognizeResult.apiKey) : '未识别到' }}
            </span>
          </div>

          <ul class="mt-2 space-y-0.5 text-[11px] leading-relaxed text-ms-text-faint">
            <li v-for="(note, index) in recognizeResult.notes" :key="index">· {{ note }}</li>
          </ul>
        </div>

        <template #footer>
          <div class="flex items-center justify-between">
            <NButton size="small" quaternary :disabled="!recognizeText" @click="clearRecognize()">
              清空
            </NButton>
            <div class="flex items-center gap-2">
              <NButton
                size="small"
                :disabled="!recognizeText"
                @click="runRecognize(true)"
              >
                重新识别
              </NButton>
              <NButton size="small" type="primary" @click="recognizeOpen = false">
                完成
              </NButton>
            </div>
          </div>
        </template>
      </NCard>
    </NModal>

    <!-- ---- 模型挑选弹窗 ---- -->
    <NModal v-model:show="pickerOpen" preset="card" style="width: 640px">
      <NCard
        :bordered="false"
        :title="`选择要探测的模型（共发现 ${probe.discovered.value.length} 个）`"
        size="small"
      >
        <div class="mb-2 flex items-center gap-2">
          <NButton
            size="tiny"
            @click="pickedIds = probe.discovered.value.map((m) => m.id)"
          >
            全选
          </NButton>
          <NButton size="tiny" @click="pickedIds = []">全不选</NButton>
          <span class="text-[11px] text-ms-text-faint">
            已选 {{ pickedIds.length }} 个。不选任何模型表示探测全部。
          </span>
        </div>

        <div
          v-if="probe.discovered.value.length === 0"
          class="py-8 text-center text-xs text-ms-text-faint"
        >
          <NEmpty description="尚未获取模型列表，请先在上方点击「获取模型列表」" />
        </div>

        <div
          v-else
          class="max-h-[380px] overflow-y-auto rounded border border-ms-border bg-ms-bg/40 p-3"
        >
          <NCheckboxGroup v-model:value="pickedIds">
            <NSpace vertical size="small">
              <NCheckbox
                v-for="model in probe.discovered.value"
                :key="model.id"
                :value="model.id"
              >
                <span class="font-mono text-xs">{{ model.id }}</span>
                <span
                  v-if="model.ownedBy"
                  class="ml-2 text-[10px] text-ms-text-faint"
                >
                  {{ model.ownedBy }}
                </span>
              </NCheckbox>
            </NSpace>
          </NCheckboxGroup>
        </div>

        <template #footer>
          <div class="flex justify-end gap-2">
            <NButton size="small" @click="clearPicker()">重置为全部</NButton>
            <NButton size="small" type="primary" @click="confirmPicker()">
              确定
            </NButton>
          </div>
        </template>
      </NCard>
    </NModal>
  </section>
</template>
