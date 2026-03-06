FROM mcr.microsoft.com/devcontainers/rust:latest

RUN apt-get update && apt-get install -y \
    python3 \
    python3-pip \
    qemu-system \
    binutils-x86-64-linux-gnu

RUN rustup toolchain install nightly-2025-01-27 \
    --component rust-src \
    --component llvm-tools-preview \
    --component rustfmt \
    --component clippy

RUN pip install pre-commit --break-system-packages
