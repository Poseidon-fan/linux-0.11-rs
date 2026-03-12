# mbrkit

`mbrkit` is a command-line tool for building, inspecting, extracting, and
verifying MBR-backed disk images.

It is intended for workflows where you already have raw partition payloads and
need a small tool to assemble or analyze a complete disk image.

## Features

- pack one or more raw payloads into an MBR disk image
- inspect MBR metadata in text or JSON format
- extract a primary partition back into a raw image
- verify bounds, overlaps, signatures, boot flags, and warning conditions
- accept both human-friendly partition aliases and numeric partition type values

## MBR Layout

The current implementation uses the Windows NT style MBR prefix layout:

- `0..440`: bootstrap code
- `440..444`: disk signature
- `444..446`: reserved bytes
- `446..510`: primary partition table
- `510..512`: `0x55AA`

## Installation

From crates.io:

```bash
cargo install mbrkit
```

From source:

```bash
git clone https://github.com/Poseidon-fan/linux-0.11-rs.git
cd linux-0.11-rs/mbrkit
cargo install --path .
```

## Quick Start

Pack a raw payload into a disk image:

```bash
mbrkit pack \
  --output build/disk.img \
  --disk-size 32MiB \
  --disk-signature 0x12345678 \
  --partition file=build/rootfs.img,type=minix,bootable,start=2048,size=4MiB
```

Inspect a disk image:

```bash
mbrkit inspect build/disk.img
mbrkit inspect build/disk.img --format json
```

Extract the first partition:

```bash
mbrkit extract build/disk.img --partition 1 --output out/rootfs.img
```

Verify a disk image:

```bash
mbrkit verify build/disk.img
mbrkit verify build/disk.img --format json --strict
```

## Command Reference

### `pack`

Create a new disk image from either explicit CLI flags or a TOML manifest.

If `--manifest` is present, the layout-related flags must not be used together
with it.

#### `pack` options

| Option | Required | Default | Notes |
| --- | --- | --- | --- |
| `--manifest <FILE>` | No | none | Read the full disk layout from a TOML file. |
| `-o, --output <FILE>` | Yes unless `--manifest` is used | none | Output disk image path. |
| `--disk-size <SIZE>` | Yes unless `--manifest` is used | none | Final logical disk size, for example `32MiB` or `64M`. |
| `--boot-code <FILE>` | No | zero-filled 440-byte bootstrap code | Optional bootstrap code image. The file must be at most 440 bytes. |
| `--disk-signature <HEX_OR_DECIMAL>` | No | `0` | Accepts hexadecimal like `0x12345678` or decimal like `305419896`. |
| `--align <SECTORS>` | No | `2048` | Used for automatic partition placement. |
| `--partition <SPEC>` | Yes unless `--manifest` is used; repeatable | none | Partition declaration in `key=value` form. |
| `--dry-run` | No | `false` | Print the resolved layout without writing the disk image. |
| `--force` | No | `false` | Overwrite an existing output image. |

#### `--partition` spec

Each partition spec is a comma-separated list:

```text
file=...,type=...,bootable,start=...,size=...
```

Supported keys:

| Key | Required | Default | Notes |
| --- | --- | --- | --- |
| `file` | Yes | none | Path to the source raw image. |
| `type` | No | `linux` (`0x83`) | Accepts aliases, hexadecimal, or decimal values. |
| `bootable` | No | `false` | Bare flag. If present, the active bit is set. |
| `start` | No | automatic | If omitted, the partition is placed using `--align`. |
| `size` | No | source file size rounded up to a sector | If present, it must not be smaller than the source file. |

#### Supported `type` aliases

`mbrkit` currently understands these common aliases:

- `empty`
- `fat12`
- `fat16_small`
- `extended`
- `fat16`
- `ntfs`
- `fat32`
- `fat32_lba`
- `fat16_lba`
- `extended_lba`
- `minix`
- `linux_swap`
- `linux`

It also accepts:

- hexadecimal values such as `0x81`, `0x83`, or `0x0f`
- decimal values such as `129`, `131`, or `15`

#### Size strings

Size values in `--disk-size`, `size=...`, and TOML manifests accept:

- bytes: `512`, `512B`
- decimal units: `4K`, `4M`, `1G`
- binary units: `4KiB`, `4MiB`, `1GiB`

#### Manifest example

```toml
output = "build/disk.img"
disk_size = "32MiB"
boot_code = "build/mbr.bin"
disk_signature = "0x12345678"
align_lba = 2048

[[partition]]
file = "build/rootfs.img"
type = "minix"
bootable = true
start_lba = 2048
size = "4MiB"

[[partition]]
file = "build/data.img"
type = "linux"
start_lba = 12288
size = "8MiB"
```

Then run:

```bash
mbrkit pack --manifest build/disk.toml
```

### `inspect`

Read an existing disk image and print the decoded MBR contents.

#### `inspect` arguments

| Argument or option | Required | Default | Notes |
| --- | --- | --- | --- |
| `<DISK>` | Yes | none | Path to the disk image to inspect. |
| `--format <table\|json>` | No | `table` | Choose human-readable text or structured JSON output. |

#### `inspect` behavior

- If the image is at least 512 bytes long, `inspect` tries to decode and report
  the MBR even if the signature is invalid.
- Invalid signatures are reported as diagnostics instead of causing the command
  to fail.
- If the image is smaller than one sector, the command fails.

### `extract`

Extract one primary partition from an existing disk image.

#### `extract` arguments

| Argument or option | Required | Default | Notes |
| --- | --- | --- | --- |
| `<DISK>` | Yes | none | Path to the source disk image. |
| `--partition <INDEX>` | Yes | none | One-based partition number in the range `1..=4`. |
| `-o, --output <FILE>` | Yes | none | Output path for the extracted raw image. |
| `--force` | No | `false` | Overwrite an existing output file. |

#### `extract` behavior

- The extracted file size matches the full partition size recorded in the MBR.
- If the original payload was smaller than the reserved partition space, the
  output includes trailing zero padding.
- Empty partition slots and out-of-bounds partitions are rejected.

### `verify`

Validate an existing disk image and return a meaningful exit code.

#### `verify` arguments

| Argument or option | Required | Default | Notes |
| --- | --- | --- | --- |
| `<DISK>` | Yes | none | Path to the disk image to validate. |
| `--format <table|json>` | No | `table` | Choose human-readable text or structured JSON output. |
| `--strict` | No | `false` | Promote warnings to failures. |

#### `verify` behavior

`verify` always checks at least:

- MBR signature validity
- partition bounds
- partition overlap
- boot flag validity

In non-strict mode:

- warnings are reported but do not fail the command

In strict mode:

- warnings also cause a non-zero exit status
- current warnings include unknown partition type, zero disk signature, and
  multiple active partitions

## Output Formats

`inspect` and `verify` support:

- `--format table`: human-friendly text output
- `--format json`: machine-friendly structured output

This makes `mbrkit` suitable for both interactive use and CI scripting.

## Limitations

The current release intentionally keeps the scope tight:

- only 512-byte sectors are supported
- only primary MBR partitions are supported
- at most four partitions can be defined
- extended partitions are not implemented yet
- GPT is not implemented

## Development

Run the test suite:

```bash
cargo test
```

## License

`mbrkit` is distributed under the MIT license.
