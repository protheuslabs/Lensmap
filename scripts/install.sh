#!/usr/bin/env bash
set -euo pipefail

REPO="protheuslabs/Lensmap"
VERSION="${1:-latest}"
INSTALL_DIR="${INSTALL_DIR:-$HOME/.local/bin}"

uname_s="$(uname -s)"
uname_m="$(uname -m)"

case "$uname_s" in
  Linux) os="unknown-linux-gnu" ;;
  Darwin) os="apple-darwin" ;;
  *) echo "Unsupported OS: $uname_s" >&2; exit 1 ;;
esac

case "$uname_m" in
  x86_64|amd64) arch="x86_64" ;;
  arm64|aarch64) arch="aarch64" ;;
  *) echo "Unsupported architecture: $uname_m" >&2; exit 1 ;;
esac

if [[ "$VERSION" == "latest" ]]; then
  VERSION="$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n1)"
fi

if [[ -z "$VERSION" ]]; then
  echo "Unable to resolve release version" >&2
  exit 1
fi

asset="lensmap-${VERSION}-${arch}-${os}.tar.gz"
url="https://github.com/${REPO}/releases/download/${VERSION}/${asset}"

tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

if ! curl -fL "$url" -o "$tmp_dir/$asset"; then
  echo "No prebuilt asset found for ${arch}-${os} at ${VERSION}." >&2
  echo "Build from source with: cargo build --release" >&2
  exit 1
fi
mkdir -p "$INSTALL_DIR"
tar -xzf "$tmp_dir/$asset" -C "$tmp_dir"
install -m 0755 "$tmp_dir/lensmap" "$INSTALL_DIR/lensmap"

echo "Installed lensmap to $INSTALL_DIR/lensmap"
if ! command -v lensmap >/dev/null 2>&1; then
  echo "Add to PATH: export PATH=\"$INSTALL_DIR:\$PATH\""
fi
