use alloc::sync::Arc;

use crate::{
    fs::{
        file::{File, FileStat, OpenOptions, SeekFrom},
        minix::Inode,
    },
    sync::Mutex,
};

pub struct InodeFile {
    pub inode: Arc<Mutex<Inode>>,
    pub offset: Mutex<u64>,
    pub options: OpenOptions,
}

impl InodeFile {
    /// Create one open file object that references `inode`.
    pub fn new(inode: Arc<Mutex<Inode>>, options: OpenOptions) -> Self {
        Self {
            inode,
            offset: Mutex::new(0),
            options,
        }
    }
}

impl File for InodeFile {
    fn read(&self, _buffer: &mut [u8]) -> Result<usize, u32> {
        todo!("InodeFile::read is not implemented in the structure-only phase")
    }

    fn write(&self, _buffer: &[u8]) -> Result<usize, u32> {
        todo!("InodeFile::write is not implemented in the structure-only phase")
    }

    fn seek(&self, _position: SeekFrom) -> Result<u64, u32> {
        todo!("InodeFile::seek is not implemented in the structure-only phase")
    }

    fn stat(&self) -> Result<FileStat, u32> {
        todo!("InodeFile::stat is not implemented in the structure-only phase")
    }
}
