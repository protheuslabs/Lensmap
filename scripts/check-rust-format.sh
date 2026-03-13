#!/usr/bin/env bash
set -euo pipefail

mapfile -d '' rust_files < <(
  git ls-files \
    | awk '/\.rs$/' \
    | grep -v '^crates/lensmap-cli/src/main.rs$' \
    | tr '\n' '\0'
)

if [ "${#rust_files[@]}" -eq 0 ]; then
  echo "No Rust files to format-check (main formatter file is excluded)."
  exit 0
fi

echo "Checking format for ${#rust_files[@]} Rust file(s)..."
rustfmt --check --edition 2021 --config-path . -- "${rust_files[@]}"

