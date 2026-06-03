#!/usr/bin/env bash
#
# Run the end-to-end test suite from the repo root.
#
# By default the kernel image is rebuilt before every run so tests always
# exercise the latest code.  Pass `--without-rebuild` to skip the rebuild.
#
# Extra arguments are forwarded to `ktest` — e.g. `--suite=shell`,
# `--test-set=shell.basic`, `--disable-reboot`.
#
# Usage:
#   tools/run-tests.sh                              # rebuild + run everything
#   tools/run-tests.sh --without-rebuild               # skip rebuild
#   tools/run-tests.sh --suite=shell                   # one suite
#   tools/run-tests.sh --test-set=shell.basic          # one test
#   KERNEL=kernel/Image-console-release tools/run-tests.sh
#   IMAGE=disk.img tools/run-tests.sh --disable-reboot

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

KERNEL="${KERNEL:-$REPO_ROOT/kernel/Image-console-debug}"
IMAGE="${IMAGE:-$REPO_ROOT/disk.img}"

REBUILD=true
KTEST_ARGS=()
for arg in "$@"; do
    if [ "$arg" = "--without-rebuild" ]; then
        REBUILD=false
    else
        KTEST_ARGS+=("$arg")
    fi
done

if $REBUILD; then
    echo "run-tests.sh: rebuilding kernel image..."
    make -C "$REPO_ROOT/kernel" "$(basename "$KERNEL")"
fi

if [ ! -f "$KERNEL" ]; then
    echo "run-tests.sh: kernel image not found: $KERNEL" >&2
    echo "  build it with: make -C kernel $(basename "$KERNEL")" >&2
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
    "${KTEST_ARGS[@]}"
