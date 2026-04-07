# Linux-0.11-rs

> A modern Rust rewrite of the Linux 0.11 kernel, designed to boot on bare-metal `i386` in QEMU.

`linux-0.11-rs` is a from-scratch rewrite of the Linux 0.11 kernel in modern Rust.
It preserves the overall architecture and semantics of the original system while
rebuilding the implementation with stronger abstractions, clearer module
boundaries, and a more maintainable codebase.

The kernel can boot on emulated `i386` hardware under QEMU and already includes
substantial support for process management, memory management, filesystems,
TTY/console I/O, and ATA hard disk access.

## ✨ Features

### Modern Rust, not a line-by-line translation

This project is not a mechanical port of the original C and assembly source.
Instead, it keeps the spirit and behavior of Linux 0.11 while refactoring core
subsystems into more idiomatic Rust designs.

- **Physical page-frame and address-space abstractions**: the kernel models
physical frames and process address spaces explicitly, which makes memory
ownership clearer and allows lifetimes to be managed automatically.
- **Kernel synchronization primitives**: the kernel provides Rust-style
building blocks such as `Cell`-like and `Mutex`-like primitives adapted for
kernel use.
- **Kernel heap**: the project includes a working kernel heap for dynamic
allocation inside the kernel.

### 🧰 Image tooling included

This repository also includes two companion tools for working with bootable
disk images:

- `[mbrkit](./mbrkit)`: a small CLI for building, inspecting, extracting, and
verifying MBR disk images
- `[miniximg](./miniximg)`: a Minix filesystem image tool tailored to the
filesystem format currently supported by this kernel

Together, they make it much easier to prepare disk images for development,
testing, and experimentation.

### 🐳 Ready-to-use devcontainer environment

The repository ships with a complete devcontainer setup for VS Code-compatible
editors. It installs the Rust toolchain, QEMU, cross-binutils, and the local
image tools so that the kernel can be built and run with minimal manual setup.

### 📚 Tutorial included

The project also contains a tutorial workspace that is intended to grow into a
step-by-step guide for building this kernel from scratch in Rust.

## 🚀 Development Setup

The recommended workflow is to use a VS Code-like IDE with the Dev Containers
extension. This setup has already been verified for developing and running the
kernel in this repository.

### 1. Run and debug the kernel

The kernel expects a hard disk image named `rootfs.img` in the repository root.

You can build your own image with `user_program`, `miniximg`, and `mbrkit`, but
for now the easiest path is to start from a prebuilt Linux 0.11-compatible
image. Since the current `user_program` workspace is still evolving, using a
known-good image is the recommended way to get started.

Recommended image:

- [https://github.com/yuan-xy/Linux-0.11/blob/master/hdc-0.11.img](https://github.com/yuan-xy/Linux-0.11/blob/master/hdc-0.11.img)

Example download command from the repository root:

```bash
curl -L https://raw.githubusercontent.com/yuan-xy/Linux-0.11/master/hdc-0.11.img -o rootfs.img
```

Then run the kernel:

```bash
cd kernel
make run
```

### 2. Build and preview the tutorial

Install `mdbook` first:

```bash
cargo install mdbook
```

Then serve the tutorial locally:

```bash
cd tutorial
mdbook serve --open
```

## 🗂️ Repository Layout

- `[kernel](./kernel)`: kernel source code
- `[user_lib](./user_lib)`: user-space support library and syscall wrappers
- `[user_program](./user_program)`: user-space programs and experiments, still
evolving
- `[mbrkit](./mbrkit)`: MBR disk image tool
- `[miniximg](./miniximg)`: Minix filesystem image tool
- `[tutorial](./tutorial)`: tutorial and book sources
- `[ref](./ref)`: original Linux 0.11 source kept for reference

## 🛣️ Project Status

This project is under active development and maintenance.

### Kernel

Most major Linux 0.11-era functionality has already been implemented.
At the moment, the main missing pieces are:

- math coprocessor support
- floppy driver support
- serial driver support

Near-term work will focus on improving and polishing the current
implementation rather than only expanding the feature list.

### User programs

The long-term plan is to build a more complete Unix-style userland on top of
`user_lib`.

Current status: **TBD / in progress**

### Tutorial

The tutorial is planned to become a full walkthrough for building this kernel
from scratch.

Current status: **TBD / in progress**

## 🙏 Acknowledgements

- Thanks to [`yuan-xy/Linux-0.11`](https://github.com/yuan-xy/Linux-0.11) for
  providing the original Linux 0.11 kernel source used as an important
  reference during development.
- Many parts of this project were also inspired by or implemented with
  reference to [`rcore-os/rCore-Tutorial-v3`](https://github.com/rcore-os/rCore-Tutorial-v3).
