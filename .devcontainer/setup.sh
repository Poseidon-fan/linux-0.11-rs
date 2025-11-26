#!/bin/bash

git config --global --add safe.directory $(pwd)

# Install pre-commit hooks
pre-commit install

# Copy .vscode config to the top folder
rm -rf .vscode && cp -r .devcontainer/vscode-config .vscode

# Set global toolchain
rustup default nightly-2025-01-27
