use alloc::collections::btree_map::BTreeMap;

use crate::mm::{address::LinPageNum, frame::PhysFrame, page::PageTable};

pub struct MemorySpace {
    page_tables: [Option<PageTable>; 16],
    data_frames: BTreeMap<LinPageNum, PhysFrame>,
}
