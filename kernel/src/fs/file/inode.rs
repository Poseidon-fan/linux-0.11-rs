use alloc::sync::Arc;
use user_lib::fs::{AccessMode, OpenOptions};

use crate::{fs::minix::Inode, sync::Mutex};

/// Open file object backed by one inode data area.
///
/// This wrapper is used for ordinary files and directories whose readable
/// contents come from the inode's mapped data blocks. Device nodes and pipes
/// use different runtime objects because their I/O semantics do not go
/// through the regular Minix block mapping path.
pub struct InodeFile {
    access_mode: AccessMode,
    open_options: OpenOptions,
    inner: Mutex<InodeFileInner>,
}

/// Mutable open-file state that is private to one opened inode file.
///
/// This mirrors the role of Linux 0.11 `struct file` fields that belong to
/// one open instance instead of the inode itself.
struct InodeFileInner {
    inode: Arc<Inode>,
    offset: usize,
}
