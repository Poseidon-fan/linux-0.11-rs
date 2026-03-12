//! Error types shared by all `mbrkit` modules.

use std::fmt;
use std::io;
use std::path::PathBuf;

use thiserror::Error;

/// The crate-wide result type.
pub type Result<T> = std::result::Result<T, MbrkitError>;

/// The unified application error type.
#[derive(Debug, Error)]
pub enum MbrkitError {
    /// A wrapped I/O error with optional path context.
    #[error("{message}{path_suffix}")]
    Io {
        /// The path associated with the I/O operation.
        path: Option<PathBuf>,
        /// The high-level error message.
        message: String,
        /// The underlying I/O error.
        #[source]
        source: io::Error,
        /// The rendered path suffix used by the display implementation.
        path_suffix: PathSuffix,
    },
    /// A generic validation error.
    #[error("{0}")]
    InvalidArgument(String),
    /// A partition specification could not be parsed.
    #[error("invalid partition spec `{spec}`: {message}")]
    InvalidPartitionSpec {
        /// The original partition specification string.
        spec: String,
        /// The specific parsing failure.
        message: String,
    },
    /// A user-provided size string is invalid.
    #[error("invalid size `{value}`")]
    InvalidSize {
        /// The original size string.
        value: String,
    },
    /// A user-provided partition type string is invalid.
    #[error("invalid partition type `{value}`")]
    InvalidPartitionType {
        /// The original partition type string.
        value: String,
    },
    /// The disk image does not contain a valid or complete MBR sector.
    #[error("{0}")]
    InvalidMbr(String),
    /// The command already emitted a report and only needs a process exit code.
    #[error("")]
    SilentFailure(i32),
    /// Serialization failed while writing JSON output.
    #[error("failed to serialize report: {0}")]
    Serialize(#[from] serde_json::Error),
    /// TOML manifest parsing failed.
    #[error("failed to parse manifest `{path}`: {source}")]
    Manifest {
        /// The manifest path string used in the error message.
        path: String,
        /// The underlying parse error.
        #[source]
        source: toml::de::Error,
    },
}

/// A tiny helper wrapper keeps the main error enum concise.
#[derive(Clone, Debug)]
pub struct PathSuffix(pub String);

impl fmt::Display for PathSuffix {
    /// Render the suffix appended to I/O errors.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl MbrkitError {
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

    /// Return the exit code that should be used by the binary.
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::SilentFailure(code) => *code,
            Self::InvalidArgument(_)
            | Self::InvalidPartitionSpec { .. }
            | Self::InvalidSize { .. }
            | Self::InvalidPartitionType { .. } => 2,
            _ => 1,
        }
    }

    /// Tell the binary whether the error should be printed to stderr.
    pub fn should_print(&self) -> bool {
        !matches!(self, Self::SilentFailure(_))
    }
}
