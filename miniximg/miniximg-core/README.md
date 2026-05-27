# miniximg

> A Rust library for building, inspecting, and editing
> [Minix v1 filesystem][minix] images.

`miniximg` is the engine behind the `miniximg` CLI (in
[`miniximg-cli`](https://crates.io/crates/miniximg-cli)) and the
interactive shell (in
[`miniximg-shell`](https://crates.io/crates/miniximg-shell)). Use it
directly when you want programmatic access to Minix v1 images without
spawning a subprocess.

```toml
[dependencies]
miniximg = "1.0"
```

[minix]: https://en.wikipedia.org/wiki/MINIX_file_system

## What it does

The crate implements the exact Minix v1 layout that the `linux-0.11-rs`
kernel reads:

- 1 KiB logical blocks
- 14-byte directory-entry names
- Minix v1 magic `0x137F`
- `log_zone_size == 0`
- regular files, directories, hard links
- block / character device inodes
- read-only access to FIFO inodes that happen to be present

It deliberately does **not** model symlinks, image repair, or recursive
delete — features the kernel doesn't expect.

## What you can do with it

- create a fresh empty image
- open an existing image
- inspect (`inspect`) or validate (`check`) it
- list, stat, read, and walk inodes
- create directories (recursively) and device nodes
- write, rename, and unlink regular files
- create hard links and remove empty directories
- build an image from a structured list of host→image mappings

## A short tour

```rust,no_run
use miniximg::{CreateNodeOptions, MinixFileSystem};
use std::fs::OpenOptions;

let file = OpenOptions::new().read(true).write(true).open("disk.minix")?;
let mut fs = MinixFileSystem::open(file)?;

// Read a file from the image.
let motd = fs.read_file_at_path("/etc/motd")?;
println!("{}", String::from_utf8_lossy(&motd));

// Walk a directory.
for entry in fs.list_path("/etc")? {
    println!("{} (inode {})", entry.name, entry.metadata.inode_number);
}

// Drop a new file in.
let opts = CreateNodeOptions { mode: 0o644, uid: 0, gid: 0, mtime: 0 };
let parents = CreateNodeOptions { mode: 0o755, ..opts };
fs.write_file_at_path("/etc/banner", b"hello\n", &opts, true, &parents)?;
fs.flush()?;
# Ok::<(), miniximg::MinixError>(())
```

## Main types

| Type                      | Purpose                                          |
|---------------------------|--------------------------------------------------|
| `MinixFileSystem<S>`      | Stateful image, generic over any `Read+Write+Seek` |
| `CreateImageOptions`      | Parameters for creating an empty image           |
| `CreateNodeOptions`       | Owner / mode / mtime used when creating inodes   |
| `BuildRequest`, `BuildEntry`, `FileMapping`, … | DTOs for declarative image construction |
| `InspectReport`, `CheckReport`, `NodeMetadata` | Read-only views returned by inspect / check / stat |
| `MinixError`              | Single error type used throughout the crate      |

## Module layout

- `layout` — on-disk structures and serialisation primitives
- `bitmap` — inode and zone allocation
- `path` — absolute image-path validation and normalisation
- `fs` — the `MinixFileSystem` implementation
- `build` — host-to-image mapping DTOs and the build driver
- `report` — inspect / check output models
- `error` — shared error type

## License

MIT. See [LICENSE](../../LICENSE) in the workspace root.
