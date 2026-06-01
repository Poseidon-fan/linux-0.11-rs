#!/usr/bin/env bash
#
# Install the repo's built-in developer tools onto PATH via `cargo install`.
#
# These are the standalone helper crates the build/test scripts expect to
# find on PATH:
#
#   mbrkit    — MBR disk-image CLI        (tools/build-disk.sh)
#   miniximg  — Minix-fs image CLI        (tools/build-disk.sh)
#   ktest     — end-to-end test runner    (tools/run-tests.sh)
#
# Run this once after cloning (the devcontainer does it automatically in
# .devcontainer/setup.sh). Re-run it after changing any of these crates to
# pick up the new build.
#
# Usage:
#   tools/install-tools.sh                 # install all built-in tools
#   tools/install-tools.sh --locked        # forward extra args to cargo install

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

cd "$REPO_ROOT"

for path in mbrkit miniximg/miniximg-cli ktest; do
    echo "==> Installing $(basename "$path")"
    cargo install --path "$path" "$@"
done

echo "==> Done: mbrkit, miniximg, ktest installed on PATH"
