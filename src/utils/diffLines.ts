/**
 * 行级 diff（LCS 算法）：一键填写的左右对比视图使用。
 *
 * 产出「对齐的行对」序列：每一行对要么两侧相同，要么仅左侧有（删除）、
 * 仅右侧有（新增）。渲染时左右同行对齐（VSCode 并排对比的观感），
 * 一行 flex 容器里同时放左右两个单元格即可天然对齐。
 *
 * 为什么不用 Myers diff：配置文件通常几十到几百行，O(n·m) 的 LCS
 * 完全够用，实现与测试都更直观。
 */

/** 一行对齐结果 */
export interface DiffRow {
  /** 'same' 两侧相同；'del' 仅左侧有（原文件里将被改掉）；'add' 仅右侧有（新内容） */
  type: 'same' | 'del' | 'add'
  /** 左侧行号（1 起）；del/same 时有效 */
  leftNo: number | null
  /** 左侧内容；add 时为 null */
  left: string | null
  /** 右侧行号（1 起）；add/same 时有效 */
  rightNo: number | null
  /** 右侧内容；del 时为 null */
  right: string | null
}

/**
 * 计算两个文本的行级 diff。
 *
 * @param leftText  原文本（按 \n 拆行；结尾的空行不参与比较）
 * @param rightText 新文本
 */
export function diffLines(leftText: string, rightText: string): DiffRow[] {
  const left = splitLines(leftText)
  const right = splitLines(rightText)

  // ---- LCS 动态规划表 ----
  // lcs[i][j] = left[i..] 与 right[j..] 的最长公共子序列长度
  const n = left.length
  const m = right.length
  const lcs: number[][] = Array.from({ length: n + 1 }, () => new Array<number>(m + 1).fill(0))
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      lcs[i][j] = left[i] === right[j] ? lcs[i + 1][j + 1] + 1 : Math.max(lcs[i + 1][j], lcs[i][j + 1])
    }
  }

  // ---- 回溯产出对齐行 ----
  const rows: DiffRow[] = []
  let i = 0
  let j = 0
  let leftNo = 0
  let rightNo = 0

  while (i < n && j < m) {
    if (left[i] === right[j]) {
      leftNo++
      rightNo++
      rows.push({ type: 'same', leftNo, left: left[i], rightNo, right: right[j] })
      i++
      j++
    } else if (lcs[i + 1][j] >= lcs[i][j + 1]) {
      // 先消耗左侧：删除行（改动展示为「先删后加」，与常见 diff 工具一致）
      leftNo++
      rows.push({ type: 'del', leftNo, left: left[i], rightNo: null, right: null })
      i++
    } else {
      rightNo++
      rows.push({ type: 'add', leftNo: null, left: null, rightNo, right: right[j] })
      j++
    }
  }
  while (i < n) {
    leftNo++
    rows.push({ type: 'del', leftNo, left: left[i], rightNo: null, right: null })
    i++
  }
  while (j < m) {
    rightNo++
    rows.push({ type: 'add', leftNo: null, left: null, rightNo, right: right[j] })
    j++
  }

  return rows
}

/** 按换行拆行；\r\n 归一为无 \r，结尾空行丢弃 */
function splitLines(text: string): string[] {
  if (text === '') return []
  const lines = text.replace(/\r\n/g, '\n').split('\n')
  // 末尾换行产生的空尾元素不参与比较
  if (lines.length > 0 && lines[lines.length - 1] === '') lines.pop()
  return lines
}
