<script setup lang="ts">
/**
 * ============================================================================
 * ConnectionForm —— 输入区
 * ============================================================================
 *
 * 【职责】
 *   1. 采集 Base URL、API Key、并发数、超时、是否启用上限探测
 *   2. 提供「测试连接」预检按钮
 *   3. 提供「获取模型列表」按钮，并允许用户从中挑选要探测的模型
 *
 * 【安全提示】
 * API Key 使用 type="password" 输入框，并提供「显示/隐藏」切换。
 * Key 只保存在前端内存（Pinia store）中，不写 localStorage、不写文件。
 * ============================================================================
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

/* ---------------------------------------------------------------------------
 * 常用端点预设
 * ---------------------------------------------------------------------------
 * 这些是开发中最常用的 OpenAI 兼容端点，能省去大量手输。
 * 用户仍可自由编辑，预设只做「一键填充」。
 */
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

/* ---------------------------------------------------------------------------
 * 模型挑选
 * ------------------------------------------------------------------------- */

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

/* ---------------------------------------------------------------------------
 * 从文本识别连接信息
 * ---------------------------------------------------------------------------
 * 用户拿到端点和密钥的场合几乎从来不是两个干净的输入框，而是聊天记录、
 * 服务商文档、或者一份现成的配置文件。这个入口负责把那段文字整段吃进来，
 * 抠出 Base URL 与 API Key 并填入。
 *
 * 【为什么粘贴后直接填入，而不是先让用户确认】
 * 「粘贴即用」正是这个功能的全部意义。填入的是两个可见的输入框，
 * 用户一眼能看出对不对，改起来也只是重输一次 —— 比每次多按一遍
 * 「确认」划算得多。因此只在**什么都没识别到时**才拦一下。
 */

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
 * 还没进入 `v-model`（那是随后的 `input` 事件才发生的），
 * 此刻读取 `recognizeText` 拿到的还是旧值。
 *
 * 刻意**不** `preventDefault` —— 让浏览器按默认行为完成粘贴，
 * 用户看到框里出现的正是自己粘的东西，所见即所得。
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
          <NSelect
            :options="PRESETS"
            :disabled="disabled"
            placeholder="预设"
            style="width: 130px"
            @update:value="applyPreset"
          />
        </NInputGroup>
      </div>

      <!-- ---------- API Key ---------- -->
      <div class="col-span-4">
        <label class="mb-1 block text-[11px] text-ms-text-dim">API Key</label>
        <NInput
          v-model:value="probe.store.config.apiKey"
          :type="revealKey ? 'text' : 'password'"
          :disabled="disabled"
          show-password-on="click"
          placeholder="sk-..."
          clearable
          @click="revealKey = !revealKey"
        />
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

    <!-- ============ 从文本识别 ============ -->
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

    <!-- ============ 模型挑选弹窗 ============ -->
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
