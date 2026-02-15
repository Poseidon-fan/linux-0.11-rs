#!/usr/bin/env bash
set -e

BIN_PATH="target/i386-unknown-none/debug/linux_rs"
PORT=1234
MODULE_NAME=""

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
        -m|--module)
            MODULE_NAME="$2"
            shift 2
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

if [[ -z "${MODULE_NAME}" ]]; then
    MODULE_NAME="$(basename "${BIN_PATH}")"
fi

# Run LLDB
lldb \
    -o "target create ${BIN_PATH}" \
    -o "gdb-remote ${PORT}" \
    -o "target modules load --file ${MODULE_NAME} --slide 0x0"
