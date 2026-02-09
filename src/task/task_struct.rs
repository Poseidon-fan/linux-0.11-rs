use core::ops::{Deref, DerefMut};

use crate::{
    mm::{
        frame::{self, PAGE_SIZE, PhysFrame},
        space::MemorySpace,
    },
    segment::Descriptor,
    sync::KernelCell,
};

/// Process Control Block (PCB) for a task.
///
/// Contains the process identifier and interior-mutable fields wrapped in
/// [`KernelCell`] to allow mutation through shared references in a
/// single-threaded kernel context.
pub struct TaskControlBlock {
    pub pid: u32,
    pub inner: KernelCell<TaskControlBlockInner>,
}

/// Mutable fields of the process control block.
///
/// This struct holds all the mutable state of a task, separated from the
/// immutable `pid` to enable interior mutability via [`KernelCell`].
pub struct TaskControlBlockInner {
    pub sched: TaskSchedInfo,
    pub memory_space: Option<MemorySpace>,
    pub exit_code: i32,
    pub ldt: LocalDescriptorTable,
    pub tss: TaskStateSegment,
}

/// Memory layout of a task page (4KB aligned).
///
/// Each task struct occupies exactly one physical page (4096 bytes). The page is organized
/// with the Process Control Block ([`TaskControlBlock`]) at the low address,
/// and the remaining space used as the kernel stack.
///
/// # Memory Layout
///
/// ```text
///  base + 4096 ──►┌──────────────────┐ ◄─ ESP0 (stack top)
///                 │   Kernel Stack   │ ▲
///                 │       ↓          │ │ grows downward
///                 ├──────────────────┤
///                 │ TaskControlBlock │
///  base + 0    ──►└──────────────────┘
/// ```
///
/// The kernel stack pointer (ESP0 in TSS) should point to `base + 4096`.
#[repr(C, align(4096))]
pub struct TaskPage {
    pub pcb: TaskControlBlock,

    stack: [u8; PAGE_SIZE as usize - size_of::<TaskControlBlock>()],
}

/// An owned task that holds ownership of its underlying physical frame.
///
/// This struct wraps a [`PhysFrame`] and implements [`Deref`] and [`DerefMut`]
/// to provide transparent access to the [`TaskPage`] stored within the frame.
/// When a `Task` is dropped, the physical frame is automatically deallocated.
///
/// For task 0 (idle process), the frame is statically allocated in kernel
/// memory (below 1MB), so drop is a no-op (FrameAllocator ignores it).
pub struct Task(PhysFrame);

/// Local Descriptor Table (LDT) for a task.
///
/// Each task has its own LDT containing:
/// - Entry 0: Null descriptor
/// - Entry 1: User code segment
/// - Entry 2: User data segment
#[repr(C)]
#[derive(Clone)]
pub struct LocalDescriptorTable {
    pub entries: [Descriptor; 3],
}

/// x87 FPU (Math Coprocessor) state structure.
#[repr(C)]
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
#[derive(Clone, Copy, PartialEq, Eq)]
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

impl Deref for Task {
    type Target = TaskPage;

    fn deref(&self) -> &Self::Target {
        let addr = self.0.ppn.0 << 12;
        unsafe { &*(addr as *const TaskPage) }
    }
}

impl DerefMut for Task {
    fn deref_mut(&mut self) -> &mut Self::Target {
        let addr = self.0.ppn.0 << 12;
        unsafe { &mut *(addr as *mut TaskPage) }
    }
}

impl TaskControlBlock {
    pub fn new(pid: u32, inner: TaskControlBlockInner) -> Self {
        Self {
            pid,
            inner: KernelCell::new(inner),
        }
    }
}

impl TaskPage {
    pub fn new(pcb: TaskControlBlock) -> Self {
        Self {
            pcb,
            stack: [0; PAGE_SIZE as usize - size_of::<TaskControlBlock>()],
        }
    }

    pub fn stack_top(&self) -> u32 {
        self as *const TaskPage as u32 + PAGE_SIZE
    }
}

impl Task {
    /// Create a Task from a statically allocated TaskPage address.
    ///
    /// # Safety
    ///
    /// The address must point to a valid, page-aligned TaskPage that:
    /// - Lives for the entire kernel lifetime (static allocation)
    /// - Is located below 1MB (so frame allocator won't try to free it)
    pub unsafe fn from_static_addr(addr: u32) -> Self {
        use crate::mm::address::PhysPageNum;
        Self(PhysFrame {
            ppn: PhysPageNum(addr >> 12),
        })
    }

    /// Allocate a new task backed by a fresh physical page.
    ///
    /// The page is zeroed by the frame allocator. The caller is responsible
    /// for initializing the [`TaskPage`] contents (PCB fields, kernel stack)
    /// before the task is scheduled.
    ///
    /// Returns `None` if no free physical frame is available.
    pub fn new() -> Option<Self> {
        let frame = frame::alloc()?;
        Some(Self(frame))
    }
}

impl I387Struct {
    pub const fn empty() -> Self {
        Self {
            cwd: 0,
            swd: 0,
            twd: 0,
            fip: 0,
            fcs: 0,
            foo: 0,
            fos: 0,
            st_space: [0; 20],
        }
    }
}

impl LocalDescriptorTable {
    /// Create a new LDT with user code and data segments.
    ///
    /// # Arguments
    /// - `base`: Base address of the segment (typically 0 for flat model)
    /// - `limit`: Segment limit in 4KB units (e.g., 0x9f = 640KB / 4KB - 1)
    pub const fn new(base: u32, limit: u32) -> Self {
        Self {
            entries: [
                Descriptor::null(),
                Descriptor::user_code(base, limit),
                Descriptor::user_data(base, limit),
            ],
        }
    }

    /// Get the user code segment descriptor (LDT[1]).
    pub const fn code_segment(&self) -> Descriptor {
        self.entries[1]
    }

    /// Get the user data segment descriptor (LDT[2]).
    pub const fn data_segment(&self) -> Descriptor {
        self.entries[2]
    }

    /// Set the base address of both user code and data segments.
    pub fn set_base(&mut self, base: u32) {
        self.entries[1] = self.entries[1].with_base(base);
        self.entries[2] = self.entries[2].with_base(base);
    }

    /// Get the address of the LDT for use in GDT LDT descriptor.
    pub fn as_ptr(&self) -> u32 {
        self as *const _ as u32
    }
}
