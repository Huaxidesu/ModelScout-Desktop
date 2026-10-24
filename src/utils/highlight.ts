/**
 * 极简语法高亮：JSON / JSONC / TOML / YAML 通吃的一套 token 着色。
 *
 * 不引入 CodeMirror/Monaco 的原因与 ConfigPreview 相同：需求是只读
 * 预览，手写规则只需几十行且可控。规则按「常见配置文件形态」覆盖：
 *   - 注释：`//`、`#`
 *   - 字符串：双引号包裹
 *   - 键名：`"key":`（JSON 风格）与行首裸键 `key:`（YAML/TOML 风格）
 *   - 数字、布尔、null
 */

/** 转义 HTML，防止内容里的 < > & 破坏结构 */
function escapeHtml(line: string): string {
  return line.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
}

/** 把一行配置渲染成带 <span class="tok-*"> 的 HTML（输入须为原始文本行） */
export function highlightLine(line: string): string {
  let escaped = escapeHtml(line)

  // 注释整体着色，不再处理内部 token
  const commentMatch = escaped.match(/^(\s*)(\/\/|#)(.*)$/)
  if (commentMatch) {
    return `${commentMatch[1]}<span class="tok-comment">${commentMatch[2]}${commentMatch[3]}</span>`
  }

  // 字符串（双引号）
  escaped = escaped.replace(/(&quot;|")([^"]*)(&quot;|")/g, '<span class="tok-string">"$2"</span>')
  // JSON 风格键名："key" 后紧跟冒号
  escaped = escaped.replace(
    /<span class="tok-string">"([^"]+)"<\/span>(\s*:)/g,
    '<span class="tok-key">"$1"</span>$2',
  )
  // YAML/TOML 风格键名：行首裸键 `key:`（允许 - 列表前缀与缩进）
  escaped = escaped.replace(
    /^(\s*(?:-\s+)?)([A-Za-z_][\w.-]*)(\s*:)/,
    '$1<span class="tok-key">$2</span>$3',
  )
  // 数字
  escaped = escaped.replace(
    /(\s|,|:)(\s*)(-?\d+(?:\.\d+)?)(\s*)(,|$|\}|\])/g,
    '$1$2<span class="tok-number">$3</span>$4$5',
  )
  // 布尔与 null
  escaped = escaped.replace(/\b(true|false|null)\b/g, '<span class="tok-bool">$1</span>')

  return escaped
}
