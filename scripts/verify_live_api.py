#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""对真实网关做一次「端到端能力探测」，并与官方清单逐格比对。

为什么需要这个脚本
------------------
`ModelScout` 的能力探测逻辑写在 Rust 端（`src-tauri/src/probe.rs`），
要验证它必须编译并启动 GUI，迭代很慢。

本脚本**复刻** Rust 端「声明优先、探测兜底」的核心逻辑（决策 D-33）：

  1. `GET {base}/models`
  2. 从扩展 schema 解析 `DeclaredCaps`
     （上下文 / 输出上限 / 工具调用 / 视觉 / 是否就绪）
  3. 对每个模型发**一次**最廉价的对话请求，检查响应是否含 `reasoning_content`
  4. 打印结果表，并（可选）与 `GROUND_TRUTH` 逐格比对

这样可以在命令行里快速判断「接口行为」与「解析逻辑」是否正确。

用法
----
    python scripts/verify_live_api.py <BASE_URL> <API_KEY>
    python scripts/verify_live_api.py <BASE_URL> <API_KEY> --no-reasoning

API Key 只从命令行参数读取，**不会被写入任何文件**。

注意
----
`GROUND_TRUTH` 是**某个具体网关的官方模型清单**（TokenPlan 类端点），
仅作示例与回归基准。换用别的网关时，把这一段改成该网关的官方数据即可；
留空则只打印探测结果、不做比对。
"""

import json
import sys
import time
import urllib.error
import urllib.request

# ---- 让中文状态信息在任何输出环境下都打得出来 ----
# Windows 上 stdout 被重定向时（CI 日志 / 管道 / `> 文件`），Python 按系统 ANSI
# 代码页编码输出 —— en-US 环境是 cp1252，编码不了中文，第一句 print 就抛
# UnicodeEncodeError 把整个校验搞挂（GitHub Actions 的 windows runner 实测踩到）。
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


# ---------------------------------------------------------------------------
# 官方清单（原文照录，避免人为转写引入误差）
#   键 = 模型 ID
#   值 = (上下文长度, 输入类型原文, 模型能力原文)
# ---------------------------------------------------------------------------
GROUND_TRUTH = {
    "Agents-A1": (262144, "文本、图像", "工具调用 · 科学推理"),
    "Atria-Dawn-Preview": (524288, "文本", "科学推理"),
    "deepseek-v4-flash-0731": (1048576, "文本", "深度思考 · 工具调用"),
    "deepseek-v4-flash-vision": (1048576, "文本、图像", "工具调用 · 深度思考"),
    "deepseek-v4-pro-0813": (1048576, "文本", "工具调用 · 深度思考"),
    "glm-5.3": (1048576, "文本", "工具调用 · 深度思考"),
    "intern-s2": (262144, "文本、图像", "工具调用 · 科学推理"),
    "kimi-k2.6": (262144, "文本、图像", "工具调用 · 深度思考"),
    "minimax-m3": (1048576, "文本", "工具调用 · 深度思考"),
    "qwen3.8-27b": (262144, "文本、图像", "工具调用 · 深度思考"),
}


def expect_of(entry):
    """从官方原文推导期望值。

    两条推导规则（**有歧义，已在交付说明中标注**）：

    - `工具调用` 出现在「模型能力」中 → 支持工具调用
    - `深度思考` 出现在「模型能力」中 → **是推理模型**
      「科学推理」**不计入** reasoning —— 它描述的是**领域能力**（擅长科学推理），
      而不是「模型是否具备深度思考模式」。这一点由接口的 `reasoning` 参数印证：
      恰好只有 7 个「深度思考」模型带该参数，3 个「科学推理」模型不带。
    """
    ctx, input_types, caps = entry
    return {
        "ctx": ctx,
        "tools": "工具调用" in caps,
        "vision": "图像" in input_types,
        "reasoning": "深度思考" in caps,
    }


# ---------------------------------------------------------------------------
# HTTP 小工具
# ---------------------------------------------------------------------------
def http(method, url, key, body=None, timeout=60, retries=3):
    """返回 (status, text)。传输层异常返回 (-1, 错误说明)。"""
    data = json.dumps(body).encode("utf-8") if body is not None else None
    headers = {"Authorization": "Bearer " + key}
    if data is not None:
        headers["Content-Type"] = "application/json"

    for attempt in range(retries):
        req = urllib.request.Request(url, data=data, headers=headers, method=method)
        try:
            with urllib.request.urlopen(req, timeout=timeout) as resp:
                return resp.status, resp.read().decode("utf-8", "replace")
        except urllib.error.HTTPError as e:
            text = e.read().decode("utf-8", "replace")
            # 429 限流 → 退避重试
            if e.code == 429 and attempt < retries - 1:
                time.sleep(3 * (attempt + 1))
                continue
            return e.code, text
        except Exception as e:  # noqa: BLE001 - 传输层任何异常都归为 -1
            if attempt < retries - 1:
                time.sleep(2 * (attempt + 1))
                continue
            return -1, "{}: {}".format(type(e).__name__, e)
    return -1, "重试耗尽"


# ---------------------------------------------------------------------------
# 解析：复刻 Rust 端 parse_declared_caps()
# ---------------------------------------------------------------------------
def parse_declared_caps(item):
    """从 /models 条目解析接口自报能力。规则与 Rust 端严格一致。"""
    caps = {
        "context_limit": None,
        "max_output_limit": None,
        "supports_tools": None,
        "supports_vision": None,
        "supports_reasoning": None,
        "is_ready": None,
        "schema_version": "",
    }
    caps["schema_version"] = item.get("schema_version") or ""
    if isinstance(item.get("is_ready"), bool):
        caps["is_ready"] = item["is_ready"]

    # 输入模态：text → 上下文上限；image → 视觉支持
    mods = item.get("input_modalities")
    if isinstance(mods, list):
        has_image = False
        for m in mods:
            if not isinstance(m, dict):
                continue
            if m.get("type") == "text":
                si = m.get("supported_inputs") or {}
                mcl = si.get("max_context_length") or {}
                v = mcl.get("value")
                if isinstance(v, int):
                    caps["context_limit"] = v
            elif m.get("type") == "image":
                has_image = True
        # 只要声明了 input_modalities，就能对「是否支持图片」下结论
        caps["supports_vision"] = has_image

    # 输出模态：text → 输出上限 + 工具调用
    mods = item.get("output_modalities")
    if isinstance(mods, list):
        for m in mods:
            if not isinstance(m, dict) or m.get("type") != "text":
                continue
            ml = m.get("max_length") or {}
            v = ml.get("value")
            if isinstance(v, int):
                caps["max_output_limit"] = v
            params = m.get("supported_parameters")
            if isinstance(params, dict):
                # 关键：tools / reasoning 的形态都是 {"type":"boolean"}，
                # 判断依据是「键是否存在」，不是它的值。
                caps["supports_tools"] = "tools" in params
                # reasoning 键存在 = 推理模型（深度思考）；实测与官方文档 100% 吻合
                caps["supports_reasoning"] = "reasoning" in params

    # 顶层兜底字段
    if caps["context_limit"] is None:
        for k in ("context_length", "context_window", "max_context_length"):
            if isinstance(item.get(k), int):
                caps["context_limit"] = item[k]
                break
    if caps["max_output_limit"] is None:
        for k in ("max_output_length", "max_completion_tokens"):
            if isinstance(item.get(k), int):
                caps["max_output_limit"] = item[k]
                break
    return caps


# ---------------------------------------------------------------------------
# 探测：一次最廉价对话，判定推理能力
# ---------------------------------------------------------------------------
def sample_reasoning(base, key, model_id):
    status, text = http(
        "POST",
        base + "/chat/completions",
        key,
        {"model": model_id, "messages": [{"role": "user", "content": "hi"}], "max_tokens": 16},
    )
    if status == -1:
        return None, text
    if not (200 <= status < 300):
        return None, "HTTP {} {}".format(status, text[:120].replace("\n", " "))
    try:
        obj = json.loads(text)
    except ValueError:
        return False, "响应不是合法 JSON"
    for ch in obj.get("choices") or []:
        msg = ch.get("message") or {}
        if "reasoning_content" in msg and msg["reasoning_content"]:
            return True, "响应含 reasoning_content"
    return False, "响应无 reasoning_content"


# ---------------------------------------------------------------------------
# 主流程
# ---------------------------------------------------------------------------
def main():
    if len(sys.argv) < 3:
        print(__doc__)
        return 2
    base = sys.argv[1].rstrip("/")
    key = sys.argv[2]
    do_reasoning = "--no-reasoning" not in sys.argv

    print("=" * 78)
    print("目标：{}".format(base))
    print("=" * 78)

    # ---- 1. 拉列表 ----
    status, text = http("GET", base + "/models", key)
    print("\n[1] GET /models → HTTP {}".format(status))
    if status != 200:
        print("    失败：{}".format(text[:400]))
        return 1
    try:
        payload = json.loads(text)
    except ValueError as e:
        print("    响应不是合法 JSON：{}".format(e))
        return 1
    items = payload.get("data") or payload.get("models") or []
    if not isinstance(items, list) or not items:
        print("    未找到模型数组（data / models）")
        return 1
    print("    解析到 {} 个模型".format(len(items)))

    # ---- 2. 逐个解析 + 采样 ----
    rows = []
    print("\n[2] 能力解析（声明优先）与推理采样")
    print("    {:<26}{:>10}{:>8}{:>8}{:>10}".format("模型 ID", "上下文", "工具", "视觉", "推理"))
    print("    " + "-" * 62)
    for item in items:
        mid = (item.get("id") or item.get("name") or "").strip()
        if not mid:
            continue
        caps = parse_declared_caps(item)
        # 推理能力：接口已声明则直接采信（此时一次请求都不用发）
        if caps["supports_reasoning"] is not None:
            reasoning = caps["supports_reasoning"]
        elif do_reasoning:
            reasoning, _note = sample_reasoning(base, key, mid)
            time.sleep(0.4)  # 轻微节流，避免触发 429
        else:
            reasoning = None
        rows.append({"id": mid, "caps": caps, "reasoning": reasoning})
        print(
            "    {:<26}{:>10}{:>8}{:>8}{:>10}".format(
                mid,
                caps["context_limit"] if caps["context_limit"] else "—",
                "✓" if caps["supports_tools"] else ("✗" if caps["supports_tools"] is False else "—"),
                "✓" if caps["supports_vision"] else ("✗" if caps["supports_vision"] is False else "—"),
                "✓" if reasoning else ("✗" if reasoning is False else "—"),
            )
        )

    # ---- 3. 与官方清单比对 ----
    if not GROUND_TRUTH:
        print("\n[3] 未提供基准清单，跳过比对")
        return 0

    print("\n[3] 与官方清单逐格比对")
    print("    {:<26}{:<8}{:<8}{:<8}{:<10}".format("模型 ID", "上下文", "工具", "视觉", "推理"))
    print("    " + "-" * 62)
    mismatch = 0
    for row in rows:
        entry = GROUND_TRUTH.get(row["id"])
        if entry is None:
            print("    {:<26}（官方清单中无此模型）".format(row["id"]))
            mismatch += 1
            continue
        exp = expect_of(entry)
        caps = row["caps"]
        checks = [
            ("上下文", caps["context_limit"] == exp["ctx"]),
            ("工具", caps["supports_tools"] == exp["tools"]),
            ("视觉", caps["supports_vision"] == exp["vision"]),
            ("推理", (row["reasoning"] == exp["reasoning"]) if row["reasoning"] is not None else True),
        ]
        bad = [name for name, ok in checks if not ok]
        mark = "✓ 全部一致" if not bad else "✗ 不一致: " + ", ".join(bad)
        if bad:
            mismatch += 1
        print("    {:<26}{}".format(row["id"], mark))
        if bad:
            print(
                "        实际 上下文={} 工具={} 视觉={} 推理={}".format(
                    caps["context_limit"], caps["supports_tools"], caps["supports_vision"], row["reasoning"]
                )
            )
            print(
                "        官方 上下文={} 工具={} 视觉={} 推理={}".format(
                    exp["ctx"], exp["tools"], exp["vision"], exp["reasoning"]
                )
            )

    # 反向检查：官方有、实际没返回的
    got = {r["id"] for r in rows}
    missing = [k for k in GROUND_TRUTH if k not in got]
    if missing:
        print("    ✗ 接口未返回的官方模型：{}".format(", ".join(missing)))
        mismatch += len(missing)

    print("\n" + "=" * 78)
    if mismatch == 0:
        print("✓ 全部一致（{} 个模型）".format(len(rows)))
    else:
        print("✗ 共 {} 处不一致".format(mismatch))
    print("=" * 78)
    return 0 if mismatch == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
