#!/usr/bin/env bash
set -euo pipefail

REPO="protheuslabs/Lensmap"
VERSION="${1:-latest}"
INSTALL_DIR="${INSTALL_DIR:-$HOME/.local/bin}"
VERIFY_CHECKSUMS="${VERIFY_CHECKSUMS:-0}"
CHECKSUM_ENV_PATH="${CHECKSUM_ENV_PATH:-https://github.com/${REPO}/releases/download/${VERSION}/lensmap-${VERSION}-checksums.txt}"
tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

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
checksum_url="${CHECKSUM_ENV_PATH}"
checksum_tmp="${tmp_dir}/lensmap-${VERSION}-checksums.txt"

if ! curl -fL "$url" -o "$tmp_dir/$asset"; then
  echo "No prebuilt asset found for ${arch}-${os} at ${VERSION}." >&2
  echo "Build from source with: cargo build --release" >&2
  exit 1
fi

if [ "$VERIFY_CHECKSUMS" = "1" ]; then
  if curl -fsSL "$checksum_url" -o "$checksum_tmp"; then
    expected=$(awk -v a="$asset" '{
      file=$2
      sub(/^.*\//, "", file)
      if (file==a) {
        print $1
        exit
      }
    }' "$checksum_tmp" | head -n1)
    if [ -z "$expected" ]; then
      echo "Unable to find checksum for ${asset} in release checksum file." >&2
      exit 1
    fi
    actual=$(sha256sum "$tmp_dir/$asset" | awk '{print $1}')
    if [ "${actual,,}" != "${expected,,}" ]; then
      echo "Checksum mismatch for ${asset}." >&2
      exit 1
    fi
  else
    echo "Checksum file unavailable at ${checksum_url}." >&2
    exit 1
  fi
fi
mkdir -p "$INSTALL_DIR"
archive_entry="$(tar -tf "$tmp_dir/$asset" | awk 'NF>0 && $0 !~ /^\.\/$/ {print $0; exit}')"
tar -xzf "$tmp_dir/$asset" -C "$tmp_dir"
extracted_binary="$tmp_dir/$archive_entry"
if [ ! -f "$extracted_binary" ]; then
  if [ -f "$tmp_dir/lensmap" ]; then
    extracted_binary="$tmp_dir/lensmap"
  else
    echo "Unable to locate extracted lensmap binary in archive." >&2
    exit 1
  fi
fi
install -m 0755 "$extracted_binary" "$INSTALL_DIR/lensmap"

echo "Installed lensmap to $INSTALL_DIR/lensmap"
if ! command -v lensmap >/dev/null 2>&1; then
  echo "Add to PATH: export PATH=\"$INSTALL_DIR:\$PATH\""
fi
