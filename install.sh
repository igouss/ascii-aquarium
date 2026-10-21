#!/bin/bash

# Install ascii-aquarium from this checkout: build it, and link the foot
# configs the screensaver launcher runs each scene with.

set -euo pipefail

repo=$(cd "$(dirname "$0")" && pwd)
share="${XDG_DATA_HOME:-$HOME/.local/share}/ascii-aquarium"

cargo build --release --locked --manifest-path "$repo/Cargo.toml"
# install(1) replaces the file rather than rewriting it, so a running
# screensaver keeps its copy.
install -Dm755 "$repo/target/release/ascii-aquarium" "$HOME/.local/bin/ascii-aquarium"
echo "built ~/.local/bin/ascii-aquarium"

mkdir -p "$share"
ln -sfn "$repo/foot" "$share/foot"
echo "linked $share/foot -> $repo/foot"
