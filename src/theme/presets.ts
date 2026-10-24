/**
 * 主题令牌与预设配色。
 *
 * 全套颜色由 20 个令牌组成,四个预设(默认 / 浅色 / 深色 / GitHub)各持有一
 * 份完整取值;「自定义」主题同样持有这 20 个令牌的一份拷贝(默认值取自
 * 「默认」预设),由用户逐个修改。
 *
 * 运行时由 stores/theme.ts 把当前生效的颜色写到 <html> 的内联 CSS 变量上
 * (见 CSS_VAR_OF),Tailwind v4 的 ms-* 工具类与 style.css 里的
 * var(--color-ms-*) 引用都会跟随变化;Naive UI 侧由 theme/naive.ts 按同一
 * 份颜色构建 themeOverrides,保证组件与自定义元素视觉一致。
 */

/** 一套完整主题颜色(20 个令牌) */
export interface ThemeColors {
  /** 页面最底背景 */
  bg: string
  /** 面板背景(卡片 / 标题栏) */
  panel: string
  /** 次级面板(弹窗 / 表头) */
  panel2: string
  /** 浮层 / 进度轨道等更高一层 */
  elevated: string
  /** 输入框背景 */
  inputBg: string
  /** 常规边框 */
  border: string
  /** 强调边框(输入框描边) */
  borderStrong: string
  /** 悬停边框 / 滚动条悬停 */
  borderHover: string
  /** 主文字 */
  text: string
  /** 次要文字 */
  textDim: string
  /** 弱化文字 / 占位符 */
  textFaint: string
  /** 强调色(品牌色) */
  accent: string
  /** 强调色·悬停 */
  accentHover: string
  /** 强调色·按下 */
  accentDim: string
  /** 强调色·光晕(选中背景,带透明度) */
  accentGlow: string
  /** 语义色:成功 */
  success: string
  /** 语义色:警告 */
  warning: string
  /** 语义色:危险 */
  danger: string
  /** 语义色:信息 */
  info: string
  /** 紫色标记(平台识别等) */
  purple: string
}

/** 主题 id(四个预设 + 自定义) */
export type ThemeId = 'default' | 'light' | 'dark' | 'github' | 'custom'

/** 预设主题 */
export interface ThemePreset {
  id: Exclude<ThemeId, 'custom'>
  /** 设置页显示名 */
  label: string
  /** 设置页一句话说明 */
  description: string
  /** 明 / 暗(决定 Naive UI 用暗色主题还是亮色主题、原生标题栏颜色) */
  scheme: 'dark' | 'light'
  colors: ThemeColors
}

/** 令牌 → CSS 变量名(写到 <html> 上,Tailwind ms-* 与 style.css 引用它们) */
export const CSS_VAR_OF: Record<keyof ThemeColors, string> = {
  bg: '--color-ms-bg',
  panel: '--color-ms-panel',
  panel2: '--color-ms-panel-2',
  elevated: '--color-ms-elevated',
  inputBg: '--color-ms-input-bg',
  border: '--color-ms-border',
  borderStrong: '--color-ms-border-strong',
  borderHover: '--color-ms-border-hover',
  text: '--color-ms-text',
  textDim: '--color-ms-text-dim',
  textFaint: '--color-ms-text-faint',
  accent: '--color-ms-accent',
  accentHover: '--color-ms-accent-hover',
  accentDim: '--color-ms-accent-dim',
  accentGlow: '--color-ms-accent-glow',
  success: '--color-ms-success',
  warning: '--color-ms-warning',
  danger: '--color-ms-danger',
  info: '--color-ms-info',
  purple: '--color-ms-purple',
}

/** 默认配色(当前发布版本的青绿暗色,自定义主题的初始值也取自这里) */
export const DEFAULT_PRESET: ThemePreset = {
  id: 'default',
  label: '默认',
  description: '青绿强调 · 暗色(自首个版本沿用至今)',
  scheme: 'dark',
  colors: {
    bg: '#0a0e14',
    panel: '#111721',
    panel2: '#161d29',
    elevated: '#1c2433',
    inputBg: '#0d131d',
    border: '#232c3d',
    borderStrong: '#2e3a4f',
    borderHover: '#3d4b63',
    text: '#d6deeb',
    textDim: '#8b98ad',
    textFaint: '#5c6b82',
    accent: '#14b8a6',
    accentHover: '#2dd4bf',
    accentDim: '#0d9488',
    accentGlow: 'rgba(20, 184, 166, 0.15)',
    success: '#22c55e',
    warning: '#f59e0b',
    danger: '#ef4444',
    info: '#3b82f6',
    purple: '#a78bfa',
  },
}

/** 四个预设,顺序即设置页展示顺序 */
export const PRESETS: ThemePreset[] = [
  DEFAULT_PRESET,
  {
    id: 'light',
    label: '浅色',
    description: '青绿强调 · 亮底,白天/强光环境友好',
    scheme: 'light',
    colors: {
      bg: '#eef1f6',
      panel: '#ffffff',
      panel2: '#f4f6fa',
      elevated: '#e8ecf3',
      inputBg: '#ffffff',
      border: '#dbe1ea',
      borderStrong: '#c2cbd8',
      borderHover: '#a5b1c2',
      text: '#1f2733',
      textDim: '#5a6577',
      textFaint: '#8b95a7',
      accent: '#0d9488',
      accentHover: '#14b8a6',
      accentDim: '#0f766e',
      accentGlow: 'rgba(13, 148, 136, 0.12)',
      success: '#16a34a',
      warning: '#d97706',
      danger: '#dc2626',
      info: '#2563eb',
      purple: '#7c3aed',
    },
  },
  {
    id: 'dark',
    label: '深色',
    description: '中性黑 + 蓝色强调,更纯粹的夜间配色',
    scheme: 'dark',
    colors: {
      bg: '#09090b',
      panel: '#101013',
      panel2: '#17171b',
      elevated: '#1e1e24',
      inputBg: '#0c0c0e',
      border: '#26262b',
      borderStrong: '#3a3a42',
      borderHover: '#4d4d57',
      text: '#e4e4e7',
      textDim: '#a1a1aa',
      textFaint: '#71717a',
      accent: '#3b82f6',
      accentHover: '#60a5fa',
      accentDim: '#2563eb',
      accentGlow: 'rgba(59, 130, 246, 0.16)',
      success: '#22c55e',
      warning: '#f59e0b',
      danger: '#ef4444',
      info: '#22d3ee',
      purple: '#a78bfa',
    },
  },
  {
    id: 'github',
    label: 'GitHub',
    description: 'GitHub Primer 浅色风格(白底 · 蓝)',
    scheme: 'light',
    colors: {
      bg: '#f6f8fa',
      panel: '#ffffff',
      panel2: '#f6f8fa',
      elevated: '#eaeef2',
      inputBg: '#ffffff',
      border: '#d0d7de',
      borderStrong: '#afb8c1',
      borderHover: '#8c959f',
      text: '#1f2328',
      textDim: '#59636e',
      textFaint: '#8c959f',
      accent: '#0969da',
      accentHover: '#218bff',
      accentDim: '#0550ae',
      accentGlow: 'rgba(9, 105, 218, 0.12)',
      success: '#1a7f37',
      warning: '#9a6700',
      danger: '#cf222e',
      info: '#0969da',
      purple: '#8250df',
    },
  },
]

/** 全部主题 id(含 custom),顺序即设置页展示顺序 */
export const THEME_IDS: ThemeId[] = [...PRESETS.map((p) => p.id), 'custom'] as ThemeId[]

/** presetById 的返回:预设,或代表「自定义」的伪预设 */
export type ResolvedTheme = Omit<ThemePreset, 'id'> & { id: ThemeId }

/** 按 id 取预设;未知 id 回退默认 */
export function presetById(id: ThemeId): ResolvedTheme {
  if (id === 'custom') {
    return {
      id: 'custom',
      label: '自定义',
      description: '所有颜色都可改,默认值取自「默认」配色',
      scheme: 'dark',
      colors: { ...DEFAULT_PRESET.colors },
    }
  }
  return PRESETS.find((p) => p.id === id) ?? DEFAULT_PRESET
}

/**
 * 猜测一套颜色的明暗(用于自定义主题:按背景亮度决定 Naive UI 与原生
 * 标题栏走暗色还是亮色)。
 *
 * accentGlow 等 rgba 令牌不参与判断,只看 bg。
 */
export function guessScheme(colors: ThemeColors): 'dark' | 'light' {
  const hex = colors.bg.trim()
  const m = /^#?([0-9a-f]{6})$/i.exec(hex)
  if (!m) return 'dark'
  const r = parseInt(m[1].slice(0, 2), 16) / 255
  const g = parseInt(m[1].slice(2, 4), 16) / 255
  const b = parseInt(m[1].slice(4, 6), 16) / 255
  // 相对亮度(WCAG 简化线性版),0.5 为界
  const luminance = 0.2126 * r + 0.7152 * g + 0.0722 * b
  return luminance >= 0.5 ? 'light' : 'dark'
}

/** 令牌是否为「允许带透明度」的颜色(取色器开 alpha 通道) */
export function tokenAllowsAlpha(key: keyof ThemeColors): boolean {
  return key === 'accentGlow'
}
