//! Path parsing and normalization helpers for image-internal paths.

use crate::error::{MinixError, Result};
use crate::layout::MINIX_NAME_LENGTH;

/// A normalized absolute path inside one Minix image.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImagePath {
    components: Vec<String>,
}

impl ImagePath {
    /// Return the normalized root path.
    pub const fn root() -> Self {
        Self {
            components: Vec::new(),
        }
    }

    /// Parse and validate one absolute image path.
    pub fn parse(path: &str) -> Result<Self> {
        if !path.starts_with('/') {
            return Err(MinixError::InvalidPath(path.into()));
        }

        if path == "/" {
            return Ok(Self::root());
        }

        let mut components = Vec::new();

        for component in path.split('/').skip(1) {
            if component.is_empty() || component == "." || component == ".." {
                return Err(MinixError::InvalidPath(path.into()));
            }

            if component.len() > MINIX_NAME_LENGTH {
                return Err(MinixError::NameTooLong {
                    name: component.into(),
                    max_bytes: MINIX_NAME_LENGTH,
                });
            }

            components.push(component.into());
        }

        Ok(Self { components })
    }

    /// Return the normalized path components.
    pub fn components(&self) -> &[String] {
        &self.components
    }

    /// Return whether the path is the root directory.
    pub fn is_root(&self) -> bool {
        self.components.is_empty()
    }

    /// Return the last component name if the path is not the root directory.
    pub fn file_name(&self) -> Option<&str> {
        self.components.last().map(String::as_str)
    }

    /// Return the parent path if the path is not the root directory.
    pub fn parent(&self) -> Option<Self> {
        if self.is_root() {
            return None;
        }

        let mut components = self.components.clone();
        components.pop();
        Some(Self { components })
    }

    /// Append one validated component and return the child path.
    pub fn join_name(&self, name: &str) -> Result<Self> {
        if name.is_empty() || name == "." || name == ".." {
            return Err(MinixError::InvalidPath(name.into()));
        }

        if name.len() > MINIX_NAME_LENGTH {
            return Err(MinixError::NameTooLong {
                name: name.into(),
                max_bytes: MINIX_NAME_LENGTH,
            });
        }

        let mut components = self.components.clone();
        components.push(name.into());
        Ok(Self { components })
    }

    /// Render the normalized absolute path as a string.
    pub fn display(&self) -> String {
        if self.is_root() {
            return "/".into();
        }

        format!("/{}", self.components.join("/"))
    }
}

#[cfg(test)]
mod tests {
    //! Path-focused unit tests.

    use super::*;

    /// Confirm the parser accepts the root path unchanged.
    #[test]
    fn root_path_round_trips() {
        let root = ImagePath::parse("/").unwrap();
        assert!(root.is_root());
        assert_eq!(root.display(), "/");
    }

    /// Confirm the parser rejects relative and special-segment paths.
    #[test]
    fn invalid_paths_are_rejected() {
        assert!(ImagePath::parse("relative").is_err());
        assert!(ImagePath::parse("/a//b").is_err());
        assert!(ImagePath::parse("/a/./b").is_err());
        assert!(ImagePath::parse("/a/../b").is_err());
    }
}
