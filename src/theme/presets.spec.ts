import { describe, expect, it } from 'vitest'

import {
  CSS_VAR_OF,
  DEFAULT_PRESET,
  PRESETS,
  THEME_IDS,
  guessScheme,
  presetById,
  type ThemeColors,
} from './presets'
import { buildNaiveThemeOverrides } from './naive'

/** 全部令牌键(以默认配色为基准) */
const ALL_KEYS = Object.keys(DEFAULT_PRESET.colors) as (keyof ThemeColors)[]

describe('主题预设 presets', () => {
  it('四个预设 id 唯一且顺序为 默认/浅色/深色/GitHub', () => {
    expect(PRESETS.map((p) => p.id)).toEqual(['default', 'light', 'dark', 'github'])
  })

  it('每个预设都持有全部 20 个令牌,且除光晕外均为 #RRGGBB', () => {
    for (const preset of PRESETS) {
      expect(Object.keys(preset.colors).sort()).toEqual([...ALL_KEYS].sort())
      for (const key of ALL_KEYS) {
        const value = preset.colors[key]
        if (key === 'accentGlow') {
          expect(value).toMatch(/^rgba\(/)
        } else {
          expect(value).toMatch(/^#[0-9a-f]{6}$/i)
        }
      }
    }
  })

  it('四个预设彼此真的不同(浅色/GitHub 亮底,默认/深色暗底)', () => {
    expect(guessScheme(PRESETS[0].colors)).toBe('dark')
    expect(guessScheme(PRESETS[1].colors)).toBe('light')
    expect(guessScheme(PRESETS[2].colors)).toBe('dark')
    expect(guessScheme(PRESETS[3].colors)).toBe('light')
    expect(PRESETS[1].colors.bg).not.toBe(PRESETS[3].colors.bg)
    expect(PRESETS[1].colors.accent).not.toBe(PRESETS[3].colors.accent)
  })

  it('默认配色与发布版本的原始取值一致(自定义主题以此为初始值)', () => {
    const c = DEFAULT_PRESET.colors
    expect(c.bg).toBe('#0a0e14')
    expect(c.panel).toBe('#111721')
    expect(c.panel2).toBe('#161d29')
    expect(c.inputBg).toBe('#0d131d')
    expect(c.borderHover).toBe('#3d4b63')
    expect(c.accent).toBe('#14b8a6')
    expect(c.accentGlow).toBe('rgba(20, 184, 166, 0.15)')
  })

  it('CSS 变量映射覆盖全部令牌且变量名不重复', () => {
    expect(Object.keys(CSS_VAR_OF).sort()).toEqual([...ALL_KEYS].sort())
    const vars = Object.values(CSS_VAR_OF)
    expect(new Set(vars).size).toBe(vars.length)
    for (const v of vars) expect(v).toMatch(/^--color-ms-[a-z0-9-]+$/)
  })

  it('presetById 对未知 id 回退默认,custom 返回默认颜色的拷贝', () => {
    expect(presetById('github').label).toBe('GitHub')
    // @ts-expect-error 故意传入非法 id
    expect(presetById('nope').id).toBe('default')
    const custom = presetById('custom')
    expect(custom.colors).toEqual(DEFAULT_PRESET.colors)
    expect(custom.colors).not.toBe(DEFAULT_PRESET.colors)
  })

  it('THEME_IDS 含 custom 且在最后', () => {
    expect(THEME_IDS).toEqual(['default', 'light', 'dark', 'github', 'custom'])
  })

  it('guessScheme:亮背景→light,暗背景→dark,非法值回退 dark', () => {
    expect(guessScheme({ ...DEFAULT_PRESET.colors, bg: '#ffffff' })).toBe('light')
    expect(guessScheme({ ...DEFAULT_PRESET.colors, bg: '#000000' })).toBe('dark')
    expect(guessScheme({ ...DEFAULT_PRESET.colors, bg: 'not-a-color' })).toBe('dark')
  })
})

describe('Naive UI 主题映射 naive', () => {
  it('common 的品牌/语义/背景色全部来自令牌', () => {
    const c = PRESETS[3].colors // GitHub
    const o = buildNaiveThemeOverrides(c)
    const common = o.common as Record<string, string>
    expect(common.primaryColor).toBe(c.accent)
    expect(common.primaryColorHover).toBe(c.accentHover)
    expect(common.primaryColorPressed).toBe(c.accentDim)
    expect(common.successColor).toBe(c.success)
    expect(common.errorColor).toBe(c.danger)
    expect(common.bodyColor).toBe(c.bg)
    expect(common.cardColor).toBe(c.panel)
    expect(common.textColor3).toBe(c.textDim)
    expect(common.placeholderColor).toBe(c.textFaint)
    expect(common.tableColor).toBe('transparent')
  })

  it('Input 边框由令牌拼接,DataTable/Log/Tabs 使用对应层级', () => {
    const c = DEFAULT_PRESET.colors
    const o = buildNaiveThemeOverrides(c)
    expect((o.Input as Record<string, string>).border).toBe(`1px solid ${c.borderStrong}`)
    expect((o.Input as Record<string, string>).borderFocus).toBe(`1px solid ${c.accent}`)
    expect((o.DataTable as Record<string, string>).thColor).toBe(c.panel2)
    expect((o.Log as Record<string, string>).color).toBe(c.inputBg)
    expect((o.Tabs as Record<string, string>).barColor).toBe(c.accent)
  })

  it('换一套颜色就得到另一份覆盖(响应式换肤的基础)', () => {
    const a = buildNaiveThemeOverrides(PRESETS[0].colors)
    const b = buildNaiveThemeOverrides(PRESETS[1].colors)
    expect((a.common as Record<string, string>).bodyColor).not.toBe(
      (b.common as Record<string, string>).bodyColor,
    )
  })
})
