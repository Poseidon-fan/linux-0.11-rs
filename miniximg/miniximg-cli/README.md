# miniximg-cli

> Command-line tool for building, inspecting, editing, and interactively
> exploring [Minix v1 filesystem][minix] images.

`miniximg-cli` ships the `miniximg` binary. It glues
[`miniximg`](https://crates.io/crates/miniximg) (the core library) and
[`miniximg-shell`](https://crates.io/crates/miniximg-shell) (the
interactive REPL) into a single tool.

```bash
cargo install miniximg-cli
```

[minix]: https://en.wikipedia.org/wiki/MINIX_file_system

## At a glance

| Subcommand       | One-liner                                                     |
|------------------|---------------------------------------------------------------|
| `build`          | Create a new image from a TOML manifest or `--entry` flags.   |
| `inspect`        | Print a human-readable summary of an existing image.          |
| `check`          | Validate an image and exit non-zero on issues.                |
| `ls`, `tree`, `cat`, `stat` | Read-only browsing.                                |
| `get`, `put`     | Transfer one file between the host and an image.              |
| `mkdir`, `mknod`, `rm`, `rmdir`, `mv`, `ln` | Direct mutating ops.        |
| `shell`          | Drop into an interactive REPL against the image.              |

Run `miniximg <subcommand> --help` for the full set of flags on any one
command.

## Build an image

From a TOML manifest:

```bash
miniximg build --manifest rootfs.toml
```

Or with explicit mappings on the command line:

```bash
miniximg build \
  --output build/rootfs.img \
  --size 4MiB \
  --inode-count 128 \
  --entry kind=tree,source=build/root,target=/,overwrite=true \
  --entry kind=file,source=README.md,target=/etc/motd,overwrite=true
```

### Manifest shape

`build --manifest` expects a TOML file with one `[image]` table and zero
or more `[[mapping]]` entries.

```toml
[image]
output = "build/rootfs.img"
size = "4MiB"
inode_count = 128
default_uid = 0
default_gid = 0
default_file_mode = "0644"
default_dir_mode = "0755"

[[mapping]]
kind = "tree"
source = "build/root"
target = "/"
overwrite = true

[[mapping]]
kind = "file"
source = "README.md"
target = "/etc/motd"
overwrite = true

[[mapping]]
kind = "dir"
target = "/var/log"
mode = "0755"

[[mapping]]
kind = "char-device"
target = "/dev/tty0"
major = 4
minor = 0
mode = "0666"
```

Supported mapping kinds: `file`, `tree`, `dir`, `block-device`,
`char-device`.

## Inspect and validate

```bash
miniximg inspect build/rootfs.img
miniximg check   build/rootfs.img
```

## Read-only browsing

```bash
miniximg ls   build/rootfs.img /
miniximg tree build/rootfs.img /
miniximg cat  build/rootfs.img /etc/motd
miniximg stat build/rootfs.img /bin/init
miniximg get  build/rootfs.img /etc/motd --output out/motd
```

## Mutating operations

```bash
miniximg mkdir build/rootfs.img /usr/share
miniximg mknod build/rootfs.img /dev/tty0 --kind char --major 4 --minor 0
miniximg put   build/rootfs.img host/message.txt /usr/share/message --overwrite
miniximg mv    build/rootfs.img /usr/share/message /usr/share/motd
miniximg ln    build/rootfs.img /usr/share/motd /etc/motd
miniximg rm    build/rootfs.img /usr/share/motd
miniximg rmdir build/rootfs.img /usr/share
```

## Interactive shell

For ad-hoc exploration and editing, `shell` is much faster than chaining
individual subcommands.

```bash
miniximg shell build/rootfs.img            # read-write
miniximg shell --readonly build/rootfs.img # safe browsing
```

```text
(rootfs.img) /> ls /etc
group  hostname  motd  passwd  profile  rc
(rootfs.img) /> edit /etc/motd           # opens $EDITOR, writes back on save
(rootfs.img) /> put @./local-fix.sh /usr/local/bin/fix
(rootfs.img) /> diff /etc/motd @./old-motd
```

See [`miniximg-shell`](https://crates.io/crates/miniximg-shell) for the
full command list, prompt syntax, and the `@`-prefix convention.

## License

MIT. See [LICENSE](../../LICENSE) in the workspace root.
