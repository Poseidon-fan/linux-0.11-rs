//! Error types shared by the Minix image library and CLI.

use std::fmt;
use std::io;
use std::path::PathBuf;

use thiserror::Error;

/// The crate-wide result type.
pub type Result<T> = std::result::Result<T, MinixError>;

/// The unified error type used across parsing, image I/O, and validation.
#[derive(Debug, Error)]
pub enum MinixError {
    /// A wrapped I/O error with optional path context.
    #[error("{message}{path_suffix}")]
    Io {
        /// The path associated with the failing I/O operation.
        path: Option<PathBuf>,
        /// The high-level error message.
        message: String,
        /// The underlying I/O error.
        #[source]
        source: io::Error,
        /// The rendered path suffix appended to the display form.
        path_suffix: PathSuffix,
    },
    /// A generic invalid-argument failure.
    #[error("{0}")]
    InvalidArgument(String),
    /// A filesystem path failed validation.
    #[error("invalid image path `{0}`")]
    InvalidPath(String),
    /// One path component exceeds the Minix directory entry limit.
    #[error("path component `{name}` exceeds the {max_bytes}-byte Minix name limit")]
    NameTooLong {
        /// The offending component name.
        name: String,
        /// The maximum encoded byte length supported by the format.
        max_bytes: usize,
    },
    /// The image contains unsupported or nonsensical state.
    #[error("{0}")]
    Unsupported(String),
    /// The image contents are structurally invalid.
    #[error("{0}")]
    Corrupt(String),
    /// A requested path or inode does not exist.
    #[error("{0}")]
    NotFound(String),
    /// A requested object already exists.
    #[error("{0}")]
    AlreadyExists(String),
    /// An operation expected a directory but found another inode type.
    #[error("{0}")]
    NotDirectory(String),
    /// An operation expected a regular file but found a directory.
    #[error("{0}")]
    IsDirectory(String),
    /// A directory removal was attempted on a non-empty directory.
    #[error("{0}")]
    DirectoryNotEmpty(String),
    /// The filesystem ran out of available resources.
    #[error("{0}")]
    NoSpace(String),
    /// A human-friendly size string could not be parsed.
    #[error("invalid size `{value}`")]
    InvalidSize {
        /// The original size string.
        value: String,
    },
    /// A permission or mode string could not be parsed.
    #[error("invalid mode `{value}`")]
    InvalidMode {
        /// The original mode string.
        value: String,
    },
    /// A direct CLI entry specification could not be parsed.
    #[error("invalid entry spec `{spec}`: {message}")]
    InvalidEntrySpec {
        /// The original entry specification.
        spec: String,
        /// The specific parsing failure.
        message: String,
    },
    /// The command already emitted a report and only needs a process exit code.
    #[error("")]
    SilentFailure(i32),
}

/// A tiny wrapper keeps the main error enum readable.
#[derive(Clone, Debug)]
pub struct PathSuffix(pub String);

impl fmt::Display for PathSuffix {
    /// Render the suffix appended to I/O errors.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl MinixError {
    /// Build an I/O error with optional path context.
    pub fn io(path: Option<PathBuf>, message: impl Into<String>, source: io::Error) -> Self {
        let path_suffix = match &path {
            Some(value) => PathSuffix(format!(" ({})", value.display())),
            None => PathSuffix(String::new()),
        };

        Self::Io {
            path,
            message: message.into(),
            source,
            path_suffix,
        }
    }

    /// Return the exit code that should be used by the CLI.
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::SilentFailure(code) => *code,
            Self::InvalidArgument(_)
            | Self::InvalidPath(_)
            | Self::NameTooLong { .. }
            | Self::InvalidSize { .. }
            | Self::InvalidMode { .. }
            | Self::InvalidEntrySpec { .. } => 2,
            _ => 1,
        }
    }

    /// Tell the CLI whether the error should be printed to stderr.
    pub fn should_print(&self) -> bool {
        !matches!(self, Self::SilentFailure(_))
    }
}
