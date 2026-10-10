# -*- coding: utf-8 -*-
"""验证 max_tokens 修复后的正则行为（对齐 Rust 源码）。"""

import re
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


NUM = r"(\d{1,3}(?:[,_]\d{3})+|\d+(?:\.\d+)?)\s*([kKmM])?"

OUTPUT_PATTERNS = [
    r"(?i)max[_ ]?tokens?[^0-9]{0,60}?(?:at\s+most|to|<=|≤|of|is|:)\s*" + NUM,
    r"(?i)(?:maximum|max|limit)[^.\n]{0,40}?max[_ ]?tokens?[^0-9]{0,20}?" + NUM,
    r"(?i)at\s+most\s+" + NUM + r"(?:\s*[a-zA-Z]*)?\s*(?:completion|output)\s+tokens?",
    r"(?i)(?:maximum|max)[^.\n]{0,40}?(?:output|completion)[^.\n]{0,40}?" + NUM,
    r"(?i)(?:less\s+than\s+or\s+equal\s+to|not\s+greater\s+than|≤)\s*" + NUM,
    r"(?i)exceed[s]?[^.\n]{0,60}?" + NUM,
    r"(?i)(?:maximum|max|limit)[^.\n]{0,30}?(?:allowed|permitted|supported)?[^0-9]{0,20}?(?:is|of|=|:)\s*" + NUM,
    r"(?i)(?:greater|larger|more)\s+than[^.\n]{0,40}?(?:maximum|max|limit)[^.\n]{0,40}?" + NUM,
    r"最大输出(?:长度)?" + NUM,
    r"(?:输出|补全)[^0-9]{0,10}(?:上限|最大值)[^0-9]{0,10}?" + NUM,
    r"(?:最大|上限|限制)[^0-9]{0,12}?(?:为|是|不能超过|不超过)?\s*" + NUM,
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

TOO_LARGE_HINT = re.compile(
    r"(?i)("
    r"too\s+large|too\s+big|too\s+high|exceed|greater\s+than"
    r"|above\s+the\s+maximum|out\s+of\s+range|invalid\s+max"
    r"|maximum\s+(?:context|model|allowed|supported)"
    r"|max(?:imum)?\s+(?:context|model)\s+(?:length|window|size)"
    r"|context\s+length"
    r"|at\s+most|less\s+than\s+or\s+equal|not\s+greater\s+than|up\s+to"
    r"|降低|过大|超出|超过|不能大于|不能超过|不支持|上限|最大"
    r")"
)

MIN_OUT, MAX_OUT = 256, 4_000_000
MIN_CTX, MAX_CTX = 512, 32_000_000


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


def parse_limits(text):
    if not text.strip():
        return {"out": None, "ctx": None}
    if not TOO_LARGE_HINT.search(text):
        return {"out": None, "ctx": None}

    out_cands = []
    for p in OUTPUT_PATTERNS:
        for m in re.finditer(p, text):
            v = parse_number(m.group(1), m.group(2))
            if v is not None and MIN_OUT <= v <= MAX_OUT:
                out_cands.append((v, m.group(0)))
    filtered = [c for c in out_cands if c[0] < 900_000]
    pool = filtered if filtered else out_cands
    out = max((v for v, _ in pool), default=None)

    ctx = None
    for p in CONTEXT_PATTERNS:
        found = None
        for m in re.finditer(p, text):
            v = parse_number(m.group(1), m.group(2))
            if v is not None and MIN_CTX <= v <= MAX_CTX:
                found = v
                break
        if found is not None:
            ctx = found
            break

    # 一致性修正
    if ctx is not None and out is not None and ctx < out:
        return {"out": None, "ctx": ctx}
    return {"out": out, "ctx": ctx}


TESTS = [
    ("parses_context_length_error",  # 期望 ctx=128000
     "This model's maximum context length is 128000 tokens. "
     "However, your messages resulted in 130000 tokens.",
     {"ctx": 128000}),
    ("parses_vllm_style_error",  # 期望 out=4096
     "max_tokens 999999 is greater than the maximum model length 4096",
     {"out": 4096}),
    ("parses_k_suffix_notation",  # 期望 ctx=131072
     "maximum context length is 128k tokens",
     {"ctx": 131072}),
    ("ignores_echoed_probe_value",  # 期望 out=16384
     "max_tokens 999999 is too large, maximum allowed is 16384",
     {"out": 16384}),
    ("parses_deepseek_style_error",
     "max_tokens is too large: 999999. This model supports at most 8192 completion tokens.",
     {"out": 8192}),
    ("parses_chinese_provider_error",
     "请求参数错误：单次最大输出长度不能超过 8192，请调整 max_tokens 后重试。",
     {"out": 8192}),
    ("parses_thousand_separators",
     "requested max_tokens exceeds the limit of 128,000 tokens",
     {"out": 128000}),
    ("rejects_absurd_numbers",
     "max_tokens is too large, limit is 7 tokens",
     {"out": None}),
    ("rejects_unrelated_error_text",
     "unsupported parameter: temperature. Please remove it.",
     {"out": None, "ctx": None}),
    ("resolves_contradiction_between_context_and_output",
     "max_tokens is too large; maximum context length is 4096 and set max_tokens to at most 8192",
     {"out": None, "ctx": 4096}),
]

print("=" * 74)
fails = 0
for name, text, expect in TESTS:
    r = parse_limits(text)
    ok = True
    for k, want in expect.items():
        got = r.get(k)
        if got != want:
            ok = False
            print(f"✗ {name}: {k} 期望 {want}，实际 {got}")
    if ok:
        print(f"✓ {name}  ->  {r}")
    else:
        fails += 1
        print(f"     文本: {text[:80]}")
print("=" * 74)
print(f"结果：{'全部通过' if fails == 0 else f'{fails} 个失败'}")
