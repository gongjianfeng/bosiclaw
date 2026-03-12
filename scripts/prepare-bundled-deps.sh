#!/bin/bash
# 本地开发时准备 bundled 依赖
# 用法: bash scripts/prepare-bundled-deps.sh
#
# 此脚本模拟 CI 中的 bundled deps 准备步骤，用于本地开发和测试。
# 运行后可以 `npm run tauri:build` 构建包含内置依赖的安装包。

set -euo pipefail

NODE_VERSION="22.22.1"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
TAURI_DIR="$PROJECT_DIR/src-tauri"

echo "=== 准备 bundled 依赖 ==="
echo "项目目录: $PROJECT_DIR"
echo ""

# 检测当前平台
OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
  Darwin)
    NODE_PLATFORM="darwin"
    if [ "$ARCH" = "arm64" ]; then
      NODE_ARCH="arm64"
      TARGET="aarch64-apple-darwin"
      KOFFI_KEEP="darwin_arm64"
    else
      NODE_ARCH="x64"
      TARGET="x86_64-apple-darwin"
      KOFFI_KEEP="darwin_x64"
    fi
    ;;
  Linux)
    NODE_PLATFORM="linux"
    NODE_ARCH="x64"
    TARGET="x86_64-unknown-linux-gnu"
    KOFFI_KEEP="linux_x64"
    ;;
  MINGW*|MSYS*|CYGWIN*)
    NODE_PLATFORM="win"
    NODE_ARCH="x64"
    TARGET="x86_64-pc-windows-msvc"
    KOFFI_KEEP="win32_x64"
    ;;
  *)
    echo "不支持的操作系统: $OS"
    exit 1
    ;;
esac

echo "平台: $OS ($ARCH)"
echo "Target triple: $TARGET"
echo "Node.js: v$NODE_VERSION ($NODE_PLATFORM-$NODE_ARCH)"
echo ""

mkdir -p "$TAURI_DIR/binaries" "$TAURI_DIR/resources"

# ── 1. 下载 Node.js 二进制 ──────────────────────────────
NODE_BIN="$TAURI_DIR/binaries/node-$TARGET"
if [ "$NODE_PLATFORM" = "win" ]; then
  NODE_BIN="${NODE_BIN}.exe"
fi

if [ -f "$NODE_BIN" ]; then
  echo "✓ Node.js binary 已存在: $NODE_BIN"
else
  echo "下载 Node.js v$NODE_VERSION..."
  if [ "$NODE_PLATFORM" = "win" ]; then
    curl -L "https://nodejs.org/dist/v${NODE_VERSION}/node-v${NODE_VERSION}-win-x64.zip" -o /tmp/node.zip
    unzip -j /tmp/node.zip "*/node.exe" -d "$TAURI_DIR/binaries/"
    mv "$TAURI_DIR/binaries/node.exe" "$NODE_BIN"
    rm /tmp/node.zip
  else
    curl -L "https://nodejs.org/dist/v${NODE_VERSION}/node-v${NODE_VERSION}-${NODE_PLATFORM}-${NODE_ARCH}.tar.gz" -o /tmp/node.tar.gz
    tar -xzf /tmp/node.tar.gz --strip-components=2 -C /tmp "*/bin/node"
    mv /tmp/node "$NODE_BIN"
    chmod +x "$NODE_BIN"
    rm /tmp/node.tar.gz
  fi
  echo "✓ Node.js binary 下载完成"
fi

# ── 2. 安装 openclaw 并打包 ─────────────────────────────
TGZ_FILE="$TAURI_DIR/resources/openclaw-runtime.tgz"

if [ -f "$TGZ_FILE" ]; then
  echo "✓ openclaw-runtime.tgz 已存在: $TGZ_FILE"
  echo "  如需重新生成，请先删除该文件"
else
  echo "安装 openclaw 到临时目录..."
  INSTALL_DIR=$(mktemp -d)
  cd "$INSTALL_DIR"
  npm install openclaw@latest --prefix .

  # npm 会把依赖 hoist 到顶层 node_modules/，需要搬回 openclaw 包内
  echo "将 hoisted 依赖移入 openclaw/node_modules/..."
  OPENCLAW_NM="node_modules/openclaw/node_modules"
  mkdir -p "$OPENCLAW_NM"
  for pkg in node_modules/*/; do
    name=$(basename "$pkg")
    [ "$name" = "openclaw" ] && continue
    [ "$name" = ".package-lock.json" ] && continue
    mv "$pkg" "$OPENCLAW_NM/"
  done
  # 移动 scoped packages (@scope/pkg)
  for scope in node_modules/@*/; do
    [ ! -d "$scope" ] && continue
    scope_name=$(basename "$scope")
    mkdir -p "$OPENCLAW_NM/$scope_name"
    for pkg in "$scope"/*/; do
      [ ! -d "$pkg" ] && continue
      mv "$pkg" "$OPENCLAW_NM/$scope_name/"
    done
  done
  echo "  依赖数量: $(ls "$OPENCLAW_NM" | wc -l)"

  # 精简 koffi
  KOFFI_DIR="$OPENCLAW_NM/koffi/build/koffi"
  if [ -d "$KOFFI_DIR" ]; then
    echo "精简 koffi 多平台构建，保留: $KOFFI_KEEP"
    for d in "$KOFFI_DIR"/*/; do
      platform_name=$(basename "$d")
      if [ "$platform_name" != "$KOFFI_KEEP" ]; then
        echo "  删除: $platform_name"
        rm -rf "$d"
      fi
    done
  fi

  # 打包
  echo "打包 openclaw-runtime.tgz..."
  tar -czf "$TGZ_FILE" -C node_modules/openclaw .

  cd "$PROJECT_DIR"
  rm -rf "$INSTALL_DIR"
  echo "✓ openclaw-runtime.tgz 生成完成"
fi

# ── 结果 ────────────────────────────────────────────────
echo ""
echo "=== 准备完成 ==="
echo "Binaries:"
ls -lh "$TAURI_DIR/binaries/"
echo ""
echo "Resources:"
ls -lh "$TAURI_DIR/resources/"
echo ""
echo "现在可以运行: npm run tauri:build"
