#![allow(dead_code)]

use crate::{mm::MemorySpace, sync::KernelCell};

pub struct TaskControlBlock {
    pub pid: u32,
    // mutable fields
    inner: KernelCell<TaskControlBlockInner>,
}

pub struct TaskControlBlockInner {
    pub sched: TaskSchedInfo,
    pub memory_space: MemorySpace,
    pub exit_code: i32,
    pub tss: TaskStateSegment,
}

/// x87 FPU (Math Coprocessor) state structure.
#[repr(C)]
#[derive(Debug, Default)]
pub struct I387Struct {
    /// Control word
    pub cwd: u32,
    /// Status word
    pub swd: u32,
    /// Tag word
    pub twd: u32,
    /// FPU instruction pointer
    pub fip: u32,
    /// FPU instruction pointer selector
    pub fcs: u32,
    /// FPU operand pointer
    pub foo: u32,
    /// FPU operand pointer selector
    pub fos: u32,
    /// 8 x 10 bytes for each FP register = 80 bytes (stored as 20 x u32)
    pub st_space: [u32; 20],
}

/// Task State Segment (TSS) structure for i386.
///
/// The TSS is a hardware-defined structure used by the x86 processor
/// for hardware task switching. Each task has its own TSS.
#[repr(C)]
#[derive(Debug, Default)]
pub struct TaskStateSegment {
    /// Back link to previous task's TSS selector (16 high bits zero)
    pub back_link: u32,

    // Stack pointers and segments for privilege levels 0, 1, 2
    /// Stack pointer for ring 0 (kernel mode)
    pub esp0: u32,
    /// Stack segment for ring 0 (16 high bits zero)
    pub ss0: u32,
    /// Stack pointer for ring 1
    pub esp1: u32,
    /// Stack segment for ring 1 (16 high bits zero)
    pub ss1: u32,
    /// Stack pointer for ring 2
    pub esp2: u32,
    /// Stack segment for ring 2 (16 high bits zero)
    pub ss2: u32,

    /// Page directory base register (CR3)
    pub cr3: u32,

    // Saved execution state
    /// Instruction pointer
    pub eip: u32,
    /// CPU flags register
    pub eflags: u32,

    // General purpose registers
    pub eax: u32,
    pub ecx: u32,
    pub edx: u32,
    pub ebx: u32,
    pub esp: u32,
    pub ebp: u32,
    pub esi: u32,
    pub edi: u32,

    // Segment registers (16 high bits zero for each)
    /// Extra segment
    pub es: u32,
    /// Code segment
    pub cs: u32,
    /// Stack segment
    pub ss: u32,
    /// Data segment
    pub ds: u32,
    /// Additional segment F
    pub fs: u32,
    /// Additional segment G
    pub gs: u32,

    /// LDT segment selector (16 high bits zero)
    pub ldt: u32,

    /// Bits: trace flag (bit 0), I/O map base address (bits 16-31)
    pub trace_bitmap: u32,

    /// x87 FPU state (for hardware layout alignment)
    pub i387: I387Struct,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    Running = 0,
    Interruptible = 1,
    Uninterruptible = 2,
    Zombie = 3,
    Stopped = 4,
}

pub struct TaskSchedInfo {
    pub state: TaskState,
    pub counter: u32,
    pub priority: u32,
}
