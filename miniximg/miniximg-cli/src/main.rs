//! Command-line entry point for the `miniximg` tool.

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use clap::{Args, Parser, Subcommand, ValueEnum};
use miniximg::build::current_unix_time;
use miniximg::{
    BuildEntry, BuildRequest, CheckReport, CreateNodeOptions, DeviceMapping, DeviceNodeKind,
    DirectoryEntryInfo, DirectoryMapping, FileMapping, ImageSpec, InodeMode, InodeType, MinixError,
    MinixFileSystem, NodeMetadata, TreeEntry, TreeMapping, build_image, device_number,
};
use serde::Deserialize;

/// Parse CLI arguments and execute the selected command.
fn main() {
    if let Err(error) = run() {
        if error.should_print() {
            eprintln!("{error}");
        }
        std::process::exit(error.exit_code());
    }
}

/// The command dispatcher used by `main`.
fn run() -> miniximg::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Build(args) => {
            let request = build_request_from_args(&args)?;
            build_image(&request)?;
        }
        Command::Inspect(args) => {
            let mut image = open_image(&args.image)?;
            print_inspect(&image.inspect()?);
        }
        Command::Check(args) => {
            let mut image = open_image(&args.image)?;
            let report = image.check()?;
            print_check(&report);
            if !report.is_clean() {
                return Err(MinixError::SilentFailure(1));
            }
        }
        Command::Ls(args) => {
            let mut image = open_image(&args.image)?;
            print_listing(&image.list_path(args.path.as_deref().unwrap_or("/"))?);
        }
        Command::Tree(args) => {
            let mut image = open_image(&args.image)?;
            print_tree(&image.tree(args.path.as_deref().unwrap_or("/"))?);
        }
        Command::Cat(args) => {
            let mut image = open_image(&args.image)?;
            let data = image.read_file_at_path(&args.path)?;
            io::stdout().write_all(&data).map_err(|source| {
                MinixError::io(None, "failed to write file contents to stdout", source)
            })?;
        }
        Command::Stat(args) => {
            let mut image = open_image(&args.image)?;
            print_stat(&image.stat(&args.path)?);
        }
        Command::Get(args) => {
            let mut image = open_image(&args.image)?;
            let data = image.read_file_at_path(&args.path)?;
            write_host_file(&args.output, &data, args.force)?;
        }
        Command::Put(args) => {
            let mut image = open_image(&args.image)?;
            let data = fs::read(&args.source).map_err(|source| {
                MinixError::io(
                    Some(args.source.clone()),
                    "failed to read source file",
                    source,
                )
            })?;
            let node_options = CreateNodeOptions {
                mode: parse_optional_mode(args.mode.as_deref())?.unwrap_or(0o644),
                uid: args.uid.unwrap_or(0),
                gid: args.gid.unwrap_or(0),
                mtime: args.mtime.unwrap_or_else(current_unix_time),
            };
            let parent_options = CreateNodeOptions {
                mode: 0o755,
                uid: 0,
                gid: 0,
                mtime: node_options.mtime,
            };
            image.write_file_at_path(
                &args.path,
                &data,
                &node_options,
                args.overwrite,
                &parent_options,
            )?;
            image.flush()?;
        }
        Command::Mkdir(args) => {
            let mut image = open_image(&args.image)?;
            let node_options = CreateNodeOptions {
                mode: parse_optional_mode(args.mode.as_deref())?.unwrap_or(0o755),
                uid: args.uid.unwrap_or(0),
                gid: args.gid.unwrap_or(0),
                mtime: args.mtime.unwrap_or_else(current_unix_time),
            };
            image.mkdir_all(&args.path, &node_options)?;
            image.flush()?;
        }
        Command::Mknod(args) => {
            let mut image = open_image(&args.image)?;
            let node_options = CreateNodeOptions {
                mode: parse_optional_mode(args.mode.as_deref())?.unwrap_or(0o644),
                uid: args.uid.unwrap_or(0),
                gid: args.gid.unwrap_or(0),
                mtime: args.mtime.unwrap_or_else(current_unix_time),
            };
            let parent_options = CreateNodeOptions {
                mode: 0o755,
                uid: 0,
                gid: 0,
                mtime: node_options.mtime,
            };
            image.create_device_at_path(
                &args.path,
                args.kind.into(),
                device_number(args.major, args.minor),
                &node_options,
                &parent_options,
            )?;
            image.flush()?;
        }
        Command::Rm(args) => {
            let mut image = open_image(&args.image)?;
            image.remove_file(&args.path)?;
            image.flush()?;
        }
        Command::Rmdir(args) => {
            let mut image = open_image(&args.image)?;
            image.remove_directory(&args.path)?;
            image.flush()?;
        }
        Command::Mv(args) => {
            let mut image = open_image(&args.image)?;
            image.rename_path(&args.source, &args.target)?;
            image.flush()?;
        }
        Command::Ln(args) => {
            let mut image = open_image(&args.image)?;
            image.link_path(&args.source, &args.target)?;
            image.flush()?;
        }
    }

    Ok(())
}

/// The top-level CLI parser.
#[derive(Debug, Parser)]
#[command(
    name = "miniximg",
    version,
    about = "Build and modify Minix filesystem images"
)]
struct Cli {
    /// The subcommand to execute.
    #[command(subcommand)]
    command: Command,
}

/// The supported top-level subcommands.
#[derive(Debug, Subcommand)]
enum Command {
    /// Build one new Minix image from a manifest or explicit mappings.
    Build(BuildArgs),
    /// Print a high-level summary of one existing image.
    Inspect(ImageOnlyArgs),
    /// Validate one existing image and exit non-zero when issues are found.
    Check(ImageOnlyArgs),
    /// List one path inside the image.
    Ls(ImagePathArgs),
    /// Print a recursive tree for one path inside the image.
    Tree(ImagePathArgs),
    /// Print one image file to stdout.
    Cat(RequiredImagePathArgs),
    /// Print metadata for one image path.
    Stat(RequiredImagePathArgs),
    /// Copy one image file out to the host filesystem.
    Get(GetArgs),
    /// Copy one host file into the image.
    Put(PutArgs),
    /// Create one directory and any missing parents inside the image.
    Mkdir(MkdirArgs),
    /// Create one block or character device inode inside the image.
    Mknod(MknodArgs),
    /// Remove one non-directory path from the image.
    Rm(RequiredImagePathArgs),
    /// Remove one empty directory from the image.
    Rmdir(RequiredImagePathArgs),
    /// Rename one image path.
    Mv(RenameArgs),
    /// Create one hard link inside the image.
    Ln(RenameArgs),
}

/// Arguments shared by commands that only need an image path.
#[derive(Debug, Args)]
struct ImageOnlyArgs {
    /// The image file to open.
    #[arg(value_name = "IMAGE")]
    image: PathBuf,
}

/// Arguments shared by commands that accept an optional image path.
#[derive(Debug, Args)]
struct ImagePathArgs {
    /// The image file to open.
    #[arg(value_name = "IMAGE")]
    image: PathBuf,
    /// The image path to inspect. Defaults to `/`.
    #[arg(value_name = "PATH")]
    path: Option<String>,
}

/// Arguments shared by commands that require an image path.
#[derive(Debug, Args)]
struct RequiredImagePathArgs {
    /// The image file to open.
    #[arg(value_name = "IMAGE")]
    image: PathBuf,
    /// The required image path.
    #[arg(value_name = "PATH")]
    path: String,
}

/// Arguments for `build`.
#[derive(Debug, Args)]
struct BuildArgs {
    /// Read the full build description from a TOML manifest.
    #[arg(
        long,
        value_name = "FILE",
        conflicts_with_all = [
            "output",
            "size",
            "inode_count",
            "default_uid",
            "default_gid",
            "default_mtime",
            "default_file_mode",
            "default_dir_mode",
            "entry",
            "force"
        ]
    )]
    manifest: Option<PathBuf>,
    /// The output image path when using explicit CLI flags.
    #[arg(short, long, value_name = "FILE", required_unless_present = "manifest")]
    output: Option<PathBuf>,
    /// The logical image size, such as `4MiB`.
    #[arg(long, value_name = "SIZE", required_unless_present = "manifest")]
    size: Option<String>,
    /// The inode count reserved in the image.
    #[arg(long, value_name = "COUNT", required_unless_present = "manifest")]
    inode_count: Option<u16>,
    /// The default owner used for created files and directories.
    #[arg(long, value_name = "UID")]
    default_uid: Option<u16>,
    /// The default group owner used for created files and directories.
    #[arg(long, value_name = "GID")]
    default_gid: Option<u8>,
    /// The default modification time used for created files and directories.
    #[arg(long, value_name = "UNIX_SECONDS")]
    default_mtime: Option<u32>,
    /// The default regular-file mode expressed in octal.
    #[arg(long, value_name = "MODE")]
    default_file_mode: Option<String>,
    /// The default directory mode expressed in octal.
    #[arg(long, value_name = "MODE")]
    default_dir_mode: Option<String>,
    /// One repeated `key=value,...` mapping specification.
    #[arg(long, value_name = "SPEC")]
    entry: Vec<String>,
    /// Overwrite an existing output image.
    #[arg(long)]
    force: bool,
}

/// Arguments for `get`.
#[derive(Debug, Args)]
struct GetArgs {
    /// The image file to open.
    #[arg(value_name = "IMAGE")]
    image: PathBuf,
    /// The source file path inside the image.
    #[arg(value_name = "PATH")]
    path: String,
    /// The destination host path.
    #[arg(short, long, value_name = "FILE")]
    output: PathBuf,
    /// Overwrite an existing host output file.
    #[arg(long)]
    force: bool,
}

/// Arguments for `put`.
#[derive(Debug, Args)]
struct PutArgs {
    /// The image file to open.
    #[arg(value_name = "IMAGE")]
    image: PathBuf,
    /// The host file to copy into the image.
    #[arg(value_name = "SOURCE")]
    source: PathBuf,
    /// The destination file path inside the image.
    #[arg(value_name = "PATH")]
    path: String,
    /// The permission bits to store for the new file.
    #[arg(long, value_name = "MODE")]
    mode: Option<String>,
    /// The owner ID to store for the new file.
    #[arg(long, value_name = "UID")]
    uid: Option<u16>,
    /// The group ID to store for the new file.
    #[arg(long, value_name = "GID")]
    gid: Option<u8>,
    /// The modification time to store for the new file.
    #[arg(long, value_name = "UNIX_SECONDS")]
    mtime: Option<u32>,
    /// Overwrite an existing regular file.
    #[arg(long)]
    overwrite: bool,
}

/// Arguments for `mkdir`.
#[derive(Debug, Args)]
struct MkdirArgs {
    /// The image file to open.
    #[arg(value_name = "IMAGE")]
    image: PathBuf,
    /// The directory path to create inside the image.
    #[arg(value_name = "PATH")]
    path: String,
    /// The permission bits stored for created directories.
    #[arg(long, value_name = "MODE")]
    mode: Option<String>,
    /// The owner ID stored for created directories.
    #[arg(long, value_name = "UID")]
    uid: Option<u16>,
    /// The group ID stored for created directories.
    #[arg(long, value_name = "GID")]
    gid: Option<u8>,
    /// The modification time stored for created directories.
    #[arg(long, value_name = "UNIX_SECONDS")]
    mtime: Option<u32>,
}

/// The user-facing device-node kinds accepted by the CLI.
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum DeviceKindArg {
    /// Create a block-device inode.
    Block,
    /// Create a character-device inode.
    Char,
}

impl From<DeviceKindArg> for DeviceNodeKind {
    /// Convert the CLI enum into the shared core enum.
    fn from(value: DeviceKindArg) -> Self {
        match value {
            DeviceKindArg::Block => Self::Block,
            DeviceKindArg::Char => Self::Character,
        }
    }
}

/// Arguments for `mknod`.
#[derive(Debug, Args)]
struct MknodArgs {
    /// The image file to open.
    #[arg(value_name = "IMAGE")]
    image: PathBuf,
    /// The device path to create inside the image.
    #[arg(value_name = "PATH")]
    path: String,
    /// The special inode kind to create.
    #[arg(long, value_enum)]
    kind: DeviceKindArg,
    /// The device major number.
    #[arg(long, value_name = "MAJOR")]
    major: u8,
    /// The device minor number.
    #[arg(long, value_name = "MINOR")]
    minor: u8,
    /// The permission bits stored for the created device inode.
    #[arg(long, value_name = "MODE")]
    mode: Option<String>,
    /// The owner ID stored for the created device inode.
    #[arg(long, value_name = "UID")]
    uid: Option<u16>,
    /// The group ID stored for the created device inode.
    #[arg(long, value_name = "GID")]
    gid: Option<u8>,
    /// The modification time stored for the created device inode.
    #[arg(long, value_name = "UNIX_SECONDS")]
    mtime: Option<u32>,
}

/// Arguments shared by `mv` and `ln`.
#[derive(Debug, Args)]
struct RenameArgs {
    /// The image file to open.
    #[arg(value_name = "IMAGE")]
    image: PathBuf,
    /// The source image path.
    #[arg(value_name = "SOURCE")]
    source: String,
    /// The target image path.
    #[arg(value_name = "TARGET")]
    target: String,
}

/// The root manifest structure for `build`.
#[derive(Debug, Deserialize)]
struct BuildManifest {
    /// The image-level build settings.
    image: ManifestImage,
    /// The ordered list of mappings to apply.
    #[serde(default)]
    mapping: Vec<ManifestMapping>,
}

/// The `[image]` manifest section.
#[derive(Debug, Deserialize)]
struct ManifestImage {
    /// The output image path.
    output: PathBuf,
    /// The logical image size string.
    size: String,
    /// The inode count reserved in the image.
    inode_count: u16,
    /// The default owner ID.
    default_uid: Option<u16>,
    /// The default group ID.
    default_gid: Option<u8>,
    /// The default modification time.
    default_mtime: Option<u32>,
    /// The default regular-file mode.
    default_file_mode: Option<String>,
    /// The default directory mode.
    default_dir_mode: Option<String>,
    /// Whether the output may be overwritten.
    overwrite_output: Option<bool>,
}

/// One `[[mapping]]` manifest entry.
#[derive(Debug, Deserialize)]
struct ManifestMapping {
    /// The mapping kind.
    kind: String,
    /// The optional host source path.
    source: Option<PathBuf>,
    /// The target path inside the image.
    target: String,
    /// The optional file or directory mode.
    mode: Option<String>,
    /// The optional regular-file mode override for tree mappings.
    file_mode: Option<String>,
    /// The optional directory mode override for tree mappings.
    dir_mode: Option<String>,
    /// The optional owner override.
    uid: Option<u16>,
    /// The optional group override.
    gid: Option<u8>,
    /// The optional modification-time override.
    mtime: Option<u32>,
    /// Whether existing regular files may be overwritten.
    overwrite: Option<bool>,
    /// The optional device major number for special-node mappings.
    major: Option<u8>,
    /// The optional device minor number for special-node mappings.
    minor: Option<u8>,
}

/// Open one Minix image file for reading and writing.
fn open_image(path: &Path) -> miniximg::Result<MinixFileSystem<File>> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|source| {
            MinixError::io(Some(path.to_path_buf()), "failed to open image", source)
        })?;
    MinixFileSystem::open(file)
}

/// Convert CLI build arguments into the shared core request model.
fn build_request_from_args(args: &BuildArgs) -> miniximg::Result<BuildRequest> {
    if let Some(path) = &args.manifest {
        return build_request_from_manifest(path);
    }

    let image = ImageSpec {
        output: args
            .output
            .clone()
            .ok_or_else(|| MinixError::InvalidArgument("`--output` is required".into()))?,
        image_size: parse_size(
            args.size
                .as_deref()
                .ok_or_else(|| MinixError::InvalidArgument("`--size` is required".into()))?,
        )?,
        inode_count: args
            .inode_count
            .ok_or_else(|| MinixError::InvalidArgument("`--inode-count` is required".into()))?,
        default_uid: args.default_uid.unwrap_or(0),
        default_gid: args.default_gid.unwrap_or(0),
        default_mtime: args.default_mtime.unwrap_or_else(current_unix_time),
        default_file_mode: parse_optional_mode(args.default_file_mode.as_deref())?.unwrap_or(0o644),
        default_dir_mode: parse_optional_mode(args.default_dir_mode.as_deref())?.unwrap_or(0o755),
        overwrite_output: args.force,
    };

    let entries = args
        .entry
        .iter()
        .map(|spec| parse_entry_spec(spec))
        .collect::<miniximg::Result<Vec<_>>>()?;

    Ok(BuildRequest { image, entries })
}

/// Load one TOML build manifest and convert it into the shared request model.
fn build_request_from_manifest(path: &Path) -> miniximg::Result<BuildRequest> {
    let text = fs::read_to_string(path).map_err(|source| {
        MinixError::io(Some(path.to_path_buf()), "failed to read manifest", source)
    })?;
    let manifest: BuildManifest = toml::from_str(&text).map_err(|error| {
        MinixError::InvalidArgument(format!(
            "failed to parse manifest `{}`: {error}",
            path.display()
        ))
    })?;

    let image = ImageSpec {
        output: manifest.image.output,
        image_size: parse_size(&manifest.image.size)?,
        inode_count: manifest.image.inode_count,
        default_uid: manifest.image.default_uid.unwrap_or(0),
        default_gid: manifest.image.default_gid.unwrap_or(0),
        default_mtime: manifest
            .image
            .default_mtime
            .unwrap_or_else(current_unix_time),
        default_file_mode: parse_optional_mode(manifest.image.default_file_mode.as_deref())?
            .unwrap_or(0o644),
        default_dir_mode: parse_optional_mode(manifest.image.default_dir_mode.as_deref())?
            .unwrap_or(0o755),
        overwrite_output: manifest.image.overwrite_output.unwrap_or(false),
    };

    let entries = manifest
        .mapping
        .iter()
        .map(manifest_mapping_to_entry)
        .collect::<miniximg::Result<Vec<_>>>()?;

    Ok(BuildRequest { image, entries })
}

/// Convert one manifest mapping into the shared core build entry.
fn manifest_mapping_to_entry(mapping: &ManifestMapping) -> miniximg::Result<BuildEntry> {
    let kind = mapping.kind.trim().to_ascii_lowercase();

    match kind.as_str() {
        "file" => Ok(BuildEntry::File(FileMapping {
            source: mapping.source.clone().ok_or_else(|| {
                MinixError::InvalidArgument("file mappings require `source`".into())
            })?,
            target: mapping.target.clone(),
            mode: parse_optional_mode(mapping.mode.as_deref())?,
            uid: mapping.uid,
            gid: mapping.gid,
            mtime: mapping.mtime,
            overwrite: mapping.overwrite.unwrap_or(false),
        })),
        "tree" => Ok(BuildEntry::Tree(TreeMapping {
            source: mapping.source.clone().ok_or_else(|| {
                MinixError::InvalidArgument("tree mappings require `source`".into())
            })?,
            target: mapping.target.clone(),
            file_mode: parse_optional_mode(mapping.file_mode.as_deref())?,
            dir_mode: parse_optional_mode(mapping.dir_mode.as_deref())?,
            uid: mapping.uid,
            gid: mapping.gid,
            mtime: mapping.mtime,
            overwrite: mapping.overwrite.unwrap_or(false),
        })),
        "dir" | "directory" => Ok(BuildEntry::Directory(DirectoryMapping {
            target: mapping.target.clone(),
            mode: parse_optional_mode(mapping.mode.as_deref())?,
            uid: mapping.uid,
            gid: mapping.gid,
            mtime: mapping.mtime,
        })),
        "blockdev" | "block_device" | "block-device" => Ok(BuildEntry::Device(DeviceMapping {
            target: mapping.target.clone(),
            device_kind: DeviceNodeKind::Block,
            major: mapping.major.ok_or_else(|| {
                MinixError::InvalidArgument("block-device mappings require `major`".into())
            })?,
            minor: mapping.minor.ok_or_else(|| {
                MinixError::InvalidArgument("block-device mappings require `minor`".into())
            })?,
            mode: parse_optional_mode(mapping.mode.as_deref())?,
            uid: mapping.uid,
            gid: mapping.gid,
            mtime: mapping.mtime,
        })),
        "chardev" | "char_device" | "char-device" | "character_device" => {
            Ok(BuildEntry::Device(DeviceMapping {
                target: mapping.target.clone(),
                device_kind: DeviceNodeKind::Character,
                major: mapping.major.ok_or_else(|| {
                    MinixError::InvalidArgument("character-device mappings require `major`".into())
                })?,
                minor: mapping.minor.ok_or_else(|| {
                    MinixError::InvalidArgument("character-device mappings require `minor`".into())
                })?,
                mode: parse_optional_mode(mapping.mode.as_deref())?,
                uid: mapping.uid,
                gid: mapping.gid,
                mtime: mapping.mtime,
            }))
        }
        _ => Err(MinixError::InvalidArgument(format!(
            "unknown manifest mapping kind `{}`",
            mapping.kind
        ))),
    }
}

/// Parse one direct `--entry key=value,...` specification.
fn parse_entry_spec(spec: &str) -> miniximg::Result<BuildEntry> {
    let map = parse_key_value_spec(spec)?;
    let kind = map
        .get("kind")
        .ok_or_else(|| MinixError::InvalidEntrySpec {
            spec: spec.into(),
            message: "missing `kind`".into(),
        })?
        .trim()
        .to_ascii_lowercase();

    match kind.as_str() {
        "file" => Ok(BuildEntry::File(FileMapping {
            source: PathBuf::from(required_entry_value(spec, &map, "source")?),
            target: required_entry_value(spec, &map, "target")?.into(),
            mode: parse_entry_mode(spec, &map, "mode")?,
            uid: parse_entry_optional(spec, &map, "uid")?,
            gid: parse_entry_optional(spec, &map, "gid")?,
            mtime: parse_entry_optional(spec, &map, "mtime")?,
            overwrite: parse_entry_bool(spec, &map, "overwrite")?.unwrap_or(false),
        })),
        "tree" => Ok(BuildEntry::Tree(TreeMapping {
            source: PathBuf::from(required_entry_value(spec, &map, "source")?),
            target: required_entry_value(spec, &map, "target")?.into(),
            file_mode: parse_entry_mode(spec, &map, "file_mode")?,
            dir_mode: parse_entry_mode(spec, &map, "dir_mode")?,
            uid: parse_entry_optional(spec, &map, "uid")?,
            gid: parse_entry_optional(spec, &map, "gid")?,
            mtime: parse_entry_optional(spec, &map, "mtime")?,
            overwrite: parse_entry_bool(spec, &map, "overwrite")?.unwrap_or(false),
        })),
        "dir" | "directory" => Ok(BuildEntry::Directory(DirectoryMapping {
            target: required_entry_value(spec, &map, "target")?.into(),
            mode: parse_entry_mode(spec, &map, "mode")?,
            uid: parse_entry_optional(spec, &map, "uid")?,
            gid: parse_entry_optional(spec, &map, "gid")?,
            mtime: parse_entry_optional(spec, &map, "mtime")?,
        })),
        "blockdev" | "block_device" | "block-device" => Ok(BuildEntry::Device(DeviceMapping {
            target: required_entry_value(spec, &map, "target")?.into(),
            device_kind: DeviceNodeKind::Block,
            major: required_entry_numeric(spec, &map, "major")?,
            minor: required_entry_numeric(spec, &map, "minor")?,
            mode: parse_entry_mode(spec, &map, "mode")?,
            uid: parse_entry_optional(spec, &map, "uid")?,
            gid: parse_entry_optional(spec, &map, "gid")?,
            mtime: parse_entry_optional(spec, &map, "mtime")?,
        })),
        "chardev" | "char_device" | "char-device" | "character_device" => {
            Ok(BuildEntry::Device(DeviceMapping {
                target: required_entry_value(spec, &map, "target")?.into(),
                device_kind: DeviceNodeKind::Character,
                major: required_entry_numeric(spec, &map, "major")?,
                minor: required_entry_numeric(spec, &map, "minor")?,
                mode: parse_entry_mode(spec, &map, "mode")?,
                uid: parse_entry_optional(spec, &map, "uid")?,
                gid: parse_entry_optional(spec, &map, "gid")?,
                mtime: parse_entry_optional(spec, &map, "mtime")?,
            }))
        }
        _ => Err(MinixError::InvalidEntrySpec {
            spec: spec.into(),
            message: format!("unknown `kind` value `{kind}`"),
        }),
    }
}

/// Parse one comma-separated `key=value` specification into a string map.
fn parse_key_value_spec(spec: &str) -> miniximg::Result<BTreeMap<String, String>> {
    let mut map = BTreeMap::new();

    for item in spec.split(',') {
        let trimmed = item.trim();
        let (key, value) = trimmed
            .split_once('=')
            .ok_or_else(|| MinixError::InvalidEntrySpec {
                spec: spec.into(),
                message: "expected comma-separated `key=value` items".into(),
            })?;
        map.insert(key.trim().into(), value.trim().into());
    }

    Ok(map)
}

/// Return one required entry field as a borrowed string.
fn required_entry_value<'a>(
    spec: &str,
    map: &'a BTreeMap<String, String>,
    key: &str,
) -> miniximg::Result<&'a str> {
    map.get(key)
        .map(String::as_str)
        .ok_or_else(|| MinixError::InvalidEntrySpec {
            spec: spec.into(),
            message: format!("missing `{key}`"),
        })
}

/// Parse one required numeric field from an entry specification.
fn required_entry_numeric<T>(
    spec: &str,
    map: &BTreeMap<String, String>,
    key: &str,
) -> miniximg::Result<T>
where
    T: std::str::FromStr,
{
    required_entry_value(spec, map, key)?
        .parse::<T>()
        .map_err(|_| MinixError::InvalidEntrySpec {
            spec: spec.into(),
            message: format!("invalid `{key}` value"),
        })
}

/// Parse one optional numeric field from an entry specification.
fn parse_entry_optional<T>(
    spec: &str,
    map: &BTreeMap<String, String>,
    key: &str,
) -> miniximg::Result<Option<T>>
where
    T: std::str::FromStr,
{
    map.get(key)
        .map(|value| {
            value
                .parse::<T>()
                .map_err(|_| MinixError::InvalidEntrySpec {
                    spec: spec.into(),
                    message: format!("invalid `{key}` value"),
                })
        })
        .transpose()
}

/// Parse one optional boolean field from an entry specification.
fn parse_entry_bool(
    spec: &str,
    map: &BTreeMap<String, String>,
    key: &str,
) -> miniximg::Result<Option<bool>> {
    map.get(key)
        .map(|value| {
            value
                .parse::<bool>()
                .map_err(|_| MinixError::InvalidEntrySpec {
                    spec: spec.into(),
                    message: format!("invalid `{key}` boolean value"),
                })
        })
        .transpose()
}

/// Parse one optional mode field from an entry specification.
fn parse_entry_mode(
    spec: &str,
    map: &BTreeMap<String, String>,
    key: &str,
) -> miniximg::Result<Option<u16>> {
    map.get(key)
        .map(|value| {
            parse_mode(value).map_err(|_| MinixError::InvalidEntrySpec {
                spec: spec.into(),
                message: format!("invalid `{key}` mode"),
            })
        })
        .transpose()
}

/// Parse a human-friendly size string into bytes.
fn parse_size(value: &str) -> miniximg::Result<u64> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(MinixError::InvalidSize {
            value: value.into(),
        });
    }

    let digits_end = trimmed
        .find(|character: char| !character.is_ascii_digit())
        .unwrap_or(trimmed.len());
    let (number_text, suffix_text) = trimmed.split_at(digits_end);
    if number_text.is_empty() {
        return Err(MinixError::InvalidSize {
            value: value.into(),
        });
    }

    let number = number_text
        .parse::<u64>()
        .map_err(|_| MinixError::InvalidSize {
            value: value.into(),
        })?;
    let multiplier = match suffix_text.trim().to_ascii_lowercase().as_str() {
        "" | "b" => 1,
        "k" | "kb" => 1_000,
        "m" | "mb" => 1_000_000,
        "g" | "gb" => 1_000_000_000,
        "kib" => 1024,
        "mib" => 1024 * 1024,
        "gib" => 1024 * 1024 * 1024,
        _ => {
            return Err(MinixError::InvalidSize {
                value: value.into(),
            });
        }
    };

    number
        .checked_mul(multiplier)
        .ok_or_else(|| MinixError::InvalidSize {
            value: value.into(),
        })
}

/// Parse one optional octal mode string.
fn parse_optional_mode(value: Option<&str>) -> miniximg::Result<Option<u16>> {
    value.map(parse_mode).transpose()
}

/// Parse one octal mode string into raw permission bits.
fn parse_mode(value: &str) -> miniximg::Result<u16> {
    let trimmed = value.trim();
    let trimmed = trimmed
        .strip_prefix("0o")
        .or_else(|| trimmed.strip_prefix("0O"))
        .unwrap_or(trimmed);

    u16::from_str_radix(trimmed, 8)
        .map(|mode| mode & InodeMode::FLAGS_MASK)
        .map_err(|_| MinixError::InvalidMode {
            value: value.into(),
        })
}

/// Write one host output file, optionally refusing to overwrite it.
fn write_host_file(path: &Path, data: &[u8], force: bool) -> miniximg::Result<()> {
    if path.exists() && !force {
        return Err(MinixError::AlreadyExists(format!(
            "output file `{}` already exists; use `--force` to overwrite it",
            path.display()
        )));
    }

    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|source| {
            MinixError::io(
                Some(parent.to_path_buf()),
                "failed to create parent directories for the host output",
                source,
            )
        })?;
    }

    fs::write(path, data).map_err(|source| {
        MinixError::io(
            Some(path.to_path_buf()),
            "failed to write host output file",
            source,
        )
    })
}

/// Print one filesystem summary.
fn print_inspect(report: &miniximg::InspectReport) {
    println!("Block size: {}", report.block_size);
    println!("Magic: 0x{:04x}", report.magic);
    println!("Inodes: {}", report.inode_count);
    println!("Zones: {}", report.zone_count);
    println!("Inode bitmap blocks: {}", report.inode_bitmap_blocks);
    println!("Zone bitmap blocks: {}", report.zone_bitmap_blocks);
    println!("First data zone: {}", report.first_data_zone);
    println!("Max file size: {}", report.max_file_size);
    println!("Free inodes: {}", report.free_inodes);
    println!("Free zones: {}", report.free_zones);
    println!("Root entries:");
    for entry in &report.root_entries {
        println!(
            "  {} {:>5} {:>12} {}",
            format_type_char(entry.metadata.kind),
            entry.metadata.inode_number,
            format_size_or_device(&entry.metadata),
            entry.name
        );
    }
}

/// Print one check report.
fn print_check(report: &CheckReport) {
    if report.is_clean() {
        println!("check: ok");
        return;
    }

    for issue in &report.issues {
        match &issue.path {
            Some(path) => println!("{path}: {}", issue.message),
            None => println!("{}", issue.message),
        }
    }
}

/// Print one flat listing.
fn print_listing(entries: &[DirectoryEntryInfo]) {
    for entry in entries {
        println!(
            "{} {:06o} {:>5} {:>12} {:>3} {}",
            format_type_char(entry.metadata.kind),
            entry.metadata.mode & InodeMode::FLAGS_MASK,
            entry.metadata.inode_number,
            format_size_or_device(&entry.metadata),
            entry.metadata.link_count,
            entry.name
        );
    }
}

/// Print one recursive tree listing.
fn print_tree(entries: &[TreeEntry]) {
    for entry in entries {
        let name = if entry.metadata.path == "/" {
            "/".into()
        } else {
            entry
                .metadata
                .path
                .rsplit('/')
                .next()
                .unwrap_or("/")
                .to_string()
        };
        println!(
            "{}{} {}",
            "  ".repeat(entry.depth),
            format_type_char(entry.metadata.kind),
            name
        );
    }
}

/// Print one stat report.
fn print_stat(metadata: &NodeMetadata) {
    println!("Path: {}", metadata.path);
    println!("Inode: {}", metadata.inode_number);
    println!("Type: {:?}", metadata.kind);
    println!("Mode: {:06o}", metadata.mode);
    println!("UID: {}", metadata.uid);
    println!("GID: {}", metadata.gid);
    println!("Size: {}", metadata.size);
    if let Some(device_number) = metadata.device_number {
        println!(
            "Device Number: {} (major {}, minor {})",
            device_number,
            device_major(device_number),
            device_minor(device_number)
        );
    }
    println!("Links: {}", metadata.link_count);
    println!("Mtime: {}", metadata.modification_time);
}

/// Return the conventional single-character inode type marker.
fn format_type_char(kind: InodeType) -> char {
    match kind {
        InodeType::Regular => '-',
        InodeType::Directory => 'd',
        InodeType::Fifo => 'p',
        InodeType::BlockDevice => 'b',
        InodeType::CharacterDevice => 'c',
    }
}

/// Format either a byte size or a device-number pair for listings.
fn format_size_or_device(metadata: &NodeMetadata) -> String {
    match metadata.device_number {
        Some(device_number) => format!(
            "{},{}",
            device_major(device_number),
            device_minor(device_number)
        ),
        None => metadata.size.to_string(),
    }
}

/// Return the major part of one Linux 0.11 device number.
fn device_major(device_number: u16) -> u8 {
    (device_number >> 8) as u8
}

/// Return the minor part of one Linux 0.11 device number.
fn device_minor(device_number: u16) -> u8 {
    (device_number & 0xff) as u8
}
