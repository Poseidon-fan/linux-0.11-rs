#!/bin/bash

git config --global --add safe.directory $(pwd)

# Install pre-commit hooks
pre-commit install

# Set global toolchain
rustup default nightly-2025-01-27