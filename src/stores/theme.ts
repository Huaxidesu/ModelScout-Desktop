/**
 * 主题 store:当前选中的主题 / 自定义颜色,以及把它们真正「刷」到界面上。
 *
 * 生效方式(三处同步):
 *   1. <html> 内联 CSS 变量 —— Tailwind 的 ms-* 工具类、style.css 与各组件
 *      里的 var(--color-ms-*) 全部跟随;`:root` 上的 color-scheme 一起切,
 *      让原生滚动条 / 表单控件的默认渲染匹配明暗;
 *   2. Naive UI —— App.vue 用这里的 colors 构建 themeOverrides,暗色主题配
 *      darkTheme、亮色主题配 null;
 *   3. 原生窗口标题栏 —— @tauri-apps/api 的 setTheme(需 capability 授权,
 *      失败只影响标题栏颜色,不影响页面)。
 *
 * 持久化:localStorage(modelscout.theme.v1),存主题 id + 自定义颜色拷贝;
 * 解析失败 / 结构不符时静默回退默认,不让坏数据把界面刷花。
 */

import { computed, ref, watch } from 'vue'
import { defineStore } from 'pinia'
import { getCurrentWindow } from '@tauri-apps/api/window'
import {
  CSS_VAR_OF,
  DEFAULT_PRESET,
  PRESETS,
  guessScheme,
  type ThemeColors,
  type ThemeId,
} from '@/theme/presets'

/** localStorage 键名 */
const STORAGE_KEY = 'modelscout.theme.v1'

/** 从 localStorage 恢复;返回 null 表示没有或数据不合法 */
function loadPersisted(): { themeId: ThemeId; customColors: ThemeColors } | null {
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY)
    if (!raw) return null
    const parsed = JSON.parse(raw) as { themeId?: unknown; customColors?: unknown }
    const validIds = new Set<string>([...PRESETS.map((p) => p.id), 'custom'])
    if (typeof parsed.themeId !== 'string' || !validIds.has(parsed.themeId)) return null
    if (
      typeof parsed.customColors !== 'object' ||
      parsed.customColors === null ||
      !Object.keys(CSS_VAR_OF).every((k) => k in (parsed.customColors as Record<string, unknown>))
    ) {
      return null
    }
    return {
      themeId: parsed.themeId as ThemeId,
      customColors: { ...(parsed.customColors as ThemeColors) },
    }
  } catch {
    return null
  }
}

export const useThemeStore = defineStore('theme', () => {
  const persisted = typeof window === 'undefined' ? null : loadPersisted()

  /** 当前主题 id */
  const themeId = ref<ThemeId>(persisted?.themeId ?? 'default')

  /** 自定义主题的颜色拷贝(初始值取自「默认」配色) */
  const customColors = ref<ThemeColors>({
    ...(persisted?.customColors ?? DEFAULT_PRESET.colors),
  })

  /** 当前生效的完整颜色(预设直接引用,自定义走拷贝) */
  const colors = computed<ThemeColors>(() => {
    if (themeId.value === 'custom') return customColors.value
    const preset = PRESETS.find((p) => p.id === themeId.value)
    return preset ? preset.colors : DEFAULT_PRESET.colors
  })

  /** 当前明暗(自定义主题按背景亮度猜) */
  const scheme = computed<'dark' | 'light'>(() =>
    themeId.value === 'custom' ? guessScheme(customColors.value) : (PRESETS.find((p) => p.id === themeId.value)?.scheme ?? 'dark'),
  )

  /** 把当前颜色写到 <html>:CSS 变量 + color-scheme + 原生标题栏 */
  function applyToDocument(): void {
    if (typeof document === 'undefined') return
    const root = document.documentElement
    for (const key of Object.keys(CSS_VAR_OF) as (keyof ThemeColors)[]) {
      root.style.setProperty(CSS_VAR_OF[key], colors.value[key])
    }
    root.style.colorScheme = scheme.value

    // 原生标题栏跟随明暗(浏览器/测试环境没有 Tauri,失败可忽略)
    if (typeof window !== 'undefined') {
      try {
        void getCurrentWindow().setTheme(scheme.value)
      } catch {
        /* 无 Tauri 环境或权限缺失:只影响标题栏,不阻塞换肤 */
      }
    }
  }

  /** 写入 localStorage(坏环境直接跳过) */
  function persist(): void {
    if (typeof window === 'undefined') return
    try {
      window.localStorage.setItem(
        STORAGE_KEY,
        JSON.stringify({ themeId: themeId.value, customColors: customColors.value }),
      )
    } catch {
      /* 隐私模式等场景写不进去就算了,本次会话内仍生效 */
    }
  }

  // 切主题 / 改自定义色 → 立即应用 + 持久化(store 创建时先手动执行一次,
  // 保证启动首帧就是用户选的主题,不闪默认色)
  watch([themeId, customColors], () => { applyToDocument(); persist() }, { deep: true })
  applyToDocument()

  /** 切换主题 */
  function setTheme(id: ThemeId): void {
    themeId.value = id
  }

  /** 修改自定义主题的一个颜色(自动切到自定义主题) */
  function setCustomColor(key: keyof ThemeColors, value: string): void {
    themeId.value = 'custom'
    customColors.value = { ...customColors.value, [key]: value }
  }

  /** 自定义配色恢复为「默认」配色(当前仍停留在自定义主题) */
  function resetCustomColors(): void {
    customColors.value = { ...DEFAULT_PRESET.colors }
  }

  return {
    themeId,
    customColors,
    colors,
    scheme,
    setTheme,
    setCustomColor,
    resetCustomColors,
  }
})
