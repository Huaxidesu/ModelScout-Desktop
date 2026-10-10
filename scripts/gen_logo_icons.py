# -*- coding: utf-8 -*-
"""
ModelScout —— 从品牌 logo 生成全套 Tauri 应用图标（纯标准库实现，无第三方依赖）

输入：assets/images/logo-render.png（SVG 经 Edge headless 渲染，2508x2508 RGBA，透明背景）
输出：
  - src-tauri/icons/ 全套（32/128/256/512 PNG、icon.ico、icon.icns、Windows Store 系列）
  - src/assets/logo-icon.png（「关于」弹窗内展示用，128x128）

说明：
  - SVG 源图自带透明背景，无需去白底；内含浅蓝元素（rgba(200,236,255)），
    任何「按亮度抹白」的处理都会误伤，因此本脚本不做色彩过滤。
  - 图标内容本身位于画布中央（四周已有约 8% 留白），不额外做圆角，
    避免把分子网络的细小边缘裁掉。
  - 仅用面积平均降采样（多级倍率循环），保证缩小质量。
"""
import os
import re
import struct
import sys
import zlib

# ---- 保证中文状态信息在任何输出环境下都能正确输出 ----
if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    sys.stderr.reconfigure(encoding="utf-8", errors="replace")

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SRC = os.path.join(ROOT, "assets", "images", "logo-render.png")
ICONS_OUT = os.path.join(ROOT, "src-tauri", "icons")
ASSET_OUT = os.path.join(ROOT, "src", "assets")

# ---------------------------------------------------------------------------
# PNG 解码（RGB / RGBA，8bit）
# ---------------------------------------------------------------------------


def decode_png(path):
    data = open(path, "rb").read()
    pos = 8
    w = h = None
    idat = b""
    while pos < len(data):
        length = struct.unpack(">I", data[pos:pos + 4])[0]
        ctype = data[pos + 4:pos + 8]
        body = data[pos + 8:pos + 8 + length]
        if ctype == b"IHDR":
            w, h, depth, color_type = struct.unpack(">IIBB", body[:10])
        elif ctype == b"IDAT":
            idat += body
        pos += 12 + length
    assert depth == 8, "仅支持 8bit PNG"
    channels = 4 if color_type == 6 else 3
    raw = zlib.decompress(idat)
    stride = w * channels
    out = bytearray(w * h * channels)
    prev = bytearray(stride)
    for y in range(h):
        f = raw[y * (stride + 1)]
        line = bytearray(raw[y * (stride + 1) + 1:(y + 1) * (stride + 1)])
        if f == 1:
            for i in range(channels, stride):
                line[i] = (line[i] + line[i - channels]) & 255
        elif f == 2:
            for i in range(stride):
                line[i] = (line[i] + prev[i]) & 255
        elif f == 3:
            for i in range(stride):
                a = line[i - channels] if i >= channels else 0
                line[i] = (line[i] + ((a + prev[i]) >> 1)) & 255
        elif f == 4:
            for i in range(stride):
                a = line[i - channels] if i >= channels else 0
                b = prev[i]
                c = prev[i - channels] if i >= channels else 0
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                pr = a if (pa <= pb and pa <= pc) else (b if pb <= pc else c)
                line[i] = (line[i] + pr) & 255
        out[y * stride:(y + 1) * stride] = line
        prev = line
    return w, h, channels, bytes(out)


# ---------------------------------------------------------------------------
# RGBA 像素缓冲操作
# ---------------------------------------------------------------------------


def to_rgba(w, h, channels, px):
    """统一转成 RGBA 平面（bytearray, r,g,b,a 交错）。"""
    buf = bytearray(w * h * 4)
    for i in range(w * h):
        o = i * channels
        d = i * 4
        if channels == 4:
            buf[d:d + 4] = px[o:o + 4]
        else:
            buf[d:d + 3] = px[o:o + 3]
            buf[d + 3] = 255
    return buf


def box_downscale(src_buf, sw, sh, tw, th):
    """面积平均降采样（box filter），适合缩小 logo 图。"""
    out = bytearray(tw * th * 4)
    for ty in range(th):
        y0 = ty * sh // th
        y1 = max((ty + 1) * sh // th, y0 + 1)
        for tx in range(tw):
            x0 = tx * sw // tw
            x1 = max((tx + 1) * sw // tw, x0 + 1)
            rs = gs = bs = al = 0.0
            n = 0
            for yy in range(y0, y1):
                for xx in range(x0, x1):
                    d = (yy * sw + xx) * 4
                    rs += src_buf[d]
                    gs += src_buf[d + 1]
                    bs += src_buf[d + 2]
                    al += src_buf[d + 3]
                    n += 1
            if n == 0:
                n = 1
            o = (ty * tw + tx) * 4
            out[o] = int(rs / n)
            out[o + 1] = int(gs / n)
            out[o + 2] = int(bs / n)
            out[o + 3] = int(al / n)
    return out


# ---------------------------------------------------------------------------
# PNG 编码
# ---------------------------------------------------------------------------


def write_png(path, size, buf):
    """写 RGBA PNG，逐行 filter 0。"""
    raw = bytearray()
    for y in range(size):
        raw.append(0)
        raw += buf[y * size * 4:(y + 1) * size * 4]

    def chunk(tag, data):
        body = tag + data
        return (struct.pack(">I", len(data)) + body
                + struct.pack(">I", zlib.crc32(body) & 0xFFFFFFFF))

    blob = b"\x89PNG\r\n\x1a\n"
    blob += chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0))
    blob += chunk(b"IDAT", zlib.compress(bytes(raw), 9))
    blob += chunk(b"IEND", b"")
    with open(path, "wb") as fh:
        fh.write(blob)


# ---------------------------------------------------------------------------
# ICO / ICNS
# ---------------------------------------------------------------------------


def write_ico(path, png_paths):
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
    types = [b"ic07", b"ic08", b"ic09", b"ic10", b"ic11", b"ic12", b"ic13", b"ic14"]
    entries = b""
    for tag, p in zip(types, png_paths):
        with open(p, "rb") as fh:
            blob = fh.read()
        entries += tag + struct.pack(">I", len(blob) + 8) + blob
    with open(path, "wb") as fh:
        fh.write(b"icns" + struct.pack(">I", len(entries) + 8) + entries)


# ---------------------------------------------------------------------------
# 主流程
# ---------------------------------------------------------------------------

# 源图只解码一次，全流程复用
_SRC_BUF = None
_SRC_W = _SRC_H = None


def _load_source():
    global _SRC_BUF, _SRC_W, _SRC_H
    if _SRC_BUF is None:
        sw, sh, ch, px = decode_png(SRC)
        _SRC_W, _SRC_H = sw, sh
        _SRC_BUF = to_rgba(sw, sh, ch, px)
    return _SRC_BUF, _SRC_W, _SRC_H


# 尺寸 → 渲染结果缓存（Store 系列 10 个文件共用 128，避免重复计算）
_RENDER_CACHE = {}


def render(size):
    """渲染某个尺寸的 RGBA buffer（源图为透明背景，无需去底/圆角）。"""
    if size in _RENDER_CACHE:
        return _RENDER_CACHE[size]
    buf, sw, sh = _load_source()
    # 多级降采样（2508 → ... → size）
    while sw > size * 2:
        nw = max(size, sw // 2)
        nh = max(size, sh // 2)
        buf = box_downscale(buf, sw, sh, nw, nh)
        sw, sh = nw, nh
    if sw != size or sh != size:
        buf = box_downscale(buf, sw, sh, size, size)
    _RENDER_CACHE[size] = buf
    return buf


def main():
    os.makedirs(ICONS_OUT, exist_ok=True)
    os.makedirs(ASSET_OUT, exist_ok=True)

    # 基础尺寸 PNG
    sizes = [32, 128, 256, 512]
    for s in sizes:
        write_png(os.path.join(ICONS_OUT, f"{s}x{s}.png"), s, render(s))

    # tauri.conf.json 引用的别名尺寸
    aliases = [("32x32.png", 32), ("128x128.png", 128), ("128x128@2x.png", 256), ("icon.png", 512)]
    for name, s in aliases:
        write_png(os.path.join(ICONS_OUT, name), s, render(s))

    # Windows / macOS 容器
    write_ico(
        os.path.join(ICONS_OUT, "icon.ico"),
        [os.path.join(ICONS_OUT, f"{s}x{s}.png") for s in (32, 128, 256, 512)],
    )
    icns_sources = [
        os.path.join(ICONS_OUT, n)
        for n in ("32x32.png", "128x128.png", "128x128@2x.png", "icon.png")
    ]
    write_icns(os.path.join(ICONS_OUT, "icon.icns"), icns_sources + icns_sources)

    # Windows Store 系列（方形容器，透明背景直接保留，复用 128 渲染结果）
    store_names = [
        "Square30x30Logo.png", "Square44x44Logo.png", "Square71x71Logo.png",
        "Square89x89Logo.png", "Square107x107Logo.png", "Square142x142Logo.png",
        "Square150x150Logo.png", "Square284x284Logo.png", "Square310x310Logo.png",
        "StoreLogo.png",
    ]
    buf_128 = render(128)
    for name in store_names:
        write_png(os.path.join(ICONS_OUT, name), 128, buf_128)

    # 「关于」弹窗内展示用 logo（128）
    write_png(os.path.join(ASSET_OUT, "logo-icon.png"), 128, buf_128)

    print("应用图标生成完成，共", len(os.listdir(ICONS_OUT)), "个文件；关于页 logo 已写入 src/assets/")


if __name__ == "__main__":
    main()