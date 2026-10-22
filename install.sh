#!/bin/bash

# Install ascii-aquarium from this checkout into ~/.local/bin, and link it in
# as a screensaver-rotation runner.

set -euo pipefail

repo=$(cd "$(dirname "$0")" && pwd)

cargo build --release --locked --manifest-path "$repo/Cargo.toml"
# install(1) replaces the file rather than rewriting it, so a running
# screensaver keeps its copy.
install -Dm755 "$repo/target/release/ascii-aquarium" "$HOME/.local/bin/ascii-aquarium"
echo "built ~/.local/bin/ascii-aquarium"
runners="$HOME/.local/lib/screensaver-rotation/runners"
mkdir -p "$runners"
ln -sfn "$HOME/.local/bin/ascii-aquarium" "$runners/aquarium"
rm -f "${XDG_DATA_HOME:-$HOME/.local/share}/ascii-aquarium/foot"
