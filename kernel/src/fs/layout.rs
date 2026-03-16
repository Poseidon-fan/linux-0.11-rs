use crate::fs::BLOCK_SIZE;

const NAME_LEN_LIMIT: usize = 14;

type BitmapBlock = [u8; BLOCK_SIZE];
type ZoneBlock = [u8; BLOCK_SIZE];

#[repr(C)]
struct SuperBlock {
    inode_num: u16,
    zone_num: u16,
    inode_bitmap_blocks: u16,
    zone_bitmap_blocks: u16,
    first_data_zone: u16,
    log_zone_size: u16,
    max_file_size: u32,
    magic: u16,
}

#[repr(C)]
struct DiskInode {
    mode: u16,
    uid: u16,
    size: u32,
    time: u32,
    gid: u16,
    link_count: u16,
    direct: [u16; 7],
    indirect1: u16,
    indirect2: u16,
}

#[repr(C)]
struct DirEntry {
    inode_number: u16,
    name: [u8; NAME_LEN_LIMIT],
}
