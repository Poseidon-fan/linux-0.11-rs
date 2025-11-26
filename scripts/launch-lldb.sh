#!/usr/bin/env bash
set -e

BIN_PATH="target/i386-unknown-none/debug/linux_rs"
PORT=1234

# Parse flags
while [[ $# -gt 0 ]]; do
    case $1 in
        -p|--port)
            PORT="$2"
            shift 2
            ;;
        -b|--bin)
            BIN_PATH="$2"
            shift 2
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

# Run LLDB
lldb \
    -o "target create ${BIN_PATH}" \
    -o "gdb-remote ${PORT}"
