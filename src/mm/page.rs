#![allow(dead_code)]

use alloc::vec::Vec;
use bitflags::bitflags;

use crate::mm::{address::PhysPageNum, frame::PhysFrame};

bitflags! {
    struct PageFlags: u32 {
        const PRESENT       = 1 << 0;
        const WRITABLE      = 1 << 1;
        const USER          = 1 << 2;
        const WRITE_THROUGH = 1 << 3;
        const CACHE_DISABLE = 1 << 4;
        const ACCESSED      = 1 << 5;
        const DIRTY         = 1 << 6;
        const HUGE_PAGE     = 1 << 7;
        const GLOBAL        = 1 << 8;
    }
}

#[repr(C)]
#[derive(Copy, Clone)]
struct PageTableEntry(u32);

#[repr(C)]
#[derive(Copy, Clone)]
struct PageDirectoryEntry(u32);

trait PageEntry: Sized + From<u32> + Into<u32> + Copy + Clone {
    fn new(ppn: PhysPageNum, flags: PageFlags) -> Self {
        ((ppn.0 << 12) | flags.bits()).into()
    }

    fn ppn(&self) -> PhysPageNum {
        ((*self).into() >> 12).into()
    }

    fn flags(&self) -> PageFlags {
        PageFlags::from_bits((*self).into()).unwrap()
    }
}

struct PageTable {
    root_ppn: PhysPageNum,
    frames: Vec<PhysFrame>,
}

impl From<u32> for PageTableEntry {
    fn from(value: u32) -> Self {
        Self(value)
    }
}

impl From<PageTableEntry> for u32 {
    fn from(val: PageTableEntry) -> Self {
        val.0
    }
}

impl From<u32> for PageDirectoryEntry {
    fn from(value: u32) -> Self {
        Self(value)
    }
}

impl From<PageDirectoryEntry> for u32 {
    fn from(val: PageDirectoryEntry) -> Self {
        val.0
    }
}

impl PageEntry for PageTableEntry {}
impl PageEntry for PageDirectoryEntry {}
