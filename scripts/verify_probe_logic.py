# -*- coding: utf-8 -*-
"""
模拟 Rust 侧 inspect_response 与 reasoning_by_name 的修复后行为，
在编译前快速验证 2 个 probe.rs 测试能否通过。
"""

import json
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


# ---------------- reasoning_by_name ----------------

REASONING_NAME_PATTERNS = [
    r"(?i)(^|[/\-_:])r1($|[/\-_:.])",
    r"(?i)deepseek[-_]?reasoner",
    r"(?i)deepseek[-_]?r1",
    r"(?i)^o1($|[/\-_:.])",
    r"(?i)^o3($|[/\-_:.])",
    r"(?i)^o4[-_]?mini($|[/\-_:.])",
    r"(?i)^(?:openai|azure|msft|microsoft)[/\-_]+o(?:1|3|4[-_]?mini)($|[/\-_:.])",
    r"(?i)[\-_/]thinking([\-_/]|$)",
    r"(?i)[\-_/]reason(er|ing)([\-_/]|$)",
    r"(?i)[\-_/]cot([\-_/]|$)",
    r"(?i)^qwq",
    r"(?i)[\-_/]qwen3[\-_]?(think|thinking)",
    r"(?i)(^|[\-_/])glm[\-_]?z1",
    r"(?i)[\-_/]marco[\-_]?o1",
    r"(?i)[\-_/]exaone[\-_]?deep",
]

REGEXES = [re.compile(p) for p in REASONING_NAME_PATTERNS]


def reasoning_by_name(model_id):
    for r in REGEXES:
        if r.search(model_id):
            return f"match /{r.pattern}/"
    return None


# ---------------- inspect_response ----------------


def inspect_response(text):
    try:
        value = json.loads(text)
    except Exception:
        low = text.lower()
        return ('"tool_calls"' in low and '"tool_calls":null' not in low,
                "reasoning_content" in low)

    has_tc = False
    has_rz = False
    choices = value.get("choices")
    if isinstance(choices, list):
        for choice in choices:
            msg = choice.get("message") or choice.get("delta")
            if not isinstance(msg, dict):
                continue
            tc = msg.get("tool_calls")
            if isinstance(tc, list) and len(tc) > 0:
                has_tc = True
            if msg.get("function_call") is not None:
                has_tc = True
            rc = msg.get("reasoning_content")
            if isinstance(rc, str) and rc.strip():
                has_rz = True
            rr = msg.get("reasoning")
            if isinstance(rr, str) and rr.strip():
                has_rz = True

    if not has_rz:
        rc = value.get("reasoning_content")
        if isinstance(rc, str) and rc.strip():
            has_rz = True

    return (has_tc, has_rz)


print("=" * 74)
print("reasoning_by_name 测试")
print("=" * 74)
should_match = ["deepseek-r1", "deepseek-reasoner", "o1-mini", "o3-mini",
                "qwq-32b", "qwen3-235b-thinking", "glm-z1-air", "model-thinking-7b"]
should_not = ["gpt-4o", "gpt-4o-mini", "vision-o1-like", "llama-3.3-70b", "qwen2.5-72b"]

fails = 0
for m in should_match:
    r = reasoning_by_name(m)
    if r:
        print(f"  ✓ {m:26} 命中")
    else:
        print(f"  ✗ {m:26} 应命中但未命中")
        fails += 1

for m in should_not:
    r = reasoning_by_name(m)
    if r is None:
        print(f"  ✓ {m:26} 正确排除")
    else:
        print(f"  ✗ {m:26} 不应命中却命中 -> {r}")
        fails += 1

print()
print("=" * 74)
print("inspect_response 测试")
print("=" * 74)

with_tools = '{"choices":[{"message":{"role":"assistant","tool_calls":[{"id":"1","type":"function","function":{"name":"modelscout_ping","arguments":"{}"}}]}}]}'
empty_tools = '{"choices":[{"message":{"role":"assistant","content":"hi","tool_calls":null}}]}'
plain = '{"choices":[{"message":{"role":"assistant","content":"hello"}}]}'
with_reasoning = '{"choices":[{"message":{"role":"assistant","content":"","reasoning_content":"让我想想…"}}]}'
empty_reasoning = '{"choices":[{"message":{"role":"assistant","content":"hi","reasoning_content":""}}]}'

cases = [
    ("with_tools   .0", inspect_response(with_tools)[0], True),
    ("empty_tools  .0", inspect_response(empty_tools)[0], False),
    ("plain == (F,F)", inspect_response(plain), (False, False)),
    ("with_reason  .1", inspect_response(with_reasoning)[1], True),
    ("empty_reason .1", inspect_response(empty_reasoning)[1], False),
]

for name, got, want in cases:
    if got == want:
        print(f"  ✓ {name:18} = {got}")
    else:
        print(f"  ✗ {name:18} 期望 {want}，实际 {got}")
        fails += 1

print()
print("=" * 74)
print(f"结果：{'全部通过' if fails == 0 else f'{fails} 个失败'}")
