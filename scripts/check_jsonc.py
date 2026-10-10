"""JSONC 检查器：定位语法错误位置。

opencode 的配置是 JSONC（允许注释与尾逗号）。标准 json 模块直接解析会
因为注释/尾逗号报错，掩盖真正的问题。这个脚本先把这两样东西去掉，
再把错误位置映射回原文件行号。
"""

import json
import sys

# ---- 保证中文状态信息在任何输出环境下都能正确输出 ----
# Windows 上 stdout 被重定向时（CI 日志 / 管道 / `> 文件`），Python 按系统 ANSI
# 代码页编码输出 —— en-US 环境是 cp1252，编码不了中文，第一条 print 语句即抛
# UnicodeEncodeError 导致整个检查失败（已在 GitHub Actions 的 windows runner 上复现）。
# 本地中文 Windows 控制台代码页是 GBK，能编码中文，所以本地一直没暴露。
#
# 为什么无条件切、不判断 isatty：
#   控制台场景本来就是 UTF-8（Python 在 Windows 上走控制台 API），切换是空操作；
#   而 isatty() 并不可靠 —— 实测在 Git Bash 下把 stdout 重定向到文件时它仍返回
#   True，据此跳过回退处理就会漏掉真正的重定向场景。
# errors="replace" 是最后一层保险：一句状态信息不值得让检查失败。
if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    sys.stderr.reconfigure(encoding="utf-8", errors="replace")



def strip_jsonc(text: str) -> str:
    """去掉注释与尾逗号，保持行号不变（用空格填充被删掉的内容）。

    注释与尾逗号**在同一个状态机里处理**，而不是先剥注释再用正则删逗号。
    原因：正则 `,(\\s*[}\\]])` 分不清字符串内外 ——
    遇到 `"foo, }"` 这种值就会把字符串里的逗号删掉，把一份本来正确的
    配置「修」成错的。这类工具出错比不工作更糟，所以宁可多写几行。
    """
    out = []
    i = 0
    n = len(text)
    in_string = False
    escaped = False

    while i < n:
        ch = text[i]

        # ---- 字符串内部：原样透传，只跟踪转义与结束引号 ----
        if in_string:
            out.append(ch)
            if escaped:
                escaped = False
            elif ch == "\\":
                escaped = True
            elif ch == '"':
                in_string = False
            i += 1
            continue

        if ch == '"':
            in_string = True
            out.append(ch)
            i += 1
            continue

        # ---- 行注释：整行内容替换成空格 ----
        if ch == "/" and i + 1 < n and text[i + 1] == "/":
            while i < n and text[i] != "\n":
                out.append(" ")
                i += 1
            continue

        # ---- 块注释：保留其中的换行以维持行号 ----
        if ch == "/" and i + 1 < n and text[i + 1] == "*":
            while i < n and not (text[i] == "*" and i + 1 < n and text[i + 1] == "/"):
                out.append("\n" if text[i] == "\n" else " ")
                i += 1
            out.append("  ")
            i += 2
            continue

        # ---- 尾逗号：逗号后面（跳过空白）紧跟 } 或 ] 时丢弃 ----
        if ch == ",":
            j = i + 1
            while j < n and text[j] in " \t\r\n":
                j += 1
            if j < n and text[j] in "}]":
                out.append(" ")
                i += 1
                continue

        out.append(ch)
        i += 1

    return "".join(out)


def main(path: str) -> int:
    raw = open(path, encoding="utf-8").read()
    cleaned = strip_jsonc(raw)

    try:
        json.loads(cleaned)
    except json.JSONDecodeError as exc:
        print(f"❌ 语法错误：{exc.msg}")
        print(f"   位置：第 {exc.lineno} 行，第 {exc.colno} 列（字符偏移 {exc.pos}）")
        lines = raw.splitlines()
        lo = max(0, exc.lineno - 4)
        hi = min(len(lines), exc.lineno + 3)
        print("   ---- 上下文 ----")
        for idx in range(lo, hi):
            marker = ">>" if idx == exc.lineno - 1 else "  "
            print(f"   {marker} {idx + 1:4d} | {lines[idx]}")
        return 1

    data = json.loads(cleaned)
    print(f"✅ 语法正确（去注释与尾逗号后共 {len(cleaned.splitlines())} 行）")
    print(f"   顶层键：{list(data.keys())}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1]))
