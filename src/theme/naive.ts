/**
 * 把一套 ThemeColors 映射成 Naive UI 的 GlobalThemeOverrides。
 *
 * App.vue 在 NConfigProvider 上绑定这份覆盖;颜色全部来自主题令牌,切换
 * 主题时 Naive UI 组件(按钮 / 输入框 / 表格 / Tab / 进度条…)与自定义
 * 元素一起换肤。浅色主题下 NConfigProvider 的 theme 传 null(亮色),
 * 暗色主题传 darkTheme,由 App.vue 按 scheme 决定。
 */

import type { GlobalThemeOverrides } from 'naive-ui'
import type { ThemeColors } from './presets'

export function buildNaiveThemeOverrides(c: ThemeColors): GlobalThemeOverrides {
  return {
    common: {
      // 品牌色
      primaryColor: c.accent,
      primaryColorHover: c.accentHover,
      primaryColorPressed: c.accentDim,
      primaryColorSuppl: c.accent,

      // 语义色
      successColor: c.success,
      warningColor: c.warning,
      errorColor: c.danger,
      infoColor: c.info,

      // 背景层级
      bodyColor: c.bg,
      cardColor: c.panel,
      modalColor: c.panel2,
      popoverColor: c.panel2,
      tableColor: 'transparent',

      // 边框
      borderColor: c.border,
      dividerColor: c.border,

      // 文字
      textColorBase: c.text,
      textColor1: c.text,
      textColor2: c.text,
      textColor3: c.textDim,
      placeholderColor: c.textFaint,

      // 圆角与字号
      borderRadius: '6px',
      borderRadiusSmall: '4px',
      fontSize: '13px',
      fontFamily:
        "Inter, -apple-system, BlinkMacSystemFont, 'Segoe UI', 'PingFang SC', 'Microsoft YaHei', sans-serif",
      fontFamilyMono:
        "'JetBrains Mono', 'Cascadia Code', 'Fira Code', Consolas, 'Courier New', monospace",
    },
    Card: {
      color: c.panel,
      borderColor: c.border,
      paddingMedium: '14px 16px',
    },
    DataTable: {
      thColor: c.panel2,
      thTextColor: c.textDim,
      tdColor: 'transparent',
      tdColorHover: c.panel2,
      borderColor: c.border,
      thFontWeight: '600',
    },
    Progress: {
      railColor: c.elevated,
      fillColor: c.accent,
      textColorLineInner: c.bg,
    },
    Tag: {
      borderRadius: '4px',
    },
    Input: {
      color: c.inputBg,
      colorFocus: c.inputBg,
      border: `1px solid ${c.borderStrong}`,
      borderFocus: `1px solid ${c.accent}`,
      borderHover: `1px solid ${c.borderHover}`,
    },
    Tabs: {
      tabColor: 'transparent',
      tabTextColor: c.textDim,
      tabTextColorActiveLine: c.accent,
      tabTextColorHoverLine: c.accentHover,
      barColor: c.accent,
    },
    Log: {
      color: c.inputBg,
      borderColor: c.border,
    },
  }
}
