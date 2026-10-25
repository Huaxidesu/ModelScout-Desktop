<script setup lang="ts">
/**
 * 模型参数编辑表（配置管理「编辑」与一键填写「参数调整」共用）。
 *
 * 只做展示与就地编辑（rows 由父组件持有，直接改行对象字段）；
 * 三态语义见 modelOverrides.ts：null = 配置未设置 / 未探测到。
 * 行内以「未设置」小标提示原值缺失的字段。
 *
 * v2 列（强度档位 / 思考预算 / 温度 / 输入上限）仅在 `v2` 为真时显示，
 * 对应 OpenCode 2 的 capabilities.reasoning.effort / budget /
 * capabilities.temperature / limit.input。
 */

import { NInputNumber, NSelect, NSwitch, NTag } from 'naive-ui'

import type { ModelEditRow } from '@/generators/modelOverrides'

const props = defineProps<{
  rows: ModelEditRow[]
  /** 是否显示「工具调用」列（dsh 没有该字段） */
  showTools?: boolean
  /** 是否显示「思考」列（v1 布尔；v2 是 capabilities.reasoning.supported） */
  showReasoning?: boolean
  /** 是否显示 OpenCode V2 独有列（强度档位 / 预算 / 温度 / 输入上限） */
  v2?: boolean
}>()

void props

/** 强度档位的候选（可自定义输入其他档位名） */
const EFFORT_OPTIONS = ['off', 'minimal', 'low', 'medium', 'high'].map((v) => ({
  label: v,
  value: v,
}))

/** NSelect(multiple+tag) 的选中值 → string[] | null（空 = 未设置） */
function onEffortUpdate(
  row: ModelEditRow,
  value: string | number | Array<string | number> | null | undefined,
): void {
  const arr = Array.isArray(value) ? value.map((v) => String(v)) : []
  row.effort = arr.length > 0 ? arr : null
}
</script>

<template>
  <div class="overflow-x-auto rounded border border-ms-border">
    <table class="w-full border-collapse text-[11px]">
      <thead>
        <tr class="bg-ms-panel-2 text-left text-ms-text-faint">
          <th class="px-2 py-1 font-medium">模型</th>
          <th v-if="showReasoning" class="px-2 py-1 font-medium" :title="v2 ? 'capabilities.reasoning.supported' : ''">思考</th>
          <th class="px-2 py-1 font-medium">附件/图片</th>
          <th v-if="showTools" class="px-2 py-1 font-medium">工具调用</th>
          <th v-if="v2" class="px-2 py-1 font-medium" title="capabilities.reasoning.effort：声明该模型支持哪些推理强度档位">强度档位</th>
          <th v-if="v2" class="px-2 py-1 font-medium" title="capabilities.reasoning.budget：思考预算范围（token）">思考预算</th>
          <th v-if="v2" class="px-2 py-1 font-medium" title="capabilities.temperature">温度</th>
          <th class="px-2 py-1 font-medium">上下文</th>
          <th v-if="v2" class="px-2 py-1 font-medium" title="limit.input">输入上限</th>
          <th class="px-2 py-1 font-medium">输出上限</th>
        </tr>
      </thead>
      <tbody>
        <tr
          v-for="row in rows"
          :key="row.id"
          class="border-t border-ms-border/60 hover:bg-white/[0.02]"
        >
          <td class="max-w-[220px] px-2 py-1">
            <div class="truncate font-mono text-ms-text" :title="row.id">{{ row.id }}</div>
            <div v-if="row.name && row.name !== row.id" class="truncate text-[10px] text-ms-text-faint">
              {{ row.name }}
            </div>
          </td>

          <td v-if="showReasoning" class="px-2 py-1">
            <NSwitch :value="row.reasoning ?? false" size="small" @update:value="row.reasoning = $event" />
            <span v-if="row.reasoning === null" class="ml-1 text-[10px] text-ms-text-faint">未设置</span>
          </td>

          <td class="px-2 py-1">
            <NSwitch :value="row.vision ?? false" size="small" @update:value="row.vision = $event" />
            <span v-if="row.vision === null" class="ml-1 text-[10px] text-ms-text-faint">未设置</span>
          </td>

          <td v-if="showTools" class="px-2 py-1">
            <NSwitch :value="row.tools ?? false" size="small" @update:value="row.tools = $event" />
            <span v-if="row.tools === null" class="ml-1 text-[10px] text-ms-text-faint">未设置</span>
          </td>

          <td v-if="v2" class="px-2 py-1">
            <NSelect
              :value="row.effort ?? []"
              multiple
              tag
              size="tiny"
              filterable
              placeholder="未设置"
              :options="EFFORT_OPTIONS"
              style="width: 190px"
              @update:value="(v) => onEffortUpdate(row, v)"
            />
          </td>

          <td v-if="v2" class="px-2 py-1">
            <div class="flex items-center gap-1">
              <NInputNumber
                :value="row.budgetMin"
                size="tiny"
                :show-button="false"
                :min="0"
                :step="1024"
                placeholder="min"
                style="width: 84px"
                @update:value="row.budgetMin = $event"
              />
              <span class="text-ms-text-faint">–</span>
              <NInputNumber
                :value="row.budgetMax"
                size="tiny"
                :show-button="false"
                :min="0"
                :step="1024"
                placeholder="max"
                style="width: 84px"
                @update:value="row.budgetMax = $event"
              />
            </div>
          </td>

          <td v-if="v2" class="px-2 py-1">
            <NSwitch :value="row.temperature ?? false" size="small" @update:value="row.temperature = $event" />
            <span v-if="row.temperature === null" class="ml-1 text-[10px] text-ms-text-faint">未设置</span>
          </td>

          <td class="px-2 py-1">
            <NInputNumber
              :value="row.contextWindow"
              size="tiny"
              :show-button="false"
              :min="1024"
              :step="1024"
              placeholder="未设置"
              style="width: 96px"
              @update:value="row.contextWindow = $event"
            />
          </td>

          <td v-if="v2" class="px-2 py-1">
            <NInputNumber
              :value="row.inputLimit"
              size="tiny"
              :show-button="false"
              :min="1024"
              :step="1024"
              placeholder="未设置"
              style="width: 96px"
              @update:value="row.inputLimit = $event"
            />
          </td>

          <td class="px-2 py-1">
            <NInputNumber
              :value="row.maxOutput"
              size="tiny"
              :show-button="false"
              :min="256"
              :step="256"
              placeholder="未设置"
              style="width: 96px"
              @update:value="row.maxOutput = $event"
            />
          </td>
        </tr>
        <tr v-if="rows.length === 0">
          <td :colspan="v2 ? 10 : 6" class="px-2 py-4 text-center text-ms-text-faint">
            该 provider 里没有模型条目
          </td>
        </tr>
      </tbody>
    </table>
    <div class="flex items-center gap-2 border-t border-ms-border px-2 py-1 text-[10px] text-ms-text-faint">
      <NTag size="tiny" :bordered="false">只写动过的字段</NTag>
      「未设置」表示配置里没写、跟随工具默认判定；开关一旦拨动就会写成明确值。
      <template v-if="v2">
        强度档位对应 v2 的 capabilities.reasoning.effort（可多选、可输入自定义档位）；
        预算是思考 token 的区间。
      </template>
    </div>
  </div>
</template>
