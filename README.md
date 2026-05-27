# linux-0.11-rs

> A modern Rust rewrite of the Linux 0.11 kernel — boots on `i386` in
> QEMU, runs a self-hosted Unix-style userland.

```sh
$ cd kernel && make run
…
Welcome to linux-0.11-rs.
https://github.com/Poseidon-fan/linux-0.11-rs
(login: 0 @ linux-rs :: Wed May 27 09:35:43 UTC 2026)
[root@linux-rs /usr/root]# ls /bin | wc -l
69
```

`linux-0.11-rs` rebuilds the 1991 Linux 0.11 kernel from scratch in modern
Rust. It keeps the original system's semantics — what it does — while
rethinking how it's expressed: stronger types, clearer module boundaries,
idiomatic abstractions everywhere. The kernel boots on emulated `i386`
hardware, runs a full `init` → shell → coreutils stack, and ships with the
tooling to build your own bootable image in one command.

## ✨ Highlights

- **A kernel with most of what Linux 0.11 had** — processes, virtual memory
  with demand paging and CoW fork, the Minix v1 filesystem, ATA disk
  driver, VGA + PS/2 console, 8250 serial console, TTY layer, signals, and
  the complete syscall table.
- **A Rust user-space "std"** — `user_lib` mirrors the public shape of
  `std::{fs, io, path, env, process, time}` so user programs read like
  ordinary Rust, not like syscall plumbing.
- **A real userland** — 80+ coreutils plus a hand-written POSIX-subset
  shell (`sh`) with pipelines, control flow, functions, glob, command and
  arithmetic substitution, and an interactive line editor with Tab
  completion and history.
- **One-command images** — `tools/build-disk.sh` compiles every user
  program, lays them out into a Unix-style filesystem, and packs the
  result into a bootable disk image.
- **Companion image tools** — [`mbrkit`](./mbrkit) and
  [`miniximg`](./miniximg) are standalone crates, useful on their own for
  any project that touches MBR or Minix v1 images.
- **Devcontainer included** — clone, open in VS Code, hit "Reopen in
  Container", run `make run`.

## 🚀 Quick start

```bash
# Build every user program, lay out /etc /dev /bin, pack into disk.img.
tools/build-disk.sh

# Build the kernel and boot it in QEMU.
cd kernel && make run            # VGA console
cd kernel && make run-console    # serial console (-nographic)
```

You'll land at a shell prompt in `/usr/root`. Try:

```sh
ls /bin                              # browse what's installed
echo $((1 + 2 * 3))                  # arithmetic expansion
for f in /etc/*; do echo $f; done    # for loop + glob
fact() { if [ $1 -le 1 ]; then echo 1; else echo $(($1 * $(fact $(($1-1))))); fi; }
fact 7                               # → 5040
ec<TAB>                              # completes to `echo `
↑                                    # walks command history
```

Outside a devcontainer you'll also need a recent Rust nightly (pinned in
`rust-toolchain.toml`), `qemu-system-i386`, the `x86_64-linux-gnu-*`
cross-binutils, and the two local image tools:

```bash
cargo install --path mbrkit
cargo install --path miniximg/miniximg-cli
```

## 🗂️ Repository layout

```
kernel/              The kernel itself
user_lib/            std-style user-space library
user_lib_macros/     proc-macro: #[user_lib::main]
user_program/        ~80 coreutils + the `sh` shell
mbrkit/              MBR disk-image CLI (also on crates.io)
miniximg/            Minix-fs image CLI and library
platform/            custom i386-unknown-none target spec
rootfs/              disk-image content template (/etc, /usr/root, …)
tools/               developer scripts (build-disk.sh and friends)
tutorial/            mdbook walkthrough (work in progress)
.devcontainer/       ready-to-use dev environment
```

## 🛣️ Project status

- **Kernel** — substantially feature-complete relative to Linux 0.11.
  Floppy support and x87 emulation are intentionally out of scope; ongoing
  work is on polish and tooling.
- **User library** — covers what the shell and coreutils need today.
- **Userland** — usable for real interactive work.
- **Tutorial** — early draft; the long-term plan is a complete
  build-from-scratch walkthrough.

## 📚 Tutorial

```bash
cargo install mdbook
cd tutorial && mdbook serve --open
```

## 🙏 Acknowledgements

- Thanks to [yuan-xy/Linux-0.11](https://github.com/yuan-xy/Linux-0.11) for
  providing the original Linux 0.11 kernel source used as an important
  reference during development.
- Many parts of this project were also inspired by or implemented with
  reference to [rcore-os/rCore-Tutorial-v3](https://github.com/rcore-os/rCore-Tutorial-v3).

## 📄 License

See [LICENSE](./LICENSE).
