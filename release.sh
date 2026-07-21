#!/usr/bin/env sh
# Build a release `ramos` binary and stage it in dist/.
#
# Usage:
#   ./release.sh
set -e
cd "$(dirname "$0")"

out_dir="dist"
bin="$out_dir/ramos"
version=$(grep '^version' Cargo.toml | head -1 | cut -d'"' -f2)
host=$(rustc -vV | sed -n 's/^host: //p')

echo "Building ramos $version for $host (release)…"
cargo build --release

mkdir -p "$out_dir"
# The release profile already strips symbols (see Cargo.toml).
cp target/release/ramos "$bin"

size=$(ls -lh "$bin" | awk '{print $5}')
echo "Staged $bin ($size)"
