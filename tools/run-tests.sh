#!/usr/bin/env bash
#
# Run the end-to-end test suite from the repo root.
#
# Picks a kernel image and a disk image, then invokes the `ktest`
# runner (installed on PATH by .devcontainer/setup.sh). All extra
# arguments are forwarded — e.g. `--suite=shell`,
# `--test-set=shell.basic`, `--disable-reboot`.
#
# Usage:
#   tools/run-tests.sh                              # run everything
#   tools/run-tests.sh --suite=shell                   # one suite
#   tools/run-tests.sh --test-set=shell.basic          # one test
#   KERNEL=kernel/Image-console-release tools/run-tests.sh
#   IMAGE=disk.img tools/run-tests.sh --disable-reboot

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

KERNEL="${KERNEL:-$REPO_ROOT/kernel/Image-console-debug}"
IMAGE="${IMAGE:-$REPO_ROOT/disk.img}"

if [ ! -f "$KERNEL" ]; then
    echo "run-tests.sh: kernel image not found: $KERNEL" >&2
    echo "  build it with: make -C kernel Image-console-debug" >&2
    exit 1
fi
if [ ! -f "$IMAGE" ]; then
    echo "run-tests.sh: disk image not found: $IMAGE" >&2
    echo "  build it with: tools/build-disk.sh" >&2
    exit 1
fi

# ktest is installed on PATH by .devcontainer/setup.sh
# (cargo install --path ./ktest).
if ! command -v ktest >/dev/null 2>&1; then
    echo "run-tests.sh: ktest not on PATH" >&2
    echo "  install it with: cargo install --path ./ktest" >&2
    exit 1
fi

cd "$REPO_ROOT"
exec ktest \
    --kernel "$KERNEL" \
    --image "$IMAGE" \
    --suites-root "$REPO_ROOT/ktest/suites" \
    "$@"
