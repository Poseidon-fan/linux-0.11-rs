//! Build-request DTOs and host-directory image construction helpers.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::{MinixError, Result};
use crate::fs::{CreateImageOptions, CreateNodeOptions, MinixFileSystem};

/// The supported special-device inode kinds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeviceNodeKind {
    /// A block-device inode.
    Block,
    /// A character-device inode.
    Character,
}

/// Top-level image defaults and output configuration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImageSpec {
    /// The output image path.
    pub output: PathBuf,
    /// The logical image size in bytes.
    pub image_size: u64,
    /// The inode count reserved in the image.
    pub inode_count: u16,
    /// The default file owner.
    pub default_uid: u16,
    /// The default group owner.
    pub default_gid: u8,
    /// The default modification time.
    pub default_mtime: u32,
    /// The default regular-file permissions.
    pub default_file_mode: u16,
    /// The default directory permissions.
    pub default_dir_mode: u16,
    /// Whether the output file may be overwritten.
    pub overwrite_output: bool,
}

/// One full build request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BuildRequest {
    /// The image-level configuration.
    pub image: ImageSpec,
    /// The ordered list of host-to-image mappings.
    pub entries: Vec<BuildEntry>,
}

/// One host-to-image mapping.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BuildEntry {
    /// Copy one host file into the image.
    File(FileMapping),
    /// Recursively copy one host directory tree into the image.
    Tree(TreeMapping),
    /// Create one directory inside the image.
    Directory(DirectoryMapping),
    /// Create one device inode inside the image.
    Device(DeviceMapping),
}

/// A single host-file copy operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileMapping {
    /// The host file to copy.
    pub source: PathBuf,
    /// The absolute target path inside the image.
    pub target: String,
    /// An optional permission override.
    pub mode: Option<u16>,
    /// An optional owner override.
    pub uid: Option<u16>,
    /// An optional group override.
    pub gid: Option<u8>,
    /// An optional modification-time override.
    pub mtime: Option<u32>,
    /// Whether an existing regular file may be overwritten.
    pub overwrite: bool,
}

/// A recursive host-directory copy operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TreeMapping {
    /// The host directory to scan.
    pub source: PathBuf,
    /// The absolute target directory inside the image.
    pub target: String,
    /// An optional regular-file permission override.
    pub file_mode: Option<u16>,
    /// An optional directory permission override.
    pub dir_mode: Option<u16>,
    /// An optional owner override.
    pub uid: Option<u16>,
    /// An optional group override.
    pub gid: Option<u8>,
    /// An optional modification-time override.
    pub mtime: Option<u32>,
    /// Whether existing regular files may be overwritten.
    pub overwrite: bool,
}

/// A directory-creation mapping.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DirectoryMapping {
    /// The absolute directory path to create.
    pub target: String,
    /// An optional permission override.
    pub mode: Option<u16>,
    /// An optional owner override.
    pub uid: Option<u16>,
    /// An optional group override.
    pub gid: Option<u8>,
    /// An optional modification-time override.
    pub mtime: Option<u32>,
}

/// A device-node creation mapping.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeviceMapping {
    /// The absolute target path to create inside the image.
    pub target: String,
    /// The special device kind to create.
    pub device_kind: DeviceNodeKind,
    /// The device major number.
    pub major: u8,
    /// The device minor number.
    pub minor: u8,
    /// An optional permission override.
    pub mode: Option<u16>,
    /// An optional owner override.
    pub uid: Option<u16>,
    /// An optional group override.
    pub gid: Option<u8>,
    /// An optional modification-time override.
    pub mtime: Option<u32>,
}

/// Build one filesystem image directly from a resolved request.
pub fn build_image(request: &BuildRequest) -> Result<()> {
    if let Some(parent) = request.image.output.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|source| {
            MinixError::io(
                Some(parent.to_path_buf()),
                "failed to create parent directories for the image output",
                source,
            )
        })?;
    }

    let mut open_options = OpenOptions::new();
    open_options
        .read(true)
        .write(true)
        .create(true)
        .truncate(true);

    if !request.image.overwrite_output && request.image.output.exists() {
        return Err(MinixError::AlreadyExists(format!(
            "output image `{}` already exists; use `--force` to overwrite it",
            request.image.output.display()
        )));
    }

    let file = open_options.open(&request.image.output).map_err(|source| {
        MinixError::io(
            Some(request.image.output.clone()),
            "failed to create output image",
            source,
        )
    })?;
    let mut image = MinixFileSystem::create(
        file,
        CreateImageOptions {
            image_size: request.image.image_size,
            inode_count: request.image.inode_count,
            root_mode: request.image.default_dir_mode,
            default_uid: request.image.default_uid,
            default_gid: request.image.default_gid,
            default_mtime: request.image.default_mtime,
        },
    )?;

    for entry in &request.entries {
        match entry {
            BuildEntry::File(mapping) => apply_file_mapping(&mut image, &request.image, mapping)?,
            BuildEntry::Tree(mapping) => apply_tree_mapping(&mut image, &request.image, mapping)?,
            BuildEntry::Directory(mapping) => {
                apply_directory_mapping(&mut image, &request.image, mapping)?
            }
            BuildEntry::Device(mapping) => {
                apply_device_mapping(&mut image, &request.image, mapping)?
            }
        }
    }

    image.flush()?;
    let mut file = image.into_inner()?;
    file.flush().map_err(|source| {
        MinixError::io(
            Some(request.image.output.clone()),
            "failed to flush output image",
            source,
        )
    })
}

/// Apply one single-file mapping.
fn apply_file_mapping(
    image: &mut MinixFileSystem<File>,
    image_spec: &ImageSpec,
    mapping: &FileMapping,
) -> Result<()> {
    let metadata = fs::symlink_metadata(&mapping.source).map_err(|source| {
        MinixError::io(
            Some(mapping.source.clone()),
            "failed to read source file metadata",
            source,
        )
    })?;

    if metadata.file_type().is_symlink() {
        return Err(MinixError::Unsupported(format!(
            "symbolic links are not supported by build mappings (`{}`)",
            mapping.source.display()
        )));
    }

    if !metadata.is_file() {
        return Err(MinixError::InvalidArgument(format!(
            "file mapping source `{}` is not a regular file",
            mapping.source.display()
        )));
    }

    let data = fs::read(&mapping.source).map_err(|source| {
        MinixError::io(
            Some(mapping.source.clone()),
            "failed to read source file",
            source,
        )
    })?;
    let file_options = create_file_options(
        image_spec,
        mapping.mode,
        mapping.uid,
        mapping.gid,
        mapping.mtime,
        Some(&metadata),
    );
    let parent_options = default_directory_options(image_spec);

    image.write_file_at_path(
        &mapping.target,
        &data,
        &file_options,
        mapping.overwrite,
        &parent_options,
    )
}

/// Apply one recursive tree mapping.
fn apply_tree_mapping(
    image: &mut MinixFileSystem<File>,
    image_spec: &ImageSpec,
    mapping: &TreeMapping,
) -> Result<()> {
    let metadata = fs::symlink_metadata(&mapping.source).map_err(|source| {
        MinixError::io(
            Some(mapping.source.clone()),
            "failed to read source directory metadata",
            source,
        )
    })?;

    if metadata.file_type().is_symlink() {
        return Err(MinixError::Unsupported(format!(
            "symbolic links are not supported by build mappings (`{}`)",
            mapping.source.display()
        )));
    }

    if !metadata.is_dir() {
        return Err(MinixError::InvalidArgument(format!(
            "tree mapping source `{}` is not a directory",
            mapping.source.display()
        )));
    }

    let directory_options = create_directory_options(
        image_spec,
        mapping.dir_mode,
        mapping.uid,
        mapping.gid,
        mapping.mtime,
    );
    image.mkdir_all(&mapping.target, &directory_options)?;

    apply_tree_directory(
        image,
        image_spec,
        mapping,
        &mapping.source,
        Path::new(""),
        &directory_options,
    )
}

/// Recursively copy one source directory into the image.
fn apply_tree_directory(
    image: &mut MinixFileSystem<File>,
    image_spec: &ImageSpec,
    mapping: &TreeMapping,
    source_root: &Path,
    relative: &Path,
    directory_options: &CreateNodeOptions,
) -> Result<()> {
    let host_directory = source_root.join(relative);
    let mut entries = fs::read_dir(&host_directory)
        .map_err(|source| {
            MinixError::io(
                Some(host_directory.clone()),
                "failed to read source directory",
                source,
            )
        })?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|source| {
            MinixError::io(
                Some(host_directory.clone()),
                "failed to enumerate source directory",
                source,
            )
        })?;
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let path = entry.path();
        let file_name = entry.file_name();
        let file_name = file_name.to_string_lossy().into_owned();
        let child_relative = relative.join(&file_name);
        let target_path = if mapping.target == "/" {
            format!("/{}", child_relative.display())
        } else {
            format!(
                "{}/{}",
                mapping.target.trim_end_matches('/'),
                child_relative.display()
            )
        };
        let metadata = fs::symlink_metadata(&path).map_err(|source| {
            MinixError::io(
                Some(path.clone()),
                "failed to read source entry metadata",
                source,
            )
        })?;

        if metadata.file_type().is_symlink() {
            return Err(MinixError::Unsupported(format!(
                "symbolic links are not supported by build mappings (`{}`)",
                path.display()
            )));
        }

        if metadata.is_dir() {
            let child_directory_options = create_directory_options(
                image_spec,
                mapping.dir_mode,
                mapping.uid,
                mapping.gid,
                mapping.mtime,
            );
            image.mkdir_all(&target_path, &child_directory_options)?;
            apply_tree_directory(
                image,
                image_spec,
                mapping,
                source_root,
                &child_relative,
                &child_directory_options,
            )?;
            continue;
        }

        if !metadata.is_file() {
            return Err(MinixError::Unsupported(format!(
                "unsupported source entry kind at `{}`",
                path.display()
            )));
        }

        let mut file = File::open(&path).map_err(|source| {
            MinixError::io(Some(path.clone()), "failed to open source file", source)
        })?;
        let mut data = Vec::new();
        file.read_to_end(&mut data).map_err(|source| {
            MinixError::io(Some(path.clone()), "failed to read source file", source)
        })?;

        let file_options = create_file_options(
            image_spec,
            mapping.file_mode,
            mapping.uid,
            mapping.gid,
            mapping.mtime,
            Some(&metadata),
        );
        image.write_file_at_path(
            &target_path,
            &data,
            &file_options,
            mapping.overwrite,
            directory_options,
        )?;
    }

    Ok(())
}

/// Apply one directory-only mapping.
fn apply_directory_mapping(
    image: &mut MinixFileSystem<File>,
    image_spec: &ImageSpec,
    mapping: &DirectoryMapping,
) -> Result<()> {
    let options = create_directory_options(
        image_spec,
        mapping.mode,
        mapping.uid,
        mapping.gid,
        mapping.mtime,
    );
    image.mkdir_all(&mapping.target, &options)
}

/// Apply one device-node mapping.
fn apply_device_mapping(
    image: &mut MinixFileSystem<File>,
    image_spec: &ImageSpec,
    mapping: &DeviceMapping,
) -> Result<()> {
    let options = create_device_options(
        image_spec,
        mapping.mode,
        mapping.uid,
        mapping.gid,
        mapping.mtime,
    );
    let parent_options = default_directory_options(image_spec);

    image.create_device_at_path(
        &mapping.target,
        mapping.device_kind,
        device_number(mapping.major, mapping.minor),
        &options,
        &parent_options,
    )
}

/// Build directory attributes from image defaults and optional overrides.
fn create_directory_options(
    image_spec: &ImageSpec,
    mode: Option<u16>,
    uid: Option<u16>,
    gid: Option<u8>,
    mtime: Option<u32>,
) -> CreateNodeOptions {
    CreateNodeOptions {
        mode: mode.unwrap_or(image_spec.default_dir_mode),
        uid: uid.unwrap_or(image_spec.default_uid),
        gid: gid.unwrap_or(image_spec.default_gid),
        mtime: mtime.unwrap_or(image_spec.default_mtime),
    }
}

/// Build device-node attributes from image defaults and optional overrides.
fn create_device_options(
    image_spec: &ImageSpec,
    mode: Option<u16>,
    uid: Option<u16>,
    gid: Option<u8>,
    mtime: Option<u32>,
) -> CreateNodeOptions {
    CreateNodeOptions {
        mode: mode.unwrap_or(0o644),
        uid: uid.unwrap_or(image_spec.default_uid),
        gid: gid.unwrap_or(image_spec.default_gid),
        mtime: mtime.unwrap_or(image_spec.default_mtime),
    }
}

/// Return the default directory attributes used for auto-created parents.
fn default_directory_options(image_spec: &ImageSpec) -> CreateNodeOptions {
    CreateNodeOptions {
        mode: image_spec.default_dir_mode,
        uid: image_spec.default_uid,
        gid: image_spec.default_gid,
        mtime: image_spec.default_mtime,
    }
}

/// Build file attributes from image defaults, host metadata, and explicit overrides.
fn create_file_options(
    image_spec: &ImageSpec,
    mode: Option<u16>,
    uid: Option<u16>,
    gid: Option<u8>,
    mtime: Option<u32>,
    metadata: Option<&fs::Metadata>,
) -> CreateNodeOptions {
    let mode = mode.unwrap_or_else(|| {
        let mut resolved = image_spec.default_file_mode;
        if let Some(metadata) = metadata
            && host_file_is_executable(metadata)
        {
            resolved |= 0o111;
        }
        resolved
    });

    let mtime = mtime
        .or_else(|| metadata.and_then(host_modified_time))
        .unwrap_or(image_spec.default_mtime);

    CreateNodeOptions {
        mode,
        uid: uid.unwrap_or(image_spec.default_uid),
        gid: gid.unwrap_or(image_spec.default_gid),
        mtime,
    }
}

/// Return whether the host file has any executable bit set.
#[cfg(unix)]
fn host_file_is_executable(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;

    metadata.permissions().mode() & 0o111 != 0
}

/// Return whether the host file has any executable bit set.
#[cfg(not(unix))]
fn host_file_is_executable(_metadata: &fs::Metadata) -> bool {
    false
}

/// Return the host modification time as a Unix timestamp.
fn host_modified_time(metadata: &fs::Metadata) -> Option<u32> {
    let modified = metadata.modified().ok()?;
    let duration = modified.duration_since(UNIX_EPOCH).ok()?;
    u32::try_from(duration.as_secs()).ok()
}

/// Return the current Unix timestamp truncated to 32 bits.
pub fn current_unix_time() -> u32 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .ok()
        .and_then(|seconds| u32::try_from(seconds).ok())
        .unwrap_or(0)
}

/// Pack one Linux 0.11 device number from 8-bit major and minor parts.
pub const fn device_number(major: u8, minor: u8) -> u16 {
    ((major as u16) << 8) | minor as u16
}
