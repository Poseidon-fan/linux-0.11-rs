#!/bin/bash

git config --global --add safe.directory $(pwd)

# Install pre-commit hooks
pre-commit install

# Copy .vscode config to the top folder
rm -rf .vscode && cp -r .devcontainer/vscode-config .vscode

# Install the repo's built-in developer tools (mbrkit, miniximg, ktest)
./tools/install-tools.sh
