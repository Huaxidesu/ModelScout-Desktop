# -*- coding: utf-8 -*-
"""
ModelScout —— JSON 语法门禁（阶段 5 QA 系统性检查）
=====================================================

【为什么需要这个脚本】
阶段 5 首次运行 `cargo check` 时，Tauri 构建脚本报：

    failed to parse JSON: expected value at line 1 column 1

根因是 `src-tauri/capabilities/default.json` 顶部写了 `/** ... */` 注释块。
JSON 标准**不允许注释**，而 Tauri 用 serde_json 做严格解析。

同类问题还有 `src-tauri/resources/model_params.json`（用了 `#` 注释块），
它不会导致编译失败，而是让运行期的外部字典加载**静默退化为内嵌字典** ——
这种隐性故障靠肉眼几乎不可能发现。

因此把「JSON 文件必须是严格合法 JSON」做成脚本门禁，
比留在交付报告的「注意事项」里可靠得多。

【豁免名单】
TypeScript 的 tsconfig 系列是 **JSONC** 格式，官方允许注释，
因此显式跳过（不是漏检，是有意为之）。

【用法】
    python scripts/check_json.py

退出码：0 = 全部合法；1 = 存在非法 JSON
"""

from __future__ import annotations

import json
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


# 项目根目录 = 本脚本的上一级
ROOT = Path(__file__).resolve().parent.parent

# 不扫描的目录（依赖、构建产物、版本库内部数据）
SKIP_DIRS = {"node_modules", "target", "target_test", "dist", ".git", "__pycache__"}

# 允许注释的 JSONC 文件（TypeScript 官方支持，非缺陷）
JSONC_WHITELIST = {
    "tsconfig.json",
    "tsconfig.app.json",
    "tsconfig.node.json",
    "tsconfig.base.json",
}


def is_skipped(path: Path) -> bool:
    """路径中任一目录命中 SKIP_DIRS 就跳过。"""
    return any(part in SKIP_DIRS for part in path.parts)


def main() -> int:
    print("=" * 72)
    print("ModelScout —— JSON 语法门禁")
    print("=" * 72)

    checked = 0
    skipped_jsonc: list[str] = []
    failures: list[tuple[str, str]] = []

    for path in sorted(ROOT.rglob("*.json")):
        if is_skipped(path):
            continue

        rel = path.relative_to(ROOT).as_posix()
        name = path.name

        # JSONC 白名单：只记录，不判定为错误
        if name in JSONC_WHITELIST:
            skipped_jsonc.append(rel)
            continue

        try:
            raw = path.read_text(encoding="utf-8")
            json.loads(raw)
            checked += 1
            print(f"  ✓ {rel}")
        except json.JSONDecodeError as err:
            failures.append((rel, f"第 {err.lineno} 行第 {err.colno} 列：{err.msg}"))
            print(f"  ✗ {rel}  ->  {err.lineno}:{err.colno} {err.msg}")
        except Exception as err:  # 编码错误等
            failures.append((rel, str(err)[:120]))
            print(f"  ✗ {rel}  ->  {err}")

    print()
    print(f"严格校验通过：{checked} 个")

    if skipped_jsonc:
        print(f"JSONC 豁免（TypeScript 允许注释）：{len(skipped_jsonc)} 个")
        for rel in skipped_jsonc:
            print(f"    - {rel}")

    if failures:
        print()
        print(f"✗ 发现 {len(failures)} 个非法 JSON 文件：")
        for rel, err in failures:
            print(f"    - {rel}")
            print(f"        {err}")
        print()
        print("修复建议：删除文件中的注释（JSON 不支持），")
        print("        或把说明文字改写进字符串字段 / 同目录 README.md。")
        return 1

    print()
    print("✓ JSON 语法门禁全部通过")
    return 0


if __name__ == "__main__":
    sys.exit(main())
