# miniximg-shell

> Interactive REPL for inspecting and editing
> [Minix v1 filesystem][minix] images.

`miniximg-shell` powers the `miniximg shell` subcommand in
[`miniximg-cli`](https://crates.io/crates/miniximg-cli). Use this crate
directly only if you want to embed the REPL in another tool; the usual
way to reach it is the CLI.

```bash
cargo install miniximg-cli
miniximg shell disk.minix
```

[minix]: https://en.wikipedia.org/wiki/MINIX_file_system

## Why a REPL

For one-off questions, the individual `miniximg ls / cat / put` commands
are fine. As soon as you start poking around — `cd`, then `ls`, then
`cat`, then maybe `edit` — the cost of re-opening the image and
re-typing the path on every command adds up. The shell loads the image
once and keeps two working directories (image-side and host-side) so
both feel close at hand.

## Path convention

By default, every path refers to the image. Prefix with `@` to mean
"this is a host path" instead.

```text
(disk.minix) /> ls /etc                # image path
(disk.minix) /> cat @./README.md       # host path
(disk.minix) /> put @./fix.sh /usr/local/bin/fix
(disk.minix) /> diff /etc/motd @./old-motd
```

`~` is expanded for host paths (`@~/.config/foo`). The two cwds are
independent — `cd` changes the image side, `lcd` changes the host side.

## Command list

| Group         | Commands                                                          |
|---------------|-------------------------------------------------------------------|
| Browse        | `ls`, `cd`, `cat`, `stat`, `tree`, `pwd`                          |
| Mutate (image)| `mkdir`, `rmdir`, `rm`, `mv`, `ln`, `touch`, `mknod`, `chmod`, `edit` |
| Host          | `lls`, `lcd`, `lpwd`                                              |
| Cross-fs      | `put`, `get`, `cp`, `diff`                                        |
| Meta          | `info`, `fsck`, `sync`, `clear`, `help`, `exit`                   |

Run `help` inside the shell for one-liners on each command, or
`help <name>` for usage details.

## Behaviour notes

- **Persistence**: every mutating command auto-commits to the underlying
  image file before returning. There's no separate `save` step.
- **`--readonly`**: opens the image read-only and refuses every mutating
  command with a clear message. The prompt grows a `:ro` tag so you
  can't forget.
- **`edit`**: spills the image file into a host temp file, invokes
  `$EDITOR` (then `$VISUAL`, then `vi`), and writes the buffer back if
  the editor exits successfully and the content actually changed.
- **`diff`**: works in any direction — image/image, host/host, or
  mixed. Lines are diffed via LCS.
- **History**: persisted to `$XDG_DATA_HOME/miniximg/history` by
  default; pass `--history none` for ephemeral or `--history PATH` for
  a custom location.
- **Tab completion**: bash-style (first TAB extends to longest common
  prefix, second TAB lists candidates). Works for commands, image
  paths, and `@`-prefixed host paths.
- **Key bindings**: rustyline defaults — `Ctrl-A`/`Ctrl-E` jump to
  start/end of line, `Ctrl-U`/`Ctrl-K` kill to start/end, ↑/↓ walk the
  history, `Ctrl-C` cancels the current line, `Ctrl-D` on an empty line
  exits.

## Embedding

```rust,no_run
use miniximg_shell::{run, ShellOptions};
use std::path::PathBuf;

run(ShellOptions {
    image: PathBuf::from("disk.minix"),
    readonly: false,
    history: None,
})?;
# Ok::<(), anyhow::Error>(())
```

## License

MIT. See [LICENSE](../../LICENSE) in the workspace root.
