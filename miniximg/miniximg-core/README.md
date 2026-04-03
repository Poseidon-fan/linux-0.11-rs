# miniximg-core

`miniximg-core` is the reusable library crate behind the `miniximg` tool.

It implements the Minix filesystem image format expected by the current
`linux-0.11-rs` kernel and keeps the actual filesystem semantics out of the CLI
layer.

## Scope

The crate is intentionally limited to the Minix layout currently supported by
the kernel:

- 1 KiB logical blocks
- 14-byte directory entry names
- Minix v1 magic `0x137F`
- `log_zone_size == 0`
- regular files, directories, hard links, and read-only support for special
  inode types already present in an image

The crate does not implement symlinks, image repair, recursive delete, or
device-node creation.

## Main Types

- `MinixFileSystem<S>`: the stateful image object over any
  `Read + Write + Seek` backend
- `CreateImageOptions`: parameters for creating an empty image
- `CreateNodeOptions`: metadata used when creating files or directories
- `BuildRequest`, `ImageSpec`, and `BuildEntry`: shared DTOs used by the CLI
  and any future automation

## Capabilities

- create a fresh filesystem image
- open an existing image
- inspect and validate the filesystem
- create directories recursively
- create block and character device inodes
- read and write regular files
- remove files and empty directories
- rename paths
- create hard links
- build an image from host file and directory mappings

## Module Layout

- `layout`: on-disk structures and serialization
- `bitmap`: inode and zone allocation helpers
- `path`: absolute image-path validation and normalization
- `fs`: the `MinixFileSystem` implementation
- `build`: host-to-image mapping DTOs and build orchestration, including
  device-node mappings
- `report`: inspect/check output models
- `error`: shared error type

## Testing

Run the library tests from the `miniximg` workspace:

```bash
cargo test -p miniximg
```

Run Clippy with warnings denied:

```bash
cargo clippy -p miniximg --all-targets --all-features -- -D warnings
```
