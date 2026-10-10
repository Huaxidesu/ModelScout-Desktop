"""交叉核对 Rust 结构体字段与 TS 接口字段是否一一对应。

作为阶段 5 QA 的可执行化工具，避免人工核对遗漏。

用法：
    python scripts/cross_check.py
"""
import io
import os
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


ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def to_camel(snake: str) -> str:
    """snake_case -> camelCase，与 serde rename_all = "camelCase" 的规则一致。"""
    parts = snake.split("_")
    return parts[0] + "".join(p.capitalize() for p in parts[1:])


def _looks_like_vue_expression(inner: str) -> bool:
    """判断 {{ ... }} 内的内容是否像 Vue 模板表达式而非未替换的占位符。

    判据（命中任一即为 Vue 表达式）：
      - 含点号（属性访问，如 probe.store.total）
      - 含括号（函数调用，如 summarizeModel(model)）
      - 含运算符或问号（三元、可选链，如 a ? b : c）
      - 含引号（字符串字面量）
      - 含中括号（索引/数组，如 STAGE_LABELS[item.stage]）
      - 只有一个单词且全部小写、无空格 —— 这是最常见的绑定形态
    """
    token = inner.strip()
    if not token:
        return False
    if any(ch in token for ch in ".[]()?:'\""):
        return True
    if any(op in token for op in ("+", "-", "*", "/", "&&", "||", "==", ">", "<")):
        return True
    # 单个标识符（如 statusLabel / version / dictCount）
    if re.fullmatch(r"[A-Za-z_$][\w$]*", token):
        return True
    return False


def read(path: str) -> str:
    with io.open(path, encoding="utf-8") as fh:
        return fh.read()


def extract_rust_struct(text: str, name: str) -> list[str]:
    """提取 Rust 结构体的 pub 字段名（排除文档注释与属性）。"""
    m = re.search(r"pub struct %s \{(.*?)^\}" % re.escape(name), text, re.S | re.M)
    if not m:
        return []
    body = m.group(1)
    fields = []
    for line in body.split("\n"):
        stripped = line.strip()
        if stripped.startswith("//") or stripped.startswith("#["):
            continue
        fm = re.match(r"pub\s+(\w+)\s*:", stripped)
        if fm:
            fields.append(fm.group(1))
    return fields


def extract_ts_interface(text: str, name: str) -> list[str]:
    """提取 TS interface 的字段名。"""
    m = re.search(
        r"export interface %s \{(.*?)^\}" % re.escape(name), text, re.S | re.M
    )
    if not m:
        return []
    body = m.group(1)
    fields = []
    for line in body.split("\n"):
        stripped = line.strip()
        if stripped.startswith("//") or stripped.startswith("*") or stripped.startswith("/*"):
            continue
        fm = re.match(r"(\w+)\??\s*:", stripped)
        if fm:
            fields.append(fm.group(1))
    return fields


# 需要核对的 (Rust 结构体名, TS 接口名) 组合
PAIRS = [
    ("ProbeConfig", "ProbeConfig"),
    ("ModelInfo", "ModelInfo"),
    ("ModelBrief", "ModelBrief"),
    ("ModelListResult", "ModelListResult"),
    ("PingResult", "PingResult"),
    ("ProbeRunSummary", "ProbeRunSummary"),
    ("ProbeSnapshot", "ProbeSnapshot"),
    ("ProbeProgress", "ProbeProgress"),
    ("ProbeLog", "ProbeLog"),
    ("ProbeModelDone", "ProbeModelDone"),
    ("ProbeFinished", "ProbeFinished"),
    ("ProbeFatal", "ProbeFatal"),
    ("SaveConfigRequest", "SaveConfigRequest"),
    ("SaveConfigResult", "SaveConfigResult"),
]


def main() -> int:
    rust_text = read(os.path.join(ROOT, "src-tauri", "src", "models.rs"))
    ts_text = read(os.path.join(ROOT, "src", "types", "contracts.ts"))

    failures = 0
    print("=" * 72)
    print("命令名核对：Rust generate_handler  vs  前端 COMMAND_NAMES")
    print("=" * 72)
    lib_text = read(os.path.join(ROOT, "src-tauri", "src", "lib.rs"))
    handler_block = re.search(r"generate_handler!\[(.*?)\]", lib_text, re.S)
    rust_cmds = set()
    if handler_block:
        rust_cmds = set(re.findall(r"commands::(\w+)", handler_block.group(1)))

    contracts_text = read(os.path.join(ROOT, "src", "types", "contracts.ts"))
    names_block = re.search(
        r"export const COMMAND_NAMES = \{(.*?)\} as const", contracts_text, re.S
    )
    ts_cmds = set()
    if names_block:
        ts_cmds = set(re.findall(r":\s*'([a-z_]+)'", names_block.group(1)))

    # 前端额外硬编码的辅助命令
    cmd_api_text = read(os.path.join(ROOT, "src", "api", "commands.ts"))
    extra_ts = set(re.findall(r"call<[^>]*>\('([a-z_]+)'", cmd_api_text))
    ts_cmds_all = ts_cmds | extra_ts

    print(f"Rust 注册 ({len(rust_cmds)}): {sorted(rust_cmds)}")
    print(f"前端常量 ({len(ts_cmds)}): {sorted(ts_cmds)}")
    print(f"前端硬编码 ({len(extra_ts)}): {sorted(extra_ts)}")
    only_rust = rust_cmds - ts_cmds_all
    only_ts = ts_cmds_all - rust_cmds
    if only_rust:
        print(f"  ✗ 仅 Rust 有：{sorted(only_rust)}")
        failures += 1
    if only_ts:
        print(f"  ✗ 仅前端有：{sorted(only_ts)}")
        failures += 1
    if not only_rust and not only_ts:
        print("  ✓ 命令名完全一致")

    print()
    print("=" * 72)
    print("事件名核对：Rust events.rs  vs  前端 EVENT_NAMES")
    print("=" * 72)
    events_text = read(os.path.join(ROOT, "src-tauri", "src", "events.rs"))
    rust_events = set(re.findall(r'pub const EVENT_\w+: &str = "([^"]+)"', events_text))

    ev_block = re.search(
        r"export const EVENT_NAMES = \{(.*?)\} as const", contracts_text, re.S
    )
    ts_events = set()
    if ev_block:
        ts_events = set(re.findall(r":\s*'([^']+)'", ev_block.group(1)))

    print(f"Rust ({len(rust_events)}): {sorted(rust_events)}")
    print(f"前端 ({len(ts_events)}): {sorted(ts_events)}")
    only_rust_ev = rust_events - ts_events
    only_ts_ev = ts_events - rust_events
    if only_rust_ev:
        print(f"  ✗ 仅 Rust 有：{sorted(only_rust_ev)}")
        failures += 1
    if only_ts_ev:
        print(f"  ✗ 仅前端有：{sorted(only_ts_ev)}")
        failures += 1
    if not only_rust_ev and not only_ts_ev:
        print("  ✓ 事件名完全一致")

    print()
    print("=" * 72)
    print("结构体字段核对：Rust(serde camelCase)  vs  TS interface")
    print("=" * 72)
    for rust_name, ts_name in PAIRS:
        rust_fields = extract_rust_struct(rust_text, rust_name)
        ts_fields = extract_ts_interface(ts_text, ts_name)
        if not rust_fields:
            print(f"  ! {rust_name}: 未在 models.rs 中找到")
            failures += 1
            continue
        if not ts_fields:
            print(f"  ! {ts_name}: 未在 contracts.ts 中找到")
            failures += 1
            continue

        rust_camel = set(to_camel(f) for f in rust_fields)
        ts_set = set(ts_fields)
        missing = rust_camel - ts_set
        extra = ts_set - rust_camel
        if missing or extra:
            print(f"  ✗ {rust_name} ({len(rust_fields)} 字段)")
            if missing:
                print(f"      TS 缺失：{sorted(missing)}")
            if extra:
                print(f"      TS 多余：{sorted(extra)}")
            failures += 1
        else:
            print(f"  ✓ {rust_name} ↔ {ts_name}  ({len(rust_fields)} 字段一致)")

    print()
    print("=" * 72)
    print("占位符扫描：源码中不得残留 TODO / 省略 / 待补充")
    print("=" * 72)
    patterns = [
        (r"\bTODO\b", "TODO"),
        (r"\bFIXME\b", "FIXME"),
        (r"unimplemented!\s*\(", "unimplemented!()"),
        (r"todo!\s*\(", "todo!()"),
        (r"此处省略", "此处省略"),
        (r"待补充", "待补充"),
        (r"由读者自行", "由读者自行"),
        (r"\{\{[^}]*\}\}", "未替换的 {{}} 占位符"),
    ]
    scan_dirs = [
        os.path.join(ROOT, "src"),
        os.path.join(ROOT, "src-tauri", "src"),
    ]
    # 这些文件自身的职责就是「检测占位符」，其正则字面量会被误报
    whitelist = {
        os.path.join(ROOT, "src", "generators", "opencode.ts"),
        os.path.join(ROOT, "src", "generators", "deepseekHardness.ts"),
    }
    hits = 0
    for scan_dir in scan_dirs:
        for dirpath, _dirnames, filenames in os.walk(scan_dir):
            for filename in filenames:
                if not filename.endswith((".rs", ".ts", ".vue")):
                    continue
                full = os.path.join(dirpath, filename)
                if full in whitelist:
                    continue
                text = read(full)
                is_vue = filename.endswith(".vue")
                for lineno, line in enumerate(text.split("\n"), 1):
                    for pattern, label in patterns:
                        if not re.search(pattern, line):
                            continue
                        # Vue 模板里的 {{ expression }} 是双向绑定语法，不是占位符。
                        # 判定依据：大括号内不是「裸露的标识符/短语」而是含
                        # 点号、括号、运算符、引号等引用表达式特征，即视为正常绑定。
                        if is_vue and label == "未替换的 {{}} 占位符":
                            inner = re.search(r"\{\{([^}]*)\}\}", line)
                            if inner and _looks_like_vue_expression(inner.group(1)):
                                continue
                        rel = os.path.relpath(full, ROOT)
                        print(f"  {rel}:{lineno}  [{label}]  {line.strip()[:80]}")
                        hits += 1
    if hits == 0:
        print("  ✓ 未发现占位符")
    else:
        print(f"  共 {hits} 处命中，需人工确认是否为误报")
        failures += 1

    print()
    print("=" * 72)
    if failures == 0:
        print("✓ 交叉核对全部通过")
    else:
        print(f"✗ 交叉核对发现 {failures} 项不一致")
    print("=" * 72)

    return 0 if failures == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
