#!/usr/bin/env bash
#
# Run the end-to-end test suite from the repo root.
#
# Builds the runner crate (if needed), picks a kernel image and a disk
# image, then invokes the runner. All extra arguments are forwarded —
# e.g. `--suite=shell`, `--test-set=shell.basic`, `--disable-reboot`.
#
# Usage:
#   tools/run-tests.sh                              # run everything
#   tools/run-tests.sh --suite=shell                   # one suite
#   tools/run-tests.sh --test-set=shell.basic          # one test
#   KERNEL=kernel/Image-console-debug tools/run-tests.sh
#   IMAGE=disk.img tools/run-tests.sh --disable-reboot

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

KERNEL="${KERNEL:-$REPO_ROOT/kernel/Image-console-release}"
IMAGE="${IMAGE:-$REPO_ROOT/disk.img}"

if [ ! -f "$KERNEL" ]; then
    echo "run-tests.sh: kernel image not found: $KERNEL" >&2
    echo "  build it with: make -C kernel Image-console-release" >&2
    exit 1
fi
if [ ! -f "$IMAGE" ]; then
    echo "run-tests.sh: disk image not found: $IMAGE" >&2
    echo "  build it with: tools/build-disk.sh" >&2
    exit 1
fi

echo "==> Building runner"
cargo build --quiet --manifest-path "$REPO_ROOT/ktest/Cargo.toml"

RUNNER="$REPO_ROOT/ktest/target/debug/ktest"
if [ ! -x "$RUNNER" ]; then
    # cargo with a non-default manifest still places artefacts in the
    # workspace's target dir — fall back to that location.
    RUNNER="$REPO_ROOT/target/debug/ktest"
fi

cd "$REPO_ROOT"
exec "$RUNNER" \
    --kernel "$KERNEL" \
    --image "$IMAGE" \
    --suites-root "$REPO_ROOT/ktest/suites" \
    "$@"
