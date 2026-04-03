# miniximg-cli

`miniximg-cli` provides the `miniximg` command-line interface on top of
`miniximg-core`.

The binary keeps parsing, manifest loading, and text rendering in the CLI crate
while delegating all filesystem logic to the core library.

## Commands

### Build

Create a new image from either a TOML manifest or repeated `--entry` flags.

```bash
miniximg build --manifest rootfs.toml
```

```bash
miniximg build \
  --output build/rootfs.img \
  --size 4MiB \
  --inode-count 128 \
  --entry kind=tree,source=build/root,target=/,overwrite=true \
  --entry kind=file,source=README.md,target=/etc/motd,overwrite=true
```

### Inspect And Validate

```bash
miniximg inspect build/rootfs.img
miniximg check build/rootfs.img
```

### Read-Only Image Access

```bash
miniximg ls build/rootfs.img /
miniximg tree build/rootfs.img /
miniximg stat build/rootfs.img /bin/init
miniximg cat build/rootfs.img /etc/motd
miniximg get build/rootfs.img /etc/motd --output out/motd
```

### Mutating Image Operations

```bash
miniximg mkdir build/rootfs.img /usr/share
miniximg mknod build/rootfs.img /dev/tty0 --kind char --major 4 --minor 0
miniximg put build/rootfs.img host/message.txt /usr/share/message --overwrite
miniximg mv build/rootfs.img /usr/share/message /usr/share/motd
miniximg ln build/rootfs.img /usr/share/motd /etc/motd
miniximg rm build/rootfs.img /usr/share/motd
miniximg rmdir build/rootfs.img /usr/share
```

## Manifest Shape

`build --manifest` expects a TOML file with one `[image]` table and zero or
more `[[mapping]]` entries.

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

Supported mapping kinds:

- `file`
- `tree`
- `dir`
- `block-device`
- `char-device`

## Development

Run tests:

```bash
cargo test
```

Run Clippy:

```bash
cargo clippy --all-targets --all-features -- -D warnings
```
