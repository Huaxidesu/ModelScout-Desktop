# -*- coding: utf-8 -*-
"""
临时诊断脚本：定位 max_tokens.rs 的 6 个失败测试的根因。

用 Python re 模拟 Rust regex（两者语法在本题所用子集内兼容），
不依赖 Rust 编译，快速验证正则假设。
"""

import re
import sys

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


# 与 Rust 源码完全一致的 NUM 片段
NUM = r"(\d{1,3}(?:[,_]\d{3})+|\d+(?:\.\d+)?)\s*([kKmM])?"

# 与 Rust 源码逐条对应的模式
OUTPUT_PATTERNS = [
    r"(?i)max[_ ]?tokens?[^0-9]{0,60}?(?:at\s+most|to|<=|≤|of|is|:)\s*" + NUM,
    r"(?i)(?:maximum|max|limit)[^.\n]{0,40}?max[_ ]?tokens?[^0-9]{0,20}?" + NUM,
    r"(?i)at\s+most\s+" + NUM + r"(?:\s*[a-zA-Z]*)?\s*(?:completion|output)\s+tokens?",
    r"(?i)(?:maximum|max)[^.\n]{0,40}?(?:output|completion)[^.\n]{0,40}?" + NUM,
    r"(?i)(?:less\s+than\s+or\s+equal\s+to|not\s+greater\s+than|≤)\s*" + NUM,
    r"(?i)exceed[s]?[^.\n]{0,60}?" + NUM,
    r"最大输出(?:长度)?[^0-9]{0,20}?" + NUM,
    r"(?:输出|补全)[^0-9]{0,10}(?:上限|最大值)[^0-9]{0,10}?" + NUM,
]

CONTEXT_PATTERNS = [
    r"(?i)(?:maximum|max)\s+context\s+(?:length|window|size)[^0-9]{0,40}?" + NUM,
    r"(?i)context\s+(?:length|window|size)[^0-9]{0,40}?" + NUM,
    r"(?i)context[_ ]?length[_ ]?exceeded[^0-9]{0,80}?" + NUM,
    r"(?i)(?:maximum|max)\s+model\s+length[^0-9]{0,40}?" + NUM,
    r"(?i)messages?[^.\n]{0,80}?(?:maximum|max|limit)[^0-9]{0,20}?" + NUM,
    r"上下文(?:长度|窗口|上限)[^0-9]{0,20}?" + NUM,
    r"最大上下文[^0-9]{0,20}?" + NUM,
]

TOO_LARGE_HINT = (
    r"(?i)(too\s+large|too\s+big|too\s+high|exceed|greater\s+than|above\s+the\s+maximum"
    r"|out\s+of\s+range|invalid\s+max|降低|过大|超出|超过|不能大于|不支持)"
)


def parse_number(digits, suffix):
    cleaned = digits.replace(",", "").replace("_", "")
    try:
        base = float(cleaned)
    except ValueError:
        return None
    if base <= 0:
        return None
    mult = {"k": 1024.0, "m": 1048576.0}.get((suffix or "").lower(), 1.0)
    return int(round(base * mult))


CASES = {
    "parses_context_length_error":
        "This model's maximum context length is 128000 tokens. "
        "However, your messages resulted in 130000 tokens.",
    "parses_vllm_style_error":
        "max_tokens 999999 is greater than the maximum model length 4096",
    "parses_k_suffix_notation":
        "maximum context length is 128k tokens",
    "ignores_echoed_probe_value":
        "max_tokens 999999 is too large, maximum allowed is 16384",
    "parses_deepseek_style_error (PASS基线)":
        "max_tokens is too large: 999999. This model supports at most 8192 completion tokens.",
}


def main():
    for name, text in CASES.items():
        print("=" * 74)
        print(f"[{name}]")
        print(f"  文本: {text[:76]}")

        hint = bool(re.search(TOO_LARGE_HINT, text))
        print(f"  TOO_LARGE_HINT 命中: {hint}")

        print("  --- 输出模式 ---")
        any_out = False
        for i, p in enumerate(OUTPUT_PATTERNS):
            for m in re.finditer(p, text):
                any_out = True
                # 复现 Rust 的取值逻辑
                digits, suffix = m.group(1), m.group(2)
                val = parse_number(digits, suffix)
                print(f"    模式{i}: groups=({digits!r},{suffix!r}) -> 值={val}"
                      f"  match={m.group(0)[:52]!r}")
        if not any_out:
            print("    (无命中)")

        print("  --- 上下文模式 ---")
        any_ctx = False
        for i, p in enumerate(CONTEXT_PATTERNS):
            for m in re.finditer(p, text):
                any_ctx = True
                digits, suffix = m.group(1), m.group(2)
                val = parse_number(digits, suffix)
                print(f"    模式{i}: groups=({digits!r},{suffix!r}) -> 值={val}"
                      f"  match={m.group(0)[:52]!r}")
        if not any_ctx:
            print("    (无命中)")
        print()


if __name__ == "__main__":
    main()
