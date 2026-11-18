#!/usr/bin/env bash
# 将通用（universal）二进制打包为 .app 并生成 .dmg。
# 前置：当前目录已存在 lipo 合并后的通用二进制 `PixelSynthesisCalculator`。
# 输出：PixelSynthesisCalculator-macos-universal.dmg
set -euo pipefail

APP_NAME="PixelSynthesisCalculator"
VERSION="${1:-${GITHUB_REF_NAME#v}}"
VERSION="${VERSION:-1.0.0}"

if [[ "$(uname)" != "Darwin" ]]; then
    echo "此脚本只能在 macOS 上运行" >&2
    exit 1
fi
if [[ ! -f "$APP_NAME" ]]; then
    echo "缺少通用二进制 $APP_NAME，请先构建并 lipo 合并" >&2
    exit 1
fi

APP_DIR="$APP_NAME.app"
rm -rf "$APP_DIR"
mkdir -p "$APP_DIR/Contents/MacOS" "$APP_DIR/Contents/Resources"

cp "$APP_NAME" "$APP_DIR/Contents/MacOS/"
chmod +x "$APP_DIR/Contents/MacOS/$APP_NAME"

cat > "$APP_DIR/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>$APP_NAME</string>
    <key>CFBundleDisplayName</key><string>像素星舰合成计算器</string>
    <key>CFBundleIdentifier</key><string>io.github.rayyihang.pixel-synthesis-calculator</string>
    <key>CFBundleVersion</key><string>$VERSION</string>
    <key>CFBundleShortVersionString</key><string>$VERSION</string>
    <key>CFBundleExecutable</key><string>$APP_NAME</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
    <key>LSMinimumSystemVersion</key><string>10.15</string>
    <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
EOF

# 未签名应用提示：首次打开需右键 → 打开，或执行 xattr -cr
rm -f "$APP_NAME-macos-universal.dmg"
hdiutil create -volname "$APP_NAME $VERSION" \
    -srcfolder "$APP_DIR" \
    -ov -format UDZO \
    "$APP_NAME-macos-universal.dmg"

echo "已生成 $APP_NAME-macos-universal.dmg"
