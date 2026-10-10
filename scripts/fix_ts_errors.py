# -*- coding: utf-8 -*-
"""
ModelScout —— TypeScript 类型错误批量修复脚本（阶段 5 QA）
============================================================

根因分析（两类）：
  (a) `src/composables/useProbe.ts` 第 17 行 `import { ElMessage } from 'naive-ui'`
      —— naive-ui 不导出 ElMessage（那是 Element Plus 的 API）。
      修复：改用 `createDiscreteApi(['message'])` 在模块外拿到 message 实例。

  (b) 组件中 `props.probe.store.xxx.value` 的多余 `.value`
      —— Pinia store 的 setup 返回值经 store 代理后**已经解包**，
      在组件里访问 `store.percent` 得到的就是 number 而不是 Ref<number>。
      但因为 `probe` 是 props 传入的对象、不是 setup 顶层变量，
      Vue 模板不会二次解包，所以 TS 看到的也是解包后的类型 ——
      `.value` 因此报 "Property 'value' does not exist"。
      修复：删除 store 属性后面的 `.value`。

  同时补齐 TS7006（隐式 any）参数类型标注。

用法：
    python scripts/fix_ts_errors.py
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

# ---- 让中文状态信息在任何输出环境下都打得出来 ----
# Windows 上 stdout 被重定向时（CI 日志 / 管道 / `> 文件`），Python 按系统 ANSI
# 代码页编码输出 —— en-US 环境是 cp1252，编码不了中文，第一句 print 就抛
# UnicodeEncodeError 把整个门禁搞挂（GitHub Actions 的 windows runner 实测踩到）。
# 本地中文 Windows 控制台代码页是 GBK，能编码中文，所以本地一直没暴露。
#
# 为什么无条件切、不判断 isatty：
#   控制台场景本来就是 UTF-8（Python 在 Windows 上走控制台 API），切换是空操作；
#   而 isatty() 并不可靠 —— 实测在 Git Bash 下把 stdout 重定向到文件时它仍返回
#   True，据此跳过兜底就会漏掉真正的重定向场景。
# errors="replace" 是最后一层保险：一句状态信息不值得让检查失败。
if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    sys.stderr.reconfigure(encoding="utf-8", errors="replace")


SRC = Path(__file__).resolve().parent.parent / "src"


def read(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def write(path: Path, text: str) -> None:
    path.write_text(text, encoding="utf-8")
    print(f"  [写入] {path.relative_to(SRC.parent)}  ({len(text)} 字符)")


# ---------------------------------------------------------------------------
# 修复 (a)：useProbe.ts 的 ElMessage 导入
# ---------------------------------------------------------------------------

def fix_use_probe() -> None:
    path = SRC / "composables" / "useProbe.ts"
    text = read(path)
    original = text

    # 1) 替换 import 行
    text = text.replace(
        "import { ElMessage } from 'naive-ui'",
        "import { createDiscreteApi } from 'naive-ui'",
    )

    # 2) 在 import 块之后注入 message 实例
    anchor = "import type { ModelBrief, PingResult } from '@/types/contracts'"
    if anchor in text and "const { message } = createDiscreteApi" not in text:
        injection = (
            anchor
            + "\n\n"
            + "/* ---------------------------------------------------------------------------\n"
            + " * 组件外可用的消息提示实例\n"
            + " * ------------------------------------------------------------------------- */\n\n"
            + "/**\n"
            + " * 【为什么用 createDiscreteApi 而不是 useMessage】\n"
            + " *\n"
            + " * Naive UI 的 `useMessage()` 只能在「NMessageProvider 的组件树内部」调用，\n"
            + " * 且必须在 setup 期间执行。而 useProbe() 是一个可以在任意时机被调用的\n"
            + " * 组合式函数，并且它会脱离组件上下文（例如事件回调、异步流程）弹出提示。\n"
            + " *\n"
            + " * `createDiscreteApi(['message'])` 会在首次调用时创建一个脱离组件树的\n"
            + " * 独立渲染上下文，正是官方为「在组件外调用 message」提供的方案。\n"
            + " * 代价是它不受 NConfigProvider 的主题影响 —— 因此我们在构造时显式\n"
            + " * 传入与 App.vue 一致的暗色主题，保证视觉统一。\n"
            + " */\n"
            + "const { message } = createDiscreteApi(['message'], {\n"
            + "  configProviderProps: {\n"
            + "    theme: darkTheme,\n"
            + "  },\n"
            + "})"
        )
        text = text.replace(anchor, injection)

    # 3) 补 darkTheme 导入
    if "darkTheme" in text and "import { darkTheme" not in text and "darkTheme," not in text.split("\n")[0:40].__str__():
        text = text.replace(
            "import { createDiscreteApi } from 'naive-ui'",
            "import { createDiscreteApi, darkTheme } from 'naive-ui'",
        )

    # 4) ElMessage.xxx -> message.xxx
    text = re.sub(r"\bElMessage\.", "message.", text)

    if text == original:
        print("  [跳过] useProbe.ts 无需修改")
        return
    write(path, text)


# ---------------------------------------------------------------------------
# 修复 (b)：删除 store.* 后面多余的 .value
# ---------------------------------------------------------------------------

# 这些是 store 上「已解包」的属性名 —— 后面不应再出现 .value
STORE_UNWRAPPED_PROPS = [
    "status",
    "runId",
    "hasRun",
    "total",
    "completed",
    "percent",
    "models",
    "selectedIds",
    "logs",
    "globalErrors",
    "startedAt",
    "finishedAt",
    "isRunning",
    "elapsedMs",
    "selectedModels",
    "errorSummary",
    "capabilityStats",
]

# 同时也要处理 store 上的动作调用（.resetRun() 等）—— 动作后面本就没有 .value


def strip_store_value(text: str) -> tuple[str, int]:
    """删除 `store.<prop>.value` 中的 `.value`（仅针对上面列出的已解包属性）。"""
    count = 0
    for prop in STORE_UNWRAPPED_PROPS:
        # 匹配 .store.<prop>.value  （store 后面可以跟任意前缀，如 probe.store）
        pattern = re.compile(r"(\.store\.%s)\.value\b" % re.escape(prop))
        text, n = pattern.subn(r"\1", text)
        count += n
        # 也匹配解构后直接使用的情况：store.<prop>.value 已覆盖
    return text, count


def fix_component(name: str) -> None:
    path = SRC / "components" / name
    text = read(path)
    original = text

    text, count = strip_store_value(text)

    if text == original:
        print(f"  [跳过] {name} 无需修改")
        return

    print(f"  [修复] {name}：删除 {count} 处多余的 .value")
    write(path, text)


# ---------------------------------------------------------------------------
# 修复 TS7006：隐式 any 参数
# ---------------------------------------------------------------------------

# 每个文件的定点修复：用 (旧片段, 新片段) 精确替换
TYPED_PARAM_FIXES: dict[str, list[tuple[str, str]]] = {
    "ConfigPreview.vue": [
        (
            "    context.value.models.filter(\n      (m) => m.contextLimit === 0 || m.maxOutputLimit === 0,\n    ).length,",
            "    context.value.models.filter(\n      (m: ModelInfo) => m.contextLimit === 0 || m.maxOutputLimit === 0,\n    ).length,",
        ),
    ],
    "ModelTable.vue": [
        (
            "  return list.filter((model) => {",
            "  return list.filter((model: ModelInfo) => {",
        ),
        (
            "  const ids = filteredModels.value.map((m) => m.id)",
            "  const ids = filteredModels.value.map((m: ModelInfo) => m.id)",
        ),
        (
            "  const ids = new Set(filteredModels.value.map((m) => m.id))",
            "  const ids = new Set(filteredModels.value.map((m: ModelInfo) => m.id))",
        ),
        (
            "    (id) => !ids.has(id),\n  )",
            "    (id: string) => !ids.has(id),\n  )",
        ),
    ],
    "ProbeControl.vue": [
        (
            "probe.store.errorSummary.reduce((sum, e) => sum + e.count, 0)",
            "probe.store.errorSummary.reduce((sum: number, e: { count: number }) => sum + e.count, 0)",
        ),
    ],
    "ProgressPanel.vue": [
        (
            "  const done = new Set(modelRows.value.map((m) => m.id))",
            "  const done = new Set(modelRows.value.map((m: ModelInfo) => m.id))",
        ),
    ],
}


def fix_typed_params() -> None:
    for name, fixes in TYPED_PARAM_FIXES.items():
        path = SRC / "components" / name
        text = read(path)
        changed = 0
        for old, new in fixes:
            if old in text:
                text = text.replace(old, new, 1)
                changed += 1
            else:
                print(f"  [警告] {name}：未找到片段 -> {old[:60]!r}")
        if changed:
            write(path, text)
            print(f"  [修复] {name}：补齐 {changed} 处隐式 any 类型标注")
        else:
            print(f"  [跳过] {name} 无类型标注需补齐")


# ---------------------------------------------------------------------------
# 确保组件补齐 ModelInfo 类型导入
# ---------------------------------------------------------------------------

NEEDED_IMPORT = "import type { ModelInfo } from '@/types/contracts'"


def ensure_model_info_import(name: str) -> None:
    path = SRC / "components" / name
    text = read(path)
    if "ModelInfo" not in text:
        return
    # 已经导入过（可能与其他类型一起）
    if re.search(r"import type \{[^}]*\bModelInfo\b[^}]*\} from '@/types/contracts'", text):
        return

    # 找一个已有的 contracts 类型导入行，把 ModelInfo 合并进去
    m = re.search(r"import type \{([^}]*)\} from '@/types/contracts'", text)
    if m:
        existing = m.group(1).strip()
        merged = f"import type {{ {existing}, ModelInfo }} from '@/types/contracts'"
        text = text[: m.start()] + merged + text[m.end() :]
        write(path, text)
        print(f"  [修复] {name}：合并 ModelInfo 到已有类型导入")
        return

    # 否则新加一行：插在「最后一个完整 import 语句」之后。
    # ⚠️ 不能用「最后一个以 import 开头的行」—— 多行 import 的续行会误导。
    # 做法：逐行扫描，维护「当前是否处于多行 import 中」的状态。
    lines = text.split("\n")
    insert_at = -1
    in_multiline = False
    for i, line in enumerate(lines):
        stripped = line.strip()
        if in_multiline:
            if stripped.endswith("}") or stripped.endswith("'") or stripped.endswith('"'):
                in_multiline = False
                insert_at = i + 1
            continue
        if stripped.startswith("import "):
            if stripped.endswith("'") or stripped.endswith('"'):
                insert_at = i + 1
            else:
                # 多行 import 的开头（如 `import {`）
                in_multiline = True
    if insert_at > 0:
        # 跳过紧随其后的空行，保持代码整洁
        while insert_at < len(lines) and lines[insert_at].strip() == "":
            insert_at += 1
        lines.insert(insert_at, NEEDED_IMPORT)
        write(path, "\n".join(lines))
        print(f"  [修复] {name}：新增 ModelInfo 类型导入")


def main() -> int:
    print("=" * 70)
    print("ModelScout —— TypeScript 类型错误批量修复")
    print("=" * 70)

    print("\n[1/4] 修复 useProbe.ts 的 ElMessage 导入")
    fix_use_probe()

    print("\n[2/4] 删除 store 属性后多余的 .value")
    for name in (
        "AppHeader.vue",
        "ConfigPreview.vue",
        "ConnectionForm.vue",
        "ModelTable.vue",
        "ProbeControl.vue",
        "ProgressPanel.vue",
    ):
        fix_component(name)

    print("\n[3/4] 补齐隐式 any 参数类型标注")
    fix_typed_params()

    print("\n[4/4] 确保 ModelInfo 类型导入存在")
    for name in ("ConfigPreview.vue", "ModelTable.vue", "ProbeControl.vue", "ProgressPanel.vue"):
        ensure_model_info_import(name)

    print("\n完成。请运行 `npx vue-tsc --noEmit -p tsconfig.app.json` 验证。")
    return 0


if __name__ == "__main__":
    sys.exit(main())
