//! Stateful Minix filesystem image access and mutation logic.

use std::collections::HashSet;
use std::io::{Read, Seek, SeekFrom, Write};

use crate::bitmap::{BITS_PER_BLOCK, Bitmap};
use crate::build::DeviceNodeKind;
use crate::error::{MinixError, Result};
use crate::layout::{
    BLOCK_SIZE, DIRECT_ZONE_COUNT, DIRECTORY_ENTRY_SIZE, DiskDirectoryEntry, DiskInode,
    DiskSuperBlock, INDIRECT_ENTRY_COUNT, INODES_PER_BLOCK, InodeMode, InodeType, MAX_FILE_SIZE,
    MAX_LOGICAL_BLOCKS, MINIX_SUPER_MAGIC, ROOT_INODE_NUMBER,
};
use crate::path::ImagePath;
use crate::report::{
    CheckIssue, CheckReport, DirectoryEntryInfo, InspectReport, NodeMetadata, TreeEntry,
};

/// The options needed to create one empty image.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CreateImageOptions {
    /// The logical image size in bytes.
    pub image_size: u64,
    /// The number of usable inode slots.
    pub inode_count: u16,
    /// The root-directory permission bits.
    pub root_mode: u16,
    /// The default owner used for the root inode.
    pub default_uid: u16,
    /// The default group owner used for the root inode.
    pub default_gid: u8,
    /// The default modification time used for the root inode.
    pub default_mtime: u32,
}

/// Common metadata used when creating or overwriting one inode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CreateNodeOptions {
    /// The permission bits stored below the inode type field.
    pub mode: u16,
    /// The owning user ID.
    pub uid: u16,
    /// The owning group ID.
    pub gid: u8,
    /// The modification time.
    pub mtime: u32,
}

/// One loaded Minix filesystem image backed by any seekable byte store.
pub struct MinixFileSystem<S: Read + Write + Seek> {
    storage: S,
    super_block: DiskSuperBlock,
    inode_bitmap: Bitmap,
    zone_bitmap: Bitmap,
}

impl<S: Read + Write + Seek> MinixFileSystem<S> {
    /// Open an existing Minix image and load its metadata structures.
    pub fn open(mut storage: S) -> Result<Self> {
        let mut super_block_buffer = [0_u8; BLOCK_SIZE];
        read_block_from(&mut storage, 1, &mut super_block_buffer)?;
        let super_block = DiskSuperBlock::decode(&super_block_buffer)?;

        if super_block.magic != MINIX_SUPER_MAGIC {
            return Err(MinixError::Corrupt(format!(
                "invalid Minix super-block magic 0x{:04x}",
                super_block.magic
            )));
        }

        if super_block.log_zone_size != 0 {
            return Err(MinixError::Unsupported(format!(
                "unsupported log-zone size {}; only zero is supported",
                super_block.log_zone_size
            )));
        }

        if super_block.inode_count == 0 {
            return Err(MinixError::Corrupt("the image reports zero inodes".into()));
        }

        if super_block.first_data_zone >= super_block.zone_count {
            return Err(MinixError::Corrupt(
                "the first data zone is outside the reported zone range".into(),
            ));
        }

        let mut block_number = 2_u32;
        let mut inode_blocks = Vec::with_capacity(super_block.inode_bitmap_block_count as usize);
        for _ in 0..super_block.inode_bitmap_block_count {
            let mut block = [0_u8; BLOCK_SIZE];
            read_block_from(&mut storage, block_number, &mut block)?;
            inode_blocks.push(block);
            block_number += 1;
        }

        let mut zone_blocks = Vec::with_capacity(super_block.zone_bitmap_block_count as usize);
        for _ in 0..super_block.zone_bitmap_block_count {
            let mut block = [0_u8; BLOCK_SIZE];
            read_block_from(&mut storage, block_number, &mut block)?;
            zone_blocks.push(block);
            block_number += 1;
        }

        let inode_bitmap =
            Bitmap::from_blocks(0, super_block.inode_count as usize + 1, inode_blocks);
        let zone_bitmap = Bitmap::from_blocks(
            super_block.first_data_zone as u32 - 1,
            super_block.zone_count as usize - super_block.first_data_zone as usize + 1,
            zone_blocks,
        );

        Ok(Self {
            storage,
            super_block,
            inode_bitmap,
            zone_bitmap,
        })
    }

    /// Create a new empty Minix image.
    pub fn create(mut storage: S, options: CreateImageOptions) -> Result<Self> {
        if options.image_size == 0 || options.image_size % BLOCK_SIZE as u64 != 0 {
            return Err(MinixError::InvalidArgument(format!(
                "image size must be a non-zero multiple of {} bytes",
                BLOCK_SIZE
            )));
        }

        let zone_count = options.image_size / BLOCK_SIZE as u64;
        if zone_count > u16::MAX as u64 {
            return Err(MinixError::InvalidArgument(
                "image is too large for the 16-bit Minix zone count".into(),
            ));
        }

        if options.inode_count == 0 {
            return Err(MinixError::InvalidArgument(
                "inode count must be greater than zero".into(),
            ));
        }

        let inode_bitmap_block_count =
            ((options.inode_count as usize + 1).div_ceil(BITS_PER_BLOCK)) as u16;
        let inode_table_block_count =
            ((options.inode_count as usize).div_ceil(INODES_PER_BLOCK)) as u16;
        let mut zone_bitmap_block_count = 1_u16;

        loop {
            let first_data_zone = 2_u32
                + inode_bitmap_block_count as u32
                + zone_bitmap_block_count as u32
                + inode_table_block_count as u32;

            if first_data_zone >= zone_count as u32 {
                return Err(MinixError::InvalidArgument(
                    "image is too small to hold the requested metadata layout".into(),
                ));
            }

            let zone_bit_count = zone_count as usize - first_data_zone as usize + 1;
            let required_zone_bitmap_blocks = zone_bit_count.div_ceil(BITS_PER_BLOCK) as u16;
            if required_zone_bitmap_blocks == zone_bitmap_block_count {
                break;
            }
            zone_bitmap_block_count = required_zone_bitmap_blocks.max(1);
        }

        let first_data_zone =
            2_u16 + inode_bitmap_block_count + zone_bitmap_block_count + inode_table_block_count;
        if first_data_zone >= zone_count as u16 {
            return Err(MinixError::InvalidArgument(
                "image layout leaves no room for data zones".into(),
            ));
        }

        for block_number in 0..zone_count as u32 {
            write_zero_block_to(&mut storage, block_number)?;
        }

        let super_block = DiskSuperBlock {
            inode_count: options.inode_count,
            zone_count: zone_count as u16,
            inode_bitmap_block_count,
            zone_bitmap_block_count,
            first_data_zone,
            log_zone_size: 0,
            max_file_size: MAX_FILE_SIZE,
            magic: MINIX_SUPER_MAGIC,
        };
        let mut inode_bitmap = Bitmap::empty(0, options.inode_count as usize + 1);
        let mut zone_bitmap = Bitmap::empty(
            first_data_zone as u32 - 1,
            zone_count as usize - first_data_zone as usize + 1,
        );

        // Bit 0 is reserved in both Minix bitmaps.
        inode_bitmap.set(0);
        zone_bitmap.set(first_data_zone as u32 - 1);

        let mut image = Self {
            storage,
            super_block,
            inode_bitmap,
            zone_bitmap,
        };
        image.sync_super_block()?;

        let root_inode_number = image.allocate_inode_number()?;
        debug_assert_eq!(root_inode_number, ROOT_INODE_NUMBER);
        let root_zone = image.allocate_zone()?;

        let mut root_inode = DiskInode::zeroed();
        root_inode.mode = InodeMode::directory(options.root_mode);
        root_inode.user_id = options.default_uid;
        root_inode.group_id = options.default_gid;
        root_inode.modification_time = options.default_mtime;
        root_inode.link_count = 2;
        root_inode.direct_zones[0] = root_zone;
        root_inode.size = (DIRECTORY_ENTRY_SIZE * 2) as u32;

        let root_entries = vec![
            DiskDirectoryEntry::new(".", ROOT_INODE_NUMBER)?,
            DiskDirectoryEntry::new("..", ROOT_INODE_NUMBER)?,
        ];

        image.write_directory_entries_with_inode(
            ROOT_INODE_NUMBER,
            &mut root_inode,
            &root_entries,
            options.default_mtime,
        )?;
        image.write_inode(ROOT_INODE_NUMBER, &root_inode)?;
        image.flush()?;

        Ok(image)
    }

    /// Return the loaded super block.
    pub fn super_block(&self) -> &DiskSuperBlock {
        &self.super_block
    }

    /// Flush buffered writes on the underlying storage.
    pub fn flush(&mut self) -> Result<()> {
        self.storage
            .flush()
            .map_err(|source| MinixError::io(None, "failed to flush image storage", source))
    }

    /// Return the wrapped storage after flushing pending writes.
    pub fn into_inner(mut self) -> Result<S> {
        self.flush()?;
        Ok(self.storage)
    }

    /// Return a filesystem summary suitable for human-readable inspection.
    pub fn inspect(&mut self) -> Result<InspectReport> {
        Ok(InspectReport {
            block_size: BLOCK_SIZE,
            magic: self.super_block.magic,
            inode_count: self.super_block.inode_count,
            zone_count: self.super_block.zone_count,
            inode_bitmap_blocks: self.super_block.inode_bitmap_block_count,
            zone_bitmap_blocks: self.super_block.zone_bitmap_block_count,
            first_data_zone: self.super_block.first_data_zone,
            max_file_size: self.super_block.max_file_size,
            free_inodes: self.inode_bitmap.count_free().saturating_sub(1),
            free_zones: self.zone_bitmap.count_free().saturating_sub(1),
            root_entries: self.list_directory(ROOT_INODE_NUMBER, "/")?,
        })
    }

    /// Validate common structural invariants and return the issue report.
    pub fn check(&mut self) -> Result<CheckReport> {
        let mut report = CheckReport::default();
        let referenced_inodes = self.collect_referenced_inodes();

        if !self.inode_bitmap.is_set(ROOT_INODE_NUMBER as u32) {
            report.issues.push(CheckIssue {
                path: Some("/".into()),
                message: "root inode is not marked allocated in the inode bitmap".into(),
            });
        }

        if let Ok(root_inode) = self.read_inode(ROOT_INODE_NUMBER)
            && root_inode.mode.file_type() != Some(InodeType::Directory)
        {
            report.issues.push(CheckIssue {
                path: Some("/".into()),
                message: "root inode is not a directory".into(),
            });
        }

        for inode_number in 1..=self.super_block.inode_count {
            if !self.inode_bitmap.is_set(inode_number as u32) {
                continue;
            }

            let inode = match self.read_inode(inode_number) {
                Ok(inode) => inode,
                Err(error) => {
                    report.issues.push(CheckIssue {
                        path: Some(format!("inode {}", inode_number)),
                        message: error.to_string(),
                    });
                    continue;
                }
            };

            let Some(kind) = inode.mode.file_type() else {
                if inode.is_zeroed() && !referenced_inodes.contains(&inode_number) {
                    continue;
                }
                report.issues.push(CheckIssue {
                    path: Some(format!("inode {}", inode_number)),
                    message: format!("inode {} has an unrecognized type field", inode_number),
                });
                continue;
            };

            if matches!(kind, InodeType::Regular | InodeType::Directory) {
                self.check_inode_zones(inode_number, &inode, &mut report);
            }

            if kind == InodeType::Directory {
                self.check_directory_entries(inode_number, &inode, &mut report);
            }
        }

        Ok(report)
    }

    /// Return metadata for one path.
    pub fn stat(&mut self, path: &str) -> Result<NodeMetadata> {
        let path = ImagePath::parse(path)?;
        let inode_number = self.resolve_path(&path)?;
        self.node_metadata(&path.display(), inode_number)
    }

    /// Return a directory listing for the requested path.
    pub fn list_path(&mut self, path: &str) -> Result<Vec<DirectoryEntryInfo>> {
        let path = ImagePath::parse(path)?;
        let inode_number = self.resolve_path(&path)?;
        let inode = self.read_inode(inode_number)?;

        match inode.mode.file_type() {
            Some(InodeType::Directory) => self.list_directory(inode_number, &path.display()),
            Some(_) => Ok(vec![DirectoryEntryInfo {
                name: path.file_name().unwrap_or("/").into(),
                metadata: self.node_metadata(&path.display(), inode_number)?,
            }]),
            None => Err(MinixError::Corrupt(format!(
                "inode {} has an unrecognized type field",
                inode_number
            ))),
        }
    }

    /// Return a recursive listing for the requested path.
    pub fn tree(&mut self, path: &str) -> Result<Vec<TreeEntry>> {
        let path = ImagePath::parse(path)?;
        let inode_number = self.resolve_path(&path)?;
        let mut entries = Vec::new();
        let mut visited = HashSet::new();
        self.collect_tree_entries(&path.display(), inode_number, 0, &mut visited, &mut entries)?;
        Ok(entries)
    }

    /// Read one regular file into memory.
    pub fn read_file_at_path(&mut self, path: &str) -> Result<Vec<u8>> {
        let path = ImagePath::parse(path)?;
        let inode_number = self.resolve_path(&path)?;
        let inode = self.read_inode(inode_number)?;
        match inode.mode.file_type() {
            Some(InodeType::Regular) => self.read_inode_contents(&inode),
            Some(InodeType::Directory) => Err(MinixError::IsDirectory(format!(
                "`{}` is a directory",
                path.display()
            ))),
            Some(_) => Err(MinixError::Unsupported(format!(
                "`{}` is a special inode and cannot be read as a regular file",
                path.display()
            ))),
            None => Err(MinixError::Corrupt(format!(
                "inode {} has an unrecognized type field",
                inode_number
            ))),
        }
    }

    /// Create missing parent directories and then create or overwrite one file.
    pub fn write_file_at_path(
        &mut self,
        path: &str,
        data: &[u8],
        options: &CreateNodeOptions,
        overwrite: bool,
        parent_options: &CreateNodeOptions,
    ) -> Result<()> {
        let path = ImagePath::parse(path)?;
        if path.is_root() {
            return Err(MinixError::IsDirectory(
                "cannot write data to the root directory".into(),
            ));
        }

        let parent = path.parent().expect("non-root paths always have a parent");
        self.mkdir_all(&parent.display(), parent_options)?;
        let parent_inode = self.resolve_path(&parent)?;
        let name = path.file_name().expect("non-root paths always have a name");

        if let Some(existing_inode_number) = self.lookup_child(parent_inode, name)? {
            let mut inode = self.read_inode(existing_inode_number)?;
            if inode.mode.file_type() == Some(InodeType::Directory) {
                return Err(MinixError::IsDirectory(format!(
                    "`{}` is a directory",
                    path.display()
                )));
            }
            if !overwrite {
                return Err(MinixError::AlreadyExists(format!(
                    "`{}` already exists",
                    path.display()
                )));
            }

            inode.mode = InodeMode::regular(options.mode);
            inode.user_id = options.uid;
            inode.group_id = options.gid;
            self.write_inode_contents(existing_inode_number, &mut inode, data, options.mtime)
        } else {
            let inode_number = self.allocate_inode_number()?;
            let mut inode = DiskInode::zeroed();
            inode.mode = InodeMode::regular(options.mode);
            inode.user_id = options.uid;
            inode.group_id = options.gid;
            inode.link_count = 1;
            self.write_inode_contents(inode_number, &mut inode, data, options.mtime)?;

            if let Err(error) =
                self.add_directory_entry(parent_inode, name, inode_number, options.mtime)
            {
                self.free_inode_data(&mut inode)?;
                self.free_inode_number(inode_number)?;
                return Err(error);
            }

            Ok(())
        }
    }

    /// Create the requested directory and all missing parents.
    pub fn mkdir_all(&mut self, path: &str, options: &CreateNodeOptions) -> Result<()> {
        let path = ImagePath::parse(path)?;
        if path.is_root() {
            return Ok(());
        }

        let mut current_inode = ROOT_INODE_NUMBER;
        let mut current_path = ImagePath::root();

        for component in path.components() {
            current_path = current_path.join_name(component)?;
            match self.lookup_child(current_inode, component)? {
                Some(child_inode) => {
                    let inode = self.read_inode(child_inode)?;
                    if inode.mode.file_type() != Some(InodeType::Directory) {
                        return Err(MinixError::NotDirectory(format!(
                            "`{}` is not a directory",
                            current_path.display()
                        )));
                    }
                    current_inode = child_inode;
                }
                None => {
                    current_inode = self.create_directory(current_inode, component, options)?;
                }
            }
        }

        Ok(())
    }

    /// Create one device inode and any missing parent directories.
    pub fn create_device_at_path(
        &mut self,
        path: &str,
        device_kind: DeviceNodeKind,
        device_number: u16,
        options: &CreateNodeOptions,
        parent_options: &CreateNodeOptions,
    ) -> Result<()> {
        let path = ImagePath::parse(path)?;
        if path.is_root() {
            return Err(MinixError::IsDirectory(
                "cannot replace the root directory with a device inode".into(),
            ));
        }

        let parent = path.parent().expect("non-root paths always have a parent");
        self.mkdir_all(&parent.display(), parent_options)?;
        let parent_inode = self.resolve_path(&parent)?;
        let name = path.file_name().expect("non-root paths always have a name");

        if self.lookup_child(parent_inode, name)?.is_some() {
            return Err(MinixError::AlreadyExists(format!(
                "`{}` already exists",
                path.display()
            )));
        }

        let inode_number = self.allocate_inode_number()?;
        let mut inode = DiskInode::zeroed();
        inode.mode = match device_kind {
            DeviceNodeKind::Block => InodeMode(0o060000 | (options.mode & InodeMode::FLAGS_MASK)),
            DeviceNodeKind::Character => {
                InodeMode(0o020000 | (options.mode & InodeMode::FLAGS_MASK))
            }
        };
        inode.user_id = options.uid;
        inode.group_id = options.gid;
        inode.modification_time = options.mtime;
        inode.link_count = 1;
        inode.direct_zones[0] = device_number;
        self.write_inode(inode_number, &inode)?;

        if let Err(error) =
            self.add_directory_entry(parent_inode, name, inode_number, options.mtime)
        {
            self.free_inode_number(inode_number)?;
            return Err(error);
        }

        Ok(())
    }

    /// Remove one regular file or special-file inode from the image.
    pub fn remove_file(&mut self, path: &str) -> Result<()> {
        let path = ImagePath::parse(path)?;
        if path.is_root() {
            return Err(MinixError::IsDirectory(
                "cannot remove the root directory".into(),
            ));
        }

        let parent = path.parent().expect("non-root paths always have a parent");
        let parent_inode = self.resolve_path(&parent)?;
        let name = path.file_name().expect("non-root paths always have a name");
        let inode_number = self
            .lookup_child(parent_inode, name)?
            .ok_or_else(|| MinixError::NotFound(format!("`{}` does not exist", path.display())))?;
        let mut inode = self.read_inode(inode_number)?;
        if inode.mode.file_type() == Some(InodeType::Directory) {
            return Err(MinixError::IsDirectory(format!(
                "`{}` is a directory; use `rmdir` instead",
                path.display()
            )));
        }

        self.remove_directory_entry(parent_inode, name, Some(inode_number))?;
        self.decrement_link_or_free(inode_number, &mut inode)
    }

    /// Remove one empty directory from the image.
    pub fn remove_directory(&mut self, path: &str) -> Result<()> {
        let path = ImagePath::parse(path)?;
        if path.is_root() {
            return Err(MinixError::IsDirectory(
                "cannot remove the root directory".into(),
            ));
        }

        let inode_number = self.resolve_path(&path)?;
        let inode = self.read_inode(inode_number)?;
        if inode.mode.file_type() != Some(InodeType::Directory) {
            return Err(MinixError::NotDirectory(format!(
                "`{}` is not a directory",
                path.display()
            )));
        }

        if !self.directory_is_empty(inode_number)? {
            return Err(MinixError::DirectoryNotEmpty(format!(
                "`{}` is not empty",
                path.display()
            )));
        }

        let parent = path.parent().expect("non-root paths always have a parent");
        let parent_inode = self.resolve_path(&parent)?;
        let name = path.file_name().expect("non-root paths always have a name");
        self.remove_directory_entry(parent_inode, name, Some(inode_number))?;

        let mut parent_disk_inode = self.read_inode(parent_inode)?;
        if parent_disk_inode.link_count > 0 {
            parent_disk_inode.link_count -= 1;
        }
        self.write_inode(parent_inode, &parent_disk_inode)?;

        let mut directory_inode = self.read_inode(inode_number)?;
        self.free_inode_data(&mut directory_inode)?;
        self.free_inode_number(inode_number)
    }

    /// Rename one path within the image.
    pub fn rename_path(&mut self, source: &str, target: &str) -> Result<()> {
        let source = ImagePath::parse(source)?;
        let target = ImagePath::parse(target)?;

        if source == target {
            return Ok(());
        }

        if source.is_root() || target.is_root() {
            return Err(MinixError::InvalidArgument(
                "renaming the root directory is not supported".into(),
            ));
        }

        let source_parent = source
            .parent()
            .expect("non-root paths always have a parent");
        let target_parent = target
            .parent()
            .expect("non-root paths always have a parent");
        let source_parent_inode = self.resolve_path(&source_parent)?;
        let target_parent_inode = self.resolve_path(&target_parent)?;
        let source_name = source
            .file_name()
            .expect("non-root paths always have a name");
        let target_name = target
            .file_name()
            .expect("non-root paths always have a name");

        if self
            .lookup_child(target_parent_inode, target_name)?
            .is_some()
        {
            return Err(MinixError::AlreadyExists(format!(
                "`{}` already exists",
                target.display()
            )));
        }

        let inode_number = self
            .lookup_child(source_parent_inode, source_name)?
            .ok_or_else(|| {
                MinixError::NotFound(format!("`{}` does not exist", source.display()))
            })?;
        let inode = self.read_inode(inode_number)?;
        let is_directory = inode.mode.file_type() == Some(InodeType::Directory);

        if is_directory && is_prefix_path(&source, &target) {
            return Err(MinixError::InvalidArgument(format!(
                "cannot move `{}` into one of its descendants",
                source.display()
            )));
        }

        self.add_directory_entry(
            target_parent_inode,
            target_name,
            inode_number,
            inode.modification_time,
        )?;

        if is_directory && source_parent_inode != target_parent_inode {
            self.update_directory_parent(
                inode_number,
                target_parent_inode,
                inode.modification_time,
            )?;

            let mut new_parent = self.read_inode(target_parent_inode)?;
            new_parent.link_count = new_parent.link_count.saturating_add(1);
            self.write_inode(target_parent_inode, &new_parent)?;
        }

        self.remove_directory_entry(source_parent_inode, source_name, Some(inode_number))?;

        if is_directory && source_parent_inode != target_parent_inode {
            let mut old_parent = self.read_inode(source_parent_inode)?;
            old_parent.link_count = old_parent.link_count.saturating_sub(1);
            self.write_inode(source_parent_inode, &old_parent)?;
        }

        Ok(())
    }

    /// Create one hard link to an existing non-directory inode.
    pub fn link_path(&mut self, source: &str, target: &str) -> Result<()> {
        let source = ImagePath::parse(source)?;
        let target = ImagePath::parse(target)?;
        if target.is_root() {
            return Err(MinixError::InvalidArgument(
                "cannot create a hard link at the root path".into(),
            ));
        }

        let source_inode_number = self.resolve_path(&source)?;
        let mut source_inode = self.read_inode(source_inode_number)?;
        if source_inode.mode.file_type() == Some(InodeType::Directory) {
            return Err(MinixError::Unsupported(
                "hard links to directories are not supported".into(),
            ));
        }

        let target_parent = target
            .parent()
            .expect("non-root paths always have a parent");
        let target_parent_inode = self.resolve_path(&target_parent)?;
        let target_name = target
            .file_name()
            .expect("non-root paths always have a name");

        if self
            .lookup_child(target_parent_inode, target_name)?
            .is_some()
        {
            return Err(MinixError::AlreadyExists(format!(
                "`{}` already exists",
                target.display()
            )));
        }

        source_inode.link_count = source_inode.link_count.saturating_add(1);
        self.write_inode(source_inode_number, &source_inode)?;
        self.add_directory_entry(
            target_parent_inode,
            target_name,
            source_inode_number,
            source_inode.modification_time,
        )
    }

    /// Resolve a normalized path to one inode number.
    fn resolve_path(&mut self, path: &ImagePath) -> Result<u16> {
        if path.is_root() {
            return Ok(ROOT_INODE_NUMBER);
        }

        let mut current_inode = ROOT_INODE_NUMBER;
        for component in path.components() {
            let inode = self.read_inode(current_inode)?;
            if inode.mode.file_type() != Some(InodeType::Directory) {
                return Err(MinixError::NotDirectory(format!(
                    "`{}` is not a directory",
                    path.display()
                )));
            }

            current_inode = self
                .lookup_child(current_inode, component)?
                .ok_or_else(|| {
                    MinixError::NotFound(format!("`{}` does not exist", path.display()))
                })?;
        }

        Ok(current_inode)
    }

    /// Return a listing for one directory inode.
    fn list_directory(
        &mut self,
        inode_number: u16,
        base_path: &str,
    ) -> Result<Vec<DirectoryEntryInfo>> {
        let entries = self.read_directory_entries(inode_number)?;
        let mut listing = Vec::new();

        for entry in entries {
            if entry.inode_number == 0 {
                continue;
            }

            let name = entry.name()?;
            if name == "." || name == ".." {
                continue;
            }

            let child_path = if base_path == "/" {
                format!("/{}", name)
            } else {
                format!("{}/{}", base_path, name)
            };
            listing.push(DirectoryEntryInfo {
                name,
                metadata: self.node_metadata(&child_path, entry.inode_number)?,
            });
        }

        listing.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(listing)
    }

    /// Recursively collect tree entries in depth-first order.
    fn collect_tree_entries(
        &mut self,
        path: &str,
        inode_number: u16,
        depth: usize,
        visited: &mut HashSet<u16>,
        entries: &mut Vec<TreeEntry>,
    ) -> Result<()> {
        let metadata = self.node_metadata(path, inode_number)?;
        let kind = metadata.kind;
        entries.push(TreeEntry { depth, metadata });

        if kind != InodeType::Directory || !visited.insert(inode_number) {
            return Ok(());
        }

        for child in self.list_directory(inode_number, path)? {
            self.collect_tree_entries(
                &child.metadata.path,
                child.metadata.inode_number,
                depth + 1,
                visited,
                entries,
            )?;
        }

        Ok(())
    }

    /// Gather every inode number referenced by a directory entry.
    fn collect_referenced_inodes(&mut self) -> HashSet<u16> {
        let mut referenced = HashSet::new();

        for inode_number in 1..=self.super_block.inode_count {
            if !self.inode_bitmap.is_set(inode_number as u32) {
                continue;
            }

            let Ok(inode) = self.read_inode(inode_number) else {
                continue;
            };

            if inode.mode.file_type() != Some(InodeType::Directory) {
                continue;
            }

            let Ok(entries) = self.read_directory_entries(inode_number) else {
                continue;
            };

            for entry in entries.into_iter().filter(|entry| entry.inode_number != 0) {
                referenced.insert(entry.inode_number);
            }
        }

        referenced
    }

    /// Return metadata for one inode number.
    fn node_metadata(&mut self, path: &str, inode_number: u16) -> Result<NodeMetadata> {
        let inode = self.read_inode(inode_number)?;
        let kind = inode.mode.file_type().ok_or_else(|| {
            MinixError::Corrupt(format!(
                "inode {} has an unrecognized type field",
                inode_number
            ))
        })?;

        Ok(NodeMetadata {
            path: path.into(),
            inode_number,
            kind,
            mode: inode.mode.0,
            uid: inode.user_id,
            gid: inode.group_id,
            size: inode.size,
            link_count: inode.link_count,
            modification_time: inode.modification_time,
            device_number: matches!(kind, InodeType::BlockDevice | InodeType::CharacterDevice)
                .then_some(inode.direct_zones[0]),
        })
    }

    /// Read one inode from the inode table.
    fn read_inode(&mut self, inode_number: u16) -> Result<DiskInode> {
        if inode_number == 0 || inode_number > self.super_block.inode_count {
            return Err(MinixError::Corrupt(format!(
                "inode {} is outside the super-block inode range",
                inode_number
            )));
        }

        let (block_number, slot_index) = self.inode_block_position(inode_number);
        let mut block = [0_u8; BLOCK_SIZE];
        self.read_block(block_number, &mut block)?;
        let start = slot_index * crate::layout::DISK_INODE_SIZE;
        let end = start + crate::layout::DISK_INODE_SIZE;
        DiskInode::decode(&block[start..end])
    }

    /// Write one inode back to the inode table.
    fn write_inode(&mut self, inode_number: u16, inode: &DiskInode) -> Result<()> {
        let (block_number, slot_index) = self.inode_block_position(inode_number);
        let mut block = [0_u8; BLOCK_SIZE];
        self.read_block(block_number, &mut block)?;
        let start = slot_index * crate::layout::DISK_INODE_SIZE;
        let end = start + crate::layout::DISK_INODE_SIZE;
        inode.encode(&mut block[start..end]);
        self.write_block(block_number, &block)
    }

    /// Return the inode-table block and slot index for one inode number.
    fn inode_block_position(&self, inode_number: u16) -> (u32, usize) {
        let zero_based = inode_number as usize - 1;
        let block_number = 2
            + self.super_block.inode_bitmap_block_count as u32
            + self.super_block.zone_bitmap_block_count as u32
            + (zero_based / INODES_PER_BLOCK) as u32;
        let slot_index = zero_based % INODES_PER_BLOCK;
        (block_number, slot_index)
    }

    /// Sync the super block to disk.
    fn sync_super_block(&mut self) -> Result<()> {
        let mut block = [0_u8; BLOCK_SIZE];
        self.super_block.encode(&mut block);
        self.write_block(1, &block)
    }

    /// Sync the inode bitmap blocks.
    fn sync_inode_bitmap(&mut self) -> Result<()> {
        let blocks = self.inode_bitmap.blocks().to_vec();
        for (index, block) in blocks.iter().enumerate() {
            self.write_block(2 + index as u32, block)?;
        }
        Ok(())
    }

    /// Sync the zone bitmap blocks.
    fn sync_zone_bitmap(&mut self) -> Result<()> {
        let start_block = 2 + self.super_block.inode_bitmap_block_count as u32;
        let blocks = self.zone_bitmap.blocks().to_vec();
        for (index, block) in blocks.iter().enumerate() {
            self.write_block(start_block + index as u32, block)?;
        }
        Ok(())
    }

    /// Allocate one inode number and mark it in the inode bitmap.
    fn allocate_inode_number(&mut self) -> Result<u16> {
        let value = self
            .inode_bitmap
            .alloc()
            .ok_or_else(|| MinixError::NoSpace("no free inodes remain".into()))?;
        self.sync_inode_bitmap()?;
        Ok(value as u16)
    }

    /// Free one inode number and clear its table slot.
    fn free_inode_number(&mut self, inode_number: u16) -> Result<()> {
        self.write_inode(inode_number, &DiskInode::zeroed())?;
        self.inode_bitmap.clear(inode_number as u32);
        self.sync_inode_bitmap()
    }

    /// Allocate one data zone and zero its backing block.
    fn allocate_zone(&mut self) -> Result<u16> {
        let value = self
            .zone_bitmap
            .alloc()
            .ok_or_else(|| MinixError::NoSpace("no free data zones remain".into()))?;
        self.sync_zone_bitmap()?;
        self.write_zero_block(value as u16)?;
        Ok(value as u16)
    }

    /// Free one data zone.
    fn free_zone(&mut self, zone: u16) -> Result<()> {
        if zone != 0 {
            self.zone_bitmap.clear(zone as u32);
            self.sync_zone_bitmap()?;
        }
        Ok(())
    }

    /// Read one block from the image storage.
    fn read_block(&mut self, block_number: u32, block: &mut [u8; BLOCK_SIZE]) -> Result<()> {
        read_block_from(&mut self.storage, block_number, block)
    }

    /// Write one block to the image storage.
    fn write_block(&mut self, block_number: u32, block: &[u8; BLOCK_SIZE]) -> Result<()> {
        write_block_to(&mut self.storage, block_number, block)
    }

    /// Write one zero-filled block to the image storage.
    fn write_zero_block(&mut self, block_number: u16) -> Result<()> {
        write_zero_block_to(&mut self.storage, block_number as u32)
    }

    /// Read all bytes referenced by one inode.
    fn read_inode_contents(&mut self, inode: &DiskInode) -> Result<Vec<u8>> {
        let mut data = vec![0_u8; inode.size as usize];
        let block_count = (inode.size as usize).div_ceil(BLOCK_SIZE);

        for logical_block in 0..block_count {
            let zone = self.lookup_data_zone(inode, logical_block)?;
            if zone == 0 {
                continue;
            }

            let mut block = [0_u8; BLOCK_SIZE];
            self.read_block(zone as u32, &mut block)?;
            let start = logical_block * BLOCK_SIZE;
            let end = data.len().min(start + BLOCK_SIZE);
            data[start..end].copy_from_slice(&block[..end - start]);
        }

        Ok(data)
    }

    /// Rewrite all file data referenced by one inode.
    fn write_inode_contents(
        &mut self,
        inode_number: u16,
        inode: &mut DiskInode,
        data: &[u8],
        mtime: u32,
    ) -> Result<()> {
        self.free_inode_data(inode)?;

        let block_count = data.len().div_ceil(BLOCK_SIZE);
        let result = (|| -> Result<()> {
            for logical_block in 0..block_count {
                let zone = self.allocate_zone()?;
                self.set_data_zone(inode, logical_block, zone)?;
                let start = logical_block * BLOCK_SIZE;
                let end = data.len().min(start + BLOCK_SIZE);
                let mut block = [0_u8; BLOCK_SIZE];
                block[..end - start].copy_from_slice(&data[start..end]);
                self.write_block(zone as u32, &block)?;
            }
            Ok(())
        })();

        if let Err(error) = result {
            self.free_inode_data(inode)?;
            inode.size = 0;
            inode.modification_time = mtime;
            self.write_inode(inode_number, inode)?;
            return Err(error);
        }

        inode.size = data.len() as u32;
        inode.modification_time = mtime;
        self.write_inode(inode_number, inode)
    }

    /// Free all data and metadata zones referenced by one inode.
    fn free_inode_data(&mut self, inode: &mut DiskInode) -> Result<()> {
        for zone in &mut inode.direct_zones {
            if *zone != 0 {
                self.free_zone(*zone)?;
                *zone = 0;
            }
        }

        if inode.single_indirect_zone != 0 {
            let table = self.read_indirect_block(inode.single_indirect_zone)?;
            for zone in table.into_iter().filter(|zone| *zone != 0) {
                self.free_zone(zone)?;
            }
            self.free_zone(inode.single_indirect_zone)?;
            inode.single_indirect_zone = 0;
        }

        if inode.double_indirect_zone != 0 {
            let outer_table = self.read_indirect_block(inode.double_indirect_zone)?;
            for inner_zone in outer_table.into_iter().filter(|zone| *zone != 0) {
                let inner_table = self.read_indirect_block(inner_zone)?;
                for zone in inner_table.into_iter().filter(|zone| *zone != 0) {
                    self.free_zone(zone)?;
                }
                self.free_zone(inner_zone)?;
            }
            self.free_zone(inode.double_indirect_zone)?;
            inode.double_indirect_zone = 0;
        }

        inode.size = 0;
        Ok(())
    }

    /// Return the data zone backing one logical block.
    fn lookup_data_zone(&mut self, inode: &DiskInode, logical_block: usize) -> Result<u16> {
        if logical_block >= MAX_LOGICAL_BLOCKS {
            return Err(MinixError::InvalidArgument(format!(
                "logical block {} exceeds the Minix addressing limit",
                logical_block
            )));
        }

        if logical_block < DIRECT_ZONE_COUNT {
            return Ok(inode.direct_zones[logical_block]);
        }

        let logical_block = logical_block - DIRECT_ZONE_COUNT;
        if logical_block < INDIRECT_ENTRY_COUNT {
            if inode.single_indirect_zone == 0 {
                return Ok(0);
            }
            let table = self.read_indirect_block(inode.single_indirect_zone)?;
            return Ok(table[logical_block]);
        }

        let logical_block = logical_block - INDIRECT_ENTRY_COUNT;
        if inode.double_indirect_zone == 0 {
            return Ok(0);
        }

        let outer_table = self.read_indirect_block(inode.double_indirect_zone)?;
        let outer_index = logical_block / INDIRECT_ENTRY_COUNT;
        let inner_index = logical_block % INDIRECT_ENTRY_COUNT;
        let inner_zone = outer_table[outer_index];
        if inner_zone == 0 {
            return Ok(0);
        }
        let inner_table = self.read_indirect_block(inner_zone)?;
        Ok(inner_table[inner_index])
    }

    /// Attach one data zone to one logical block, allocating indirect tables as needed.
    fn set_data_zone(
        &mut self,
        inode: &mut DiskInode,
        logical_block: usize,
        zone: u16,
    ) -> Result<()> {
        if logical_block >= MAX_LOGICAL_BLOCKS {
            return Err(MinixError::InvalidArgument(format!(
                "logical block {} exceeds the Minix addressing limit",
                logical_block
            )));
        }

        if logical_block < DIRECT_ZONE_COUNT {
            inode.direct_zones[logical_block] = zone;
            return Ok(());
        }

        let logical_block = logical_block - DIRECT_ZONE_COUNT;
        if logical_block < INDIRECT_ENTRY_COUNT {
            if inode.single_indirect_zone == 0 {
                inode.single_indirect_zone = self.allocate_zone()?;
            }
            let mut table = self.read_indirect_block(inode.single_indirect_zone)?;
            table[logical_block] = zone;
            self.write_indirect_block(inode.single_indirect_zone, &table)?;
            return Ok(());
        }

        let logical_block = logical_block - INDIRECT_ENTRY_COUNT;
        let outer_index = logical_block / INDIRECT_ENTRY_COUNT;
        let inner_index = logical_block % INDIRECT_ENTRY_COUNT;

        if inode.double_indirect_zone == 0 {
            inode.double_indirect_zone = self.allocate_zone()?;
        }

        let mut outer_table = self.read_indirect_block(inode.double_indirect_zone)?;
        if outer_table[outer_index] == 0 {
            outer_table[outer_index] = self.allocate_zone()?;
            self.write_indirect_block(inode.double_indirect_zone, &outer_table)?;
        }

        let inner_zone = outer_table[outer_index];
        let mut inner_table = self.read_indirect_block(inner_zone)?;
        inner_table[inner_index] = zone;
        self.write_indirect_block(inner_zone, &inner_table)
    }

    /// Read one indirect block as a table of 16-bit zones.
    fn read_indirect_block(&mut self, zone: u16) -> Result<[u16; INDIRECT_ENTRY_COUNT]> {
        let mut block = [0_u8; BLOCK_SIZE];
        self.read_block(zone as u32, &mut block)?;
        let mut table = [0_u16; INDIRECT_ENTRY_COUNT];
        for (index, slot) in table.iter_mut().enumerate() {
            let offset = index * 2;
            *slot = u16::from_le_bytes([block[offset], block[offset + 1]]);
        }
        Ok(table)
    }

    /// Write one indirect block from a table of 16-bit zones.
    fn write_indirect_block(
        &mut self,
        zone: u16,
        table: &[u16; INDIRECT_ENTRY_COUNT],
    ) -> Result<()> {
        let mut block = [0_u8; BLOCK_SIZE];
        for (index, entry) in table.iter().enumerate() {
            let offset = index * 2;
            block[offset..offset + 2].copy_from_slice(&entry.to_le_bytes());
        }
        self.write_block(zone as u32, &block)
    }

    /// Look up one child entry inside a directory.
    fn lookup_child(&mut self, directory_inode: u16, name: &str) -> Result<Option<u16>> {
        for entry in self.read_directory_entries(directory_inode)? {
            if entry.inode_number == 0 {
                continue;
            }
            if entry.name()? == name {
                return Ok(Some(entry.inode_number));
            }
        }
        Ok(None)
    }

    /// Read and decode one directory's entries.
    fn read_directory_entries(&mut self, inode_number: u16) -> Result<Vec<DiskDirectoryEntry>> {
        let inode = self.read_inode(inode_number)?;
        if inode.mode.file_type() != Some(InodeType::Directory) {
            return Err(MinixError::NotDirectory(format!(
                "inode {} is not a directory",
                inode_number
            )));
        }

        let bytes = self.read_inode_contents(&inode)?;
        if bytes.len() % DIRECTORY_ENTRY_SIZE != 0 {
            return Err(MinixError::Corrupt(format!(
                "directory inode {} has a size that is not aligned to {} bytes",
                inode_number, DIRECTORY_ENTRY_SIZE
            )));
        }

        let mut entries = Vec::with_capacity(bytes.len() / DIRECTORY_ENTRY_SIZE);
        for chunk in bytes.chunks_exact(DIRECTORY_ENTRY_SIZE) {
            entries.push(DiskDirectoryEntry::decode(chunk)?);
        }
        Ok(entries)
    }

    /// Encode and write one full directory-entry array using a preloaded inode.
    fn write_directory_entries_with_inode(
        &mut self,
        inode_number: u16,
        inode: &mut DiskInode,
        entries: &[DiskDirectoryEntry],
        mtime: u32,
    ) -> Result<()> {
        let mut bytes = vec![0_u8; entries.len() * DIRECTORY_ENTRY_SIZE];
        for (index, entry) in entries.iter().enumerate() {
            let start = index * DIRECTORY_ENTRY_SIZE;
            let end = start + DIRECTORY_ENTRY_SIZE;
            entry.encode(&mut bytes[start..end]);
        }
        self.write_inode_contents(inode_number, inode, &bytes, mtime)
    }

    /// Encode and write one full directory-entry array.
    fn write_directory_entries(
        &mut self,
        inode_number: u16,
        entries: &[DiskDirectoryEntry],
        mtime: u32,
    ) -> Result<()> {
        let mut inode = self.read_inode(inode_number)?;
        self.write_directory_entries_with_inode(inode_number, &mut inode, entries, mtime)
    }

    /// Create a new directory below one parent directory.
    fn create_directory(
        &mut self,
        parent_inode_number: u16,
        name: &str,
        options: &CreateNodeOptions,
    ) -> Result<u16> {
        let inode_number = self.allocate_inode_number()?;
        let zone = self.allocate_zone()?;

        let mut inode = DiskInode::zeroed();
        inode.mode = InodeMode::directory(options.mode);
        inode.user_id = options.uid;
        inode.group_id = options.gid;
        inode.modification_time = options.mtime;
        inode.link_count = 2;
        inode.direct_zones[0] = zone;

        let entries = vec![
            DiskDirectoryEntry::new(".", inode_number)?,
            DiskDirectoryEntry::new("..", parent_inode_number)?,
        ];
        self.write_directory_entries_with_inode(inode_number, &mut inode, &entries, options.mtime)?;

        if let Err(error) =
            self.add_directory_entry(parent_inode_number, name, inode_number, options.mtime)
        {
            self.free_inode_data(&mut inode)?;
            self.free_inode_number(inode_number)?;
            return Err(error);
        }

        let mut parent_inode = self.read_inode(parent_inode_number)?;
        parent_inode.link_count = parent_inode.link_count.saturating_add(1);
        self.write_inode(parent_inode_number, &parent_inode)?;
        Ok(inode_number)
    }

    /// Add one child entry to a directory.
    fn add_directory_entry(
        &mut self,
        directory_inode: u16,
        name: &str,
        child_inode: u16,
        mtime: u32,
    ) -> Result<()> {
        let mut entries = self.read_directory_entries(directory_inode)?;
        for entry in &entries {
            if entry.inode_number != 0 && entry.name()? == name {
                return Err(MinixError::AlreadyExists(format!(
                    "directory entry `{}` already exists",
                    name
                )));
            }
        }

        if let Some(empty_slot) = entries.iter_mut().find(|entry| entry.inode_number == 0) {
            *empty_slot = DiskDirectoryEntry::new(name, child_inode)?;
        } else {
            entries.push(DiskDirectoryEntry::new(name, child_inode)?);
        }

        self.write_directory_entries(directory_inode, &entries, mtime)
    }

    /// Remove one child entry from a directory and return the referenced inode number.
    fn remove_directory_entry(
        &mut self,
        directory_inode: u16,
        name: &str,
        expected_inode: Option<u16>,
    ) -> Result<u16> {
        let mut entries = self.read_directory_entries(directory_inode)?;
        let mut removed_inode = None;

        for entry in &mut entries {
            if entry.inode_number == 0 {
                continue;
            }

            if entry.name()? == name {
                if let Some(expected_inode) = expected_inode
                    && entry.inode_number != expected_inode
                {
                    return Err(MinixError::Corrupt(format!(
                        "directory entry `{}` points to inode {}, expected {}",
                        name, entry.inode_number, expected_inode
                    )));
                }
                removed_inode = Some(entry.inode_number);
                *entry = DiskDirectoryEntry::empty();
                break;
            }
        }

        let removed_inode = removed_inode.ok_or_else(|| {
            MinixError::NotFound(format!("directory entry `{}` does not exist", name))
        })?;
        let directory_mtime = self.read_inode(directory_inode)?.modification_time;
        self.write_directory_entries(directory_inode, &entries, directory_mtime)?;
        Ok(removed_inode)
    }

    /// Return whether a directory contains any non-dot entries.
    fn directory_is_empty(&mut self, inode_number: u16) -> Result<bool> {
        for entry in self.read_directory_entries(inode_number)? {
            if entry.inode_number == 0 {
                continue;
            }
            let name = entry.name()?;
            if name != "." && name != ".." {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Update a moved directory's `..` entry.
    fn update_directory_parent(
        &mut self,
        inode_number: u16,
        new_parent_inode: u16,
        mtime: u32,
    ) -> Result<()> {
        let mut entries = self.read_directory_entries(inode_number)?;
        for entry in &mut entries {
            if entry.inode_number == 0 {
                continue;
            }
            if entry.name()? == ".." {
                entry.inode_number = new_parent_inode;
                self.write_directory_entries(inode_number, &entries, mtime)?;
                return Ok(());
            }
        }

        Err(MinixError::Corrupt(format!(
            "directory inode {} is missing its `..` entry",
            inode_number
        )))
    }

    /// Decrement one inode's link count, freeing it when the count reaches zero.
    fn decrement_link_or_free(&mut self, inode_number: u16, inode: &mut DiskInode) -> Result<()> {
        if inode.link_count > 1 {
            inode.link_count -= 1;
            self.write_inode(inode_number, inode)
        } else {
            self.free_inode_data(inode)?;
            self.free_inode_number(inode_number)
        }
    }

    /// Validate all zones referenced by one inode.
    fn check_inode_zones(
        &mut self,
        inode_number: u16,
        inode: &DiskInode,
        report: &mut CheckReport,
    ) {
        let push_issue = |report: &mut CheckReport, message: String| {
            report.issues.push(CheckIssue {
                path: Some(format!("inode {}", inode_number)),
                message,
            });
        };

        for zone in inode.direct_zones.into_iter().filter(|zone| *zone != 0) {
            if !self.zone_in_range(zone) {
                push_issue(
                    report,
                    format!("direct zone {} is outside the valid data-zone range", zone),
                );
            }
        }

        if inode.single_indirect_zone != 0 {
            if !self.zone_in_range(inode.single_indirect_zone) {
                push_issue(
                    report,
                    format!(
                        "single-indirect zone {} is outside the valid data-zone range",
                        inode.single_indirect_zone
                    ),
                );
            } else if let Ok(table) = self.read_indirect_block(inode.single_indirect_zone) {
                for zone in table.into_iter().filter(|zone| *zone != 0) {
                    if !self.zone_in_range(zone) {
                        push_issue(
                            report,
                            format!(
                                "indirect data zone {} is outside the valid data-zone range",
                                zone
                            ),
                        );
                    }
                }
            }
        }

        if inode.double_indirect_zone != 0 {
            if !self.zone_in_range(inode.double_indirect_zone) {
                push_issue(
                    report,
                    format!(
                        "double-indirect zone {} is outside the valid data-zone range",
                        inode.double_indirect_zone
                    ),
                );
                return;
            }

            if let Ok(outer_table) = self.read_indirect_block(inode.double_indirect_zone) {
                for inner_zone in outer_table.into_iter().filter(|zone| *zone != 0) {
                    if !self.zone_in_range(inner_zone) {
                        push_issue(
                            report,
                            format!(
                                "double-indirect child zone {} is outside the valid data-zone range",
                                inner_zone
                            ),
                        );
                        continue;
                    }

                    if let Ok(inner_table) = self.read_indirect_block(inner_zone) {
                        for zone in inner_table.into_iter().filter(|zone| *zone != 0) {
                            if !self.zone_in_range(zone) {
                                push_issue(
                                    report,
                                    format!(
                                        "double-indirect data zone {} is outside the valid data-zone range",
                                        zone
                                    ),
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    /// Validate directory entries and inode references in one directory.
    fn check_directory_entries(
        &mut self,
        inode_number: u16,
        inode: &DiskInode,
        report: &mut CheckReport,
    ) {
        if inode.size as usize % DIRECTORY_ENTRY_SIZE != 0 {
            report.issues.push(CheckIssue {
                path: Some(format!("inode {}", inode_number)),
                message: format!(
                    "directory size {} is not aligned to {} bytes",
                    inode.size, DIRECTORY_ENTRY_SIZE
                ),
            });
            return;
        }

        match self.read_directory_entries(inode_number) {
            Ok(entries) => {
                for entry in entries.into_iter().filter(|entry| entry.inode_number != 0) {
                    let name = match entry.name() {
                        Ok(name) => name,
                        Err(error) => {
                            report.issues.push(CheckIssue {
                                path: Some(format!("inode {}", inode_number)),
                                message: error.to_string(),
                            });
                            continue;
                        }
                    };

                    if name.len() > crate::layout::MINIX_NAME_LENGTH {
                        report.issues.push(CheckIssue {
                            path: Some(format!("inode {} entry {}", inode_number, name)),
                            message: "directory entry name exceeds the Minix file-name limit"
                                .into(),
                        });
                    }

                    if entry.inode_number > self.super_block.inode_count || entry.inode_number == 0
                    {
                        report.issues.push(CheckIssue {
                            path: Some(format!("inode {} entry {}", inode_number, name)),
                            message: format!(
                                "directory entry references inode {} outside the valid range",
                                entry.inode_number
                            ),
                        });
                    }
                }
            }
            Err(error) => report.issues.push(CheckIssue {
                path: Some(format!("inode {}", inode_number)),
                message: error.to_string(),
            }),
        }
    }

    /// Return whether a zone number lies within the valid data-zone range.
    fn zone_in_range(&self, zone: u16) -> bool {
        zone >= self.super_block.first_data_zone && zone < self.super_block.zone_count
    }
}

/// Return whether `source` is a strict path prefix of `target`.
fn is_prefix_path(source: &ImagePath, target: &ImagePath) -> bool {
    let source_components = source.components();
    let target_components = target.components();
    source_components.len() < target_components.len()
        && target_components.starts_with(source_components)
}

/// Read one block from the provided storage object.
fn read_block_from<S: Read + Seek>(
    storage: &mut S,
    block_number: u32,
    block: &mut [u8; BLOCK_SIZE],
) -> Result<()> {
    storage
        .seek(SeekFrom::Start(block_number as u64 * BLOCK_SIZE as u64))
        .map_err(|source| MinixError::io(None, "failed to seek image block", source))?;
    storage
        .read_exact(block)
        .map_err(|source| MinixError::io(None, "failed to read image block", source))
}

/// Write one block to the provided storage object.
fn write_block_to<S: Write + Seek>(
    storage: &mut S,
    block_number: u32,
    block: &[u8; BLOCK_SIZE],
) -> Result<()> {
    storage
        .seek(SeekFrom::Start(block_number as u64 * BLOCK_SIZE as u64))
        .map_err(|source| MinixError::io(None, "failed to seek image block", source))?;
    storage
        .write_all(block)
        .map_err(|source| MinixError::io(None, "failed to write image block", source))
}

/// Write one zero-filled block to the provided storage object.
fn write_zero_block_to<S: Write + Seek>(storage: &mut S, block_number: u32) -> Result<()> {
    write_block_to(storage, block_number, &[0_u8; BLOCK_SIZE])
}

#[cfg(test)]
mod tests {
    //! Integration-style filesystem unit tests built around in-memory images.

    use std::io::Cursor;

    use super::*;

    /// Create one fresh in-memory filesystem for testing.
    fn fresh_fs(image_size: u64, inode_count: u16) -> MinixFileSystem<Cursor<Vec<u8>>> {
        let storage = Cursor::new(Vec::new());
        MinixFileSystem::create(
            storage,
            CreateImageOptions {
                image_size,
                inode_count,
                root_mode: 0o755,
                default_uid: 0,
                default_gid: 0,
                default_mtime: 1,
            },
        )
        .unwrap()
    }

    /// Confirm a fresh image contains the expected root dot entries.
    #[test]
    fn create_initializes_the_root_directory() {
        let mut fs = fresh_fs(1024 * 128, 64);
        let report = fs.check().unwrap();
        assert!(report.is_clean());

        let root_entries = fs.read_directory_entries(ROOT_INODE_NUMBER).unwrap();
        let names = root_entries
            .into_iter()
            .filter(|entry| entry.inode_number != 0)
            .map(|entry| entry.name().unwrap())
            .collect::<Vec<_>>();
        assert!(names.contains(&".".into()));
        assert!(names.contains(&"..".into()));
    }

    /// Confirm file and directory operations round-trip through the image.
    #[test]
    fn file_and_directory_operations_round_trip() {
        let mut fs = fresh_fs(1024 * 1024, 128);
        let dir = CreateNodeOptions {
            mode: 0o755,
            uid: 0,
            gid: 0,
            mtime: 10,
        };
        let file = CreateNodeOptions {
            mode: 0o644,
            uid: 0,
            gid: 0,
            mtime: 11,
        };

        fs.mkdir_all("/bin/tools", &dir).unwrap();
        fs.write_file_at_path("/bin/tools/hello", b"hello", &file, false, &dir)
            .unwrap();
        fs.link_path("/bin/tools/hello", "/bin/tools/hello-link")
            .unwrap();
        assert_eq!(
            fs.read_file_at_path("/bin/tools/hello-link").unwrap(),
            b"hello"
        );

        fs.rename_path("/bin/tools/hello-link", "/bin/hello-link")
            .unwrap();
        assert_eq!(fs.read_file_at_path("/bin/hello-link").unwrap(), b"hello");

        fs.remove_file("/bin/tools/hello").unwrap();
        assert_eq!(fs.read_file_at_path("/bin/hello-link").unwrap(), b"hello");
        fs.remove_file("/bin/hello-link").unwrap();

        fs.remove_directory("/bin/tools").unwrap();
        let listing = fs.list_path("/bin").unwrap();
        assert!(listing.is_empty());
    }

    /// Confirm writing across direct, indirect, and double-indirect ranges works.
    #[test]
    fn large_files_cross_all_addressing_levels() {
        let mut fs = fresh_fs(1024 * 1024 * 4, 512);
        let dir = CreateNodeOptions {
            mode: 0o755,
            uid: 0,
            gid: 0,
            mtime: 20,
        };
        let file = CreateNodeOptions {
            mode: 0o644,
            uid: 0,
            gid: 0,
            mtime: 21,
        };

        let block_count = DIRECT_ZONE_COUNT + INDIRECT_ENTRY_COUNT + 4;
        let mut data = vec![0_u8; block_count * BLOCK_SIZE];
        for (index, byte) in data.iter_mut().enumerate() {
            *byte = (index % 251) as u8;
        }

        fs.write_file_at_path("/big.bin", &data, &file, false, &dir)
            .unwrap();
        assert_eq!(fs.read_file_at_path("/big.bin").unwrap(), data);
    }

    /// Confirm the filesystem reports common user-facing error cases.
    #[test]
    fn invalid_operations_return_expected_errors() {
        let mut fs = fresh_fs(1024 * 128, 8);
        let dir = CreateNodeOptions {
            mode: 0o755,
            uid: 0,
            gid: 0,
            mtime: 30,
        };
        let file = CreateNodeOptions {
            mode: 0o644,
            uid: 0,
            gid: 0,
            mtime: 31,
        };

        fs.write_file_at_path("/foo", b"x", &file, false, &dir)
            .unwrap();
        let error = fs
            .write_file_at_path("/foo", b"y", &file, false, &dir)
            .unwrap_err();
        assert!(matches!(error, MinixError::AlreadyExists(_)));

        fs.mkdir_all("/dir", &dir).unwrap();
        fs.write_file_at_path("/dir/file", b"z", &file, false, &dir)
            .unwrap();
        let error = fs.remove_directory("/dir").unwrap_err();
        assert!(matches!(error, MinixError::DirectoryNotEmpty(_)));
    }

    /// Confirm `check` ignores garbage zone fields inside device inodes.
    #[test]
    fn check_skips_zone_validation_for_device_inodes() {
        let mut fs = fresh_fs(1024 * 256, 64);
        let dir = CreateNodeOptions {
            mode: 0o755,
            uid: 0,
            gid: 0,
            mtime: 40,
        };
        let node = CreateNodeOptions {
            mode: 0o644,
            uid: 0,
            gid: 0,
            mtime: 41,
        };

        fs.create_device_at_path("/dev/hd2", DeviceNodeKind::Block, 0x0312, &node, &dir)
            .unwrap();
        let inode_number = fs
            .resolve_path(&ImagePath::parse("/dev/hd2").unwrap())
            .unwrap();
        let mut inode = fs.read_inode(inode_number).unwrap();
        inode.size = 67_608_576;
        inode.direct_zones[1] = 540;
        inode.single_indirect_zone = 541;
        inode.double_indirect_zone = 259;
        fs.write_inode(inode_number, &inode).unwrap();

        let report = fs.check().unwrap();
        assert!(report.is_clean());
    }

    /// Confirm `check` tolerates an allocated but zero-filled, unreferenced inode slot.
    #[test]
    fn check_tolerates_unreferenced_zero_filled_allocated_inodes() {
        let mut fs = fresh_fs(1024 * 128, 64);
        let leaked_inode = fs.allocate_inode_number().unwrap();
        assert_eq!(leaked_inode, 2);

        let report = fs.check().unwrap();
        assert!(report.is_clean());
    }
}
