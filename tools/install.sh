#!/usr/bin/env bash
# 本机安装：构建 sumi 桌面应用并安装到本机（编译复用 build-app.sh，不重复实现）。
# macOS 安装到 /Applications/sumi.app；Linux 安装到 ~/.local/bin/sumi 并注册用户级桌面启动器
# （AppImage 同时归置到仓库根目录的 out/）。sumi-daemon 随应用包携带，由 Electron 拉起/回收。
#
# 用法: ./tools/install.sh（仓库根目录或任意位置执行均可）
# 前置: 最新 Node.js 与 Rust 工具链（https://rustup.rs/）
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_DIR="$REPO_ROOT/sumi-app"
BUILD="$REPO_ROOT/tools/build-app.sh"

case "$(uname -s)" in
  Darwin)
    # --unpacked 跳过 zip 压缩，安装只需要 .app 目录
    "$BUILD" --unpacked
    APP="$(ls -d "$APP_DIR"/dist/mac*/sumi.app 2>/dev/null | head -n1)"
    [ -n "$APP" ] || { echo "打包产物不存在: dist/mac*/sumi.app（electron-builder 未产出）"; exit 1; }
    rm -rf "/Applications/sumi.app"
    cp -R "$APP" "/Applications/sumi.app"
    echo "完成：应用已安装到 /Applications/sumi.app。"
    ;;
  Linux)
    # 复用完整构建（产物同时归置到 out/），再执行用户级安装：
    # AppImage → ~/.local/bin/sumi；图标 → hicolor；启动器 → 用户应用目录（GNOME 应用列表可搜索）
    "$BUILD"
    APPIMAGE="$REPO_ROOT/out/sumi-linux-x64.AppImage"
    [ -f "$APPIMAGE" ] || { echo "打包产物不存在: $APPIMAGE（electron-builder 未产出）"; exit 1; }

    BIN_DIR="$HOME/.local/bin"
    DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}"
    DESKTOP_DIR="$DATA_DIR/applications"
    ICON_DIR="$DATA_DIR/icons/hicolor/512x512/apps"
    mkdir -p "$BIN_DIR" "$DESKTOP_DIR" "$ICON_DIR"

    # 同目录临时文件 + 原子改名：旧版本正在运行时旧 inode 继续服务运行中的进程，不会读到写了一半的文件
    TMP_BIN="$BIN_DIR/.sumi-install.$$"
    trap 'rm -f "$TMP_BIN"' EXIT
    install -m 755 "$APPIMAGE" "$TMP_BIN"
    mv -f "$TMP_BIN" "$BIN_DIR/sumi"
    install -m 644 "$REPO_ROOT/.assets/icon.png" "$ICON_DIR/sumi.png"

    # Exec 路径按 freedesktop 规范转义（双引号内需转义 \ " $ `；HOME 含空格等字符时不至于失效）
    EXEC_PATH="$(printf '%s' "$BIN_DIR/sumi" | sed 's/[\\"$`]/\\&/g')"
    cat > "$DESKTOP_DIR/sumi.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=sumi
Comment=对标 Calibre 的开源书籍/文本资源管理工具
Exec="$EXEC_PATH" %U
Icon=sumi
Terminal=false
Categories=Office;
StartupWMClass=sumi
EOF
    chmod 644 "$DESKTOP_DIR/sumi.desktop"

    # 刷新桌面数据库与图标缓存（缺工具则跳过）
    update-desktop-database "$DESKTOP_DIR" >/dev/null 2>&1 || true
    gtk-update-icon-cache -q -t -f "$DATA_DIR/icons/hicolor" >/dev/null 2>&1 || true

    echo "完成：应用已安装到 $BIN_DIR/sumi，启动器已注册（GNOME 应用列表搜索 sumi 即可启动）。"
    case ":$PATH:" in
      *":$BIN_DIR:"*) ;;
      *) echo "提示：$BIN_DIR 不在 PATH，终端启动请用完整路径或把该目录加入 PATH。" ;;
    esac
    ;;
  *)
    echo "不支持的平台: $(uname -s)（Windows 请使用 tools/install.ps1）"; exit 1
    ;;
esac
