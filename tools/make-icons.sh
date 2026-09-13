#!/usr/bin/env bash
# 图标产物生成：从 .assets/sumi.svg（唯一真源）生成全部位图产物并提交进仓库。
#   .assets/icon.png  512×512（README 展示 / web favicon）
#   .assets/icon.icns macOS .app 打包用（iconutil）
#   .assets/icon.ico  Windows 打包用（PNG-in-ICO 多尺寸）
#
# 用法: ./tools/make-icons.sh（仓库根目录或任意位置执行均可）
# 产物已入库，本脚本仅图标改版时由维护者运行；渲染器按可用性自动选择：
#   rsvg-convert → magick → inkscape → Chrome/Chromium headless → qlmanage（无 alpha 的降级路径）
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SVG="$REPO_ROOT/.assets/sumi.svg"
ASSETS="$REPO_ROOT/.assets"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/sumi-icons.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

[ -f "$SVG" ] || { echo "图标真源不存在: $SVG"; exit 1; }

# ---------- SVG → PNG（指定尺寸，带 alpha） ----------
render_png() { # $1=输出 $2=尺寸
  local out=$1 size=$2
  if command -v rsvg-convert >/dev/null 2>&1; then
    rsvg-convert -w "$size" -h "$size" -o "$out" "$SVG"
  elif command -v magick >/dev/null 2>&1; then
    magick -background none "$SVG" -resize "${size}x${size}" "$out"
  elif command -v inkscape >/dev/null 2>&1; then
    inkscape "$SVG" -w "$size" -h "$size" -o "$out"
  else
    # Chrome/Chromium headless：--default-background-color 保留画布透明区
    local chrome=""
    for c in "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" \
             "/Applications/Chromium.app/Contents/MacOS/Chromium" \
             google-chrome chromium-browser chromium; do
      if command -v "$c" >/dev/null 2>&1 || [ -x "$c" ]; then chrome="$c"; break; fi
    done
    if [ -n "$chrome" ]; then
      "$chrome" --headless --disable-gpu --hide-scrollbars \
        --screenshot="$(cd "$(dirname "$out")" && pwd)/$(basename "$out")" \
        --window-size="$size,$size" --default-background-color=00000000 \
        "file://$SVG" >/dev/null 2>&1
    elif command -v qlmanage >/dev/null 2>&1; then
      echo "警告: 仅有 qlmanage 可用，产物不带 alpha（画布透明区会变白），建议安装 librsvg 或 Chrome。" >&2
      qlmanage -t -s "$size" -o "$(dirname "$out")" "$SVG" >/dev/null 2>&1
      mv "${SVG%.svg}.svg.png" "$out"
    else
      echo "未找到可用的 SVG 渲染器（rsvg-convert / magick / inkscape / Chrome / qlmanage）。"; exit 1
    fi
  fi
  [ -f "$out" ] || { echo "渲染失败: $out"; exit 1; }
}

# 母版（1024，最高分辨率产物由此缩放，避免各尺寸独立渲染的细微差异）
MASTER="$WORK/master.png"
render_png "$MASTER" 1024

# ---------- icon.png（512，README 展示） ----------
if command -v sips >/dev/null 2>&1; then
  sips -z 512 512 "$MASTER" --out "$ASSETS/icon.png" >/dev/null
elif command -v magick >/dev/null 2>&1; then
  magick "$MASTER" -resize 512x512 "$ASSETS/icon.png"
else
  cp "$MASTER" "$ASSETS/icon.png" # 退化：README 指定显示尺寸，文件大一点也能用
fi
echo ".assets/icon.png (512)"

# ---------- icon.icns（macOS，多尺寸 iconset → iconutil） ----------
if command -v iconutil >/dev/null 2>&1 && command -v sips >/dev/null 2>&1; then
  ICONSET="$WORK/icon.iconset"
  mkdir -p "$ICONSET"
  for s in 16 32 128 256 512; do
    sips -z "$s" "$s" "$MASTER" --out "$ICONSET/icon_${s}x${s}.png" >/dev/null
    sips -z $((s * 2)) $((s * 2)) "$MASTER" --out "$ICONSET/icon_${s}x${s}@2x.png" >/dev/null
  done
  iconutil -c icns "$ICONSET" -o "$ASSETS/icon.icns"
  echo ".assets/icon.icns"
else
  echo "跳过 icon.icns（需要 macOS 的 iconutil + sips）。"
fi

# ---------- icon.ico（Windows，PNG-in-ICO 多尺寸容器） ----------
python3 - "$MASTER" "$ASSETS/icon.ico" <<'PY'
import struct, subprocess, sys, tempfile, os

master, ico_out = sys.argv[1], sys.argv[2]
sizes = [16, 24, 32, 48, 64, 128, 256]
blobs = []
for s in sizes:
    with tempfile.NamedTemporaryFile(suffix=".png", delete=False) as f:
        tmp = f.name
    subprocess.run(["sips", "-z", str(s), str(s), master, "--out", tmp],
                   check=True, capture_output=True)
    with open(tmp, "rb") as f:
        blobs.append(f.read())
    os.unlink(tmp)

# ICONDIR + ICONDIRENTRY + PNG 数据（Vista 起 ICO 支持 PNG 负载，electron-builder/Windows 均接受）
count = len(blobs)
header = struct.pack("<HHH", 0, 1, count)
offset = 6 + 16 * count
entries = b""
for s, blob in zip(sizes, blobs):
    entries += struct.pack("<BBBBHHII", s % 256, s % 256, 0, 0, 1, 32, len(blob), offset)
    offset += len(blob)
with open(ico_out, "wb") as f:
    f.write(header + entries + b"".join(blobs))
print(".assets/icon.ico")
PY

echo "完成：产物已写入 .assets/（icon.png / icon.icns / icon.ico），请一并提交。"
