"""生成 ModelScout 需要的 Tauri 图标资源（纯标准库实现，无第三方依赖）。

产出：
  - 多尺寸 PNG（32/128/256/512）
  - icon.ico（多尺寸内嵌 PNG，Windows）
  - icon.icns（ic07-ic14 槽位，macOS）
  - Windows Store 系列方形 Logo
"""
import os
import re
import struct
import zlib
import sys

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


OUT = r"D:\Develop\Ai\ModelScout-Desktop\src-tauri\icons"

# ModelScout 品牌主色：青绿 #14B8A6
COLOR = (20, 184, 166, 255)


def write_png(path, size, rgba):
    """写一张纯色 PNG（8 位 RGBA）。"""
    raw = b""
    row = b"\x00" + bytes(rgba) * size  # 每行前缀 filter byte 0
    raw = row * size

    def chunk(tag, data):
        body = tag + data
        return (
            struct.pack(">I", len(data))
            + body
            + struct.pack(">I", zlib.crc32(body) & 0xFFFFFFFF)
        )

    blob = b"\x89PNG\r\n\x1a\n"
    blob += chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0))
    blob += chunk(b"IDAT", zlib.compress(raw, 9))
    blob += chunk(b"IEND", b"")
    with open(path, "wb") as fh:
        fh.write(blob)


def write_ico(path, png_paths):
    """把若干 PNG 打包成 ICO（Vista+ 支持 PNG 内嵌）。"""
    blobs = []
    for p in png_paths:
        with open(p, "rb") as fh:
            blobs.append(fh.read())

    count = len(blobs)
    header = struct.pack("<HHH", 0, 1, count)
    entries = b""
    offset = 6 + 16 * count

    for p, blob in zip(png_paths, blobs):
        m = re.search(r"(\d+)x\1\.png$", os.path.basename(p))
        size = int(m.group(1)) if m else 32
        dim = 0 if size >= 256 else size
        entries += struct.pack("<BBBBHHII", dim, dim, 0, 0, 1, 32, len(blob), offset)
        offset += len(blob)

    payload = b"".join(blobs)
    with open(path, "wb") as fh:
        fh.write(header + entries + payload)


def write_icns(path, png_paths):
    """把若干 PNG 打包成 ICNS。"""
    types = [b"ic07", b"ic08", b"ic09", b"ic10", b"ic11", b"ic12", b"ic13", b"ic14"]
    entries = b""
    for tag, p in zip(types, png_paths):
        with open(p, "rb") as fh:
            blob = fh.read()
        entries += tag + struct.pack(">I", len(blob) + 8) + blob
    with open(path, "wb") as fh:
        fh.write(b"icns" + struct.pack(">I", len(entries) + 8) + entries)


def main():
    os.makedirs(OUT, exist_ok=True)

    # 基础尺寸
    for size in (32, 128, 256, 512):
        write_png(os.path.join(OUT, f"{size}x{size}.png"), size, COLOR)

    # 别名尺寸（tauri.conf.json 里引用的名字）
    aliases = [("32x32.png", 32), ("128x128.png", 128), ("128x128@2x.png", 256), ("icon.png", 512)]
    for name, size in aliases:
        write_png(os.path.join(OUT, name), size, COLOR)

    # Windows / macOS 容器
    write_ico(
        os.path.join(OUT, "icon.ico"),
        [os.path.join(OUT, f"{s}x{s}.png") for s in (32, 128, 256, 512)],
    )
    icns_sources = [
        os.path.join(OUT, n)
        for n in ("32x32.png", "128x128.png", "128x128@2x.png", "icon.png")
    ]
    write_icns(os.path.join(OUT, "icon.icns"), icns_sources + icns_sources)

    # Windows Store 系列
    store_names = [
        "Square30x30Logo.png", "Square44x44Logo.png", "Square71x71Logo.png",
        "Square89x89Logo.png", "Square107x107Logo.png", "Square142x142Logo.png",
        "Square150x150Logo.png", "Square284x284Logo.png", "Square310x310Logo.png",
        "StoreLogo.png",
    ]
    for name in store_names:
        write_png(os.path.join(OUT, name), 128, COLOR)

    print("图标生成完成，共", len(os.listdir(OUT)), "个文件")


if __name__ == "__main__":
    main()
