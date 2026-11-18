#!/usr/bin/env bash
# 将 release 二进制打包为 .deb（amd64）。
# 前置：当前目录已存在 cargo 构建的 `pixel-synthesis-calculator`。
# 输出：pixel-synthesis-calculator_<版本>_amd64.deb
set -euo pipefail

VERSION="${1:-${GITHUB_REF_NAME#v}}"
VERSION="${VERSION:-1.0.0}"
PKG_NAME="pixel-synthesis-calculator"

if [[ -f "target/release/$PKG_NAME" ]]; then
    BIN="target/release/$PKG_NAME"
elif [[ -f "$PKG_NAME" ]]; then
    BIN="$PKG_NAME"
else
    echo "缺少二进制 $PKG_NAME（target/release/ 或当前目录），请先 cargo build --release" >&2
    exit 1
fi

STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT

mkdir -p "$STAGE/DEBIAN" \
    "$STAGE/usr/bin" \
    "$STAGE/usr/share/applications" \
    "$STAGE/usr/share/pixmaps"

install -m 755 "$BIN" "$STAGE/usr/bin/$PKG_NAME"

cat > "$STAGE/DEBIAN/control" <<EOF
Package: $PKG_NAME
Version: $VERSION
Section: games
Priority: optional
Architecture: amd64
Maintainer: RayYiHang <RayYiHang@users.noreply.github.com>
Depends: libfontconfig1, libfreetype6, libxkbcommon0, libxkbcommon-x11-0, libwayland-client0, libvulkan1
Description: 像素星舰 (Pixel Starships) 英雄合成路线计算器
 Rust GUI 版：输入目标英雄与已有材料，枚举全部可合成路线（3★+3★→4★，4★+4★→5★，5★+5★→7★）。
EOF

cat > "$STAGE/usr/share/applications/$PKG_NAME.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=像素星舰合成计算器
Comment=Pixel Starships 英雄合成路线计算
Exec=$PKG_NAME
Terminal=false
Categories=Game;Utility;
EOF

OUTPUT="$PKG_NAME_${VERSION}_amd64.deb"
rm -f "$OUTPUT"
dpkg-deb --build --root-owner-group "$STAGE" "$OUTPUT"
echo "已生成 $OUTPUT"
