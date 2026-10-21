#!/bin/bash

# Install ascii-aquarium from this checkout into ~/.local/bin, where the
# shader screensaver's launcher looks for it.

set -euo pipefail

repo=$(cd "$(dirname "$0")" && pwd)

cargo build --release --locked --manifest-path "$repo/Cargo.toml"
# install(1) replaces the file rather than rewriting it, so a running
# screensaver keeps its copy.
install -Dm755 "$repo/target/release/ascii-aquarium" "$HOME/.local/bin/ascii-aquarium"
echo "built ~/.local/bin/ascii-aquarium"
rm -f "${XDG_DATA_HOME:-$HOME/.local/share}/ascii-aquarium/foot"
