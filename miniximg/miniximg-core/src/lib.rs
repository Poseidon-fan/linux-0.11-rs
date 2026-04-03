//! Reusable Minix filesystem image logic.
//!
//! The library keeps all filesystem semantics inside the core crate so that the
//! CLI can stay focused on parsing user input and rendering reports.

pub mod bitmap;
pub mod build;
pub mod error;
pub mod fs;
pub mod layout;
pub mod path;
pub mod report;

pub use build::{
    BuildEntry, BuildRequest, DeviceMapping, DeviceNodeKind, DirectoryMapping, FileMapping,
    ImageSpec, TreeMapping, build_image, device_number,
};
pub use error::{MinixError, Result};
pub use fs::{CreateImageOptions, CreateNodeOptions, MinixFileSystem};
pub use layout::{
    BLOCK_SIZE, DIRECT_ZONE_COUNT, DIRECTORY_ENTRY_SIZE, INDIRECT_ENTRY_COUNT, InodeMode,
    InodeModeFlags, InodeType, MINIX_NAME_LENGTH, MINIX_SUPER_MAGIC, ROOT_INODE_NUMBER,
};
pub use report::{
    CheckIssue, CheckReport, DirectoryEntryInfo, InspectReport, NodeMetadata, TreeEntry,
};
