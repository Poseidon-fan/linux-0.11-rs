use core::mem::MaybeUninit;
use core::ptr::{addr_of_mut, write_bytes};

use lazy_static::lazy_static;

use crate::{
    mm::{
        frame::PAGE_SIZE,
        space::{MemorySpace, TASK_LINEAR_SIZE},
    },
    segment::{
        self,
        selectors::{self, KERNEL_DS, USER_CS, USER_DS},
    },
    sync::KernelCell,
    syscall::{EAGAIN, SyscallContext},
    task::task_struct::*,
};

pub const TASK_NUM: usize = 64;

unsafe extern "C" {
    static pg_dir: u8;
}

/// Statically allocated memory for task 0 (idle process).
///
/// Located in kernel memory (below 1MB), so the frame allocator
/// won't try to free it when the Task is dropped.
static mut INIT_TASK_PAGE: MaybeUninit<TaskPage> = MaybeUninit::uninit();

pub struct TaskManager {
    pub tasks: [Option<Task>; TASK_NUM],
    pub current: usize,
    pub last_pid: u32,
}

impl TaskManager {
    pub fn fork(&mut self, ctx: &SyscallContext) -> Result<u32, u32> {
        // 1. Find a free slot in task array.
        let slot = self.find_empty_process().ok_or(EAGAIN)?;

        // 2. Allocate a new task page.
        let mut new_task = Task::new().ok_or(EAGAIN)?;

        // 3. Initialize PCB: copy parent task's PCB and reset some fields.
        let parent_inner = self.current().pcb.inner.borrow();
        new_task.pcb = TaskControlBlock {
            pid: self.last_pid,
            inner: KernelCell::new(TaskControlBlockInner {
                sched: TaskSchedInfo {
                    state: TaskState::Uninterruptible,
                    counter: parent_inner.sched.priority,
                    priority: parent_inner.sched.priority,
                },
                memory_space: None, // empty, will be replaced by cow_copy
                exit_code: 0,
                ldt: parent_inner.ldt.clone(),
                tss: TaskStateSegment {
                    back_link: 0,
                    esp0: new_task.stack_top(),
                    ss0: KERNEL_DS.as_u32(),
                    esp1: 0,
                    ss1: 0,
                    esp2: 0,
                    ss2: 0,
                    cr3: unsafe { &pg_dir as *const u8 as u32 },
                    eip: ctx.eip,
                    eflags: ctx.eflags,
                    eax: 0, // child returns 0 from fork
                    ecx: ctx.ecx,
                    edx: ctx.edx,
                    ebx: ctx.ebx,
                    esp: ctx.user_esp,
                    ebp: ctx.ebp,
                    esi: ctx.esi,
                    edi: ctx.edi,
                    es: ctx.es & 0xffff,
                    cs: ctx.cs & 0xffff,
                    ss: ctx.user_ss & 0xffff,
                    ds: ctx.ds & 0xffff,
                    fs: ctx.fs & 0xffff,
                    gs: ctx.gs & 0xffff,
                    ldt: selectors::ldt_selector(slot as u16).as_u32(),
                    trace_bitmap: 0x8000_0000,
                    i387: I387Struct::empty(),
                },
            }),
        };

        // 4. Copy memory space from parent to child (COW).
        let old_base = parent_inner.ldt.data_segment().base();
        let code_base = parent_inner.ldt.code_segment().base();
        assert_eq!(old_base, code_base, "separate I&D not supported");

        let code_limit = parent_inner.ldt.code_segment().byte_limit();
        let data_limit = parent_inner.ldt.data_segment().byte_limit();
        assert!(
            data_limit >= code_limit,
            "bad data_limit: data < code (0x{:x} < 0x{:x})",
            data_limit,
            code_limit
        );

        // Set the child's LDT segment base to its linear address slot.
        let new_base = slot as u32 * TASK_LINEAR_SIZE;
        let mut child_inner = new_task.pcb.inner.borrow_mut();
        child_inner.ldt.set_base(new_base);

        // Perform the COW page table copy.
        let child_space = parent_inner
            .memory_space
            .as_ref()
            .expect("parent memory space is none, unexpected error")
            .cow_copy(slot, data_limit)
            .map_err(|_| EAGAIN)?;
        child_inner.memory_space = Some(child_space);

        // 5. Install TSS and LDT descriptors in GDT for the new task.
        let tss_addr = &child_inner.tss as *const TaskStateSegment as u32;
        let ldt_addr = child_inner.ldt.as_ptr();
        segment::set_tss_desc(slot as u16, tss_addr);
        segment::set_ldt_desc(slot as u16, ldt_addr);

        // 6. Mark the child as runnable and insert into the task table.
        child_inner.sched.state = TaskState::Running;
        drop(parent_inner);
        drop(child_inner);
        self.tasks[slot] = Some(new_task);

        Ok(self.last_pid)
    }

    pub fn current(&self) -> &Task {
        self.tasks[self.current]
            .as_ref()
            .expect("get current task failed")
    }

    /// Find an unused PID and an empty slot in the task table.
    ///
    /// Increments `last_pid` (wrapping to 1 on overflow) until a PID is
    /// found that no existing task uses, then scans `tasks[1..]` for the
    /// first empty slot.
    ///
    /// Returns the slot index on success (`self.last_pid` holds the new PID).
    /// Returns `None` if no empty slot is available.
    fn find_empty_process(&mut self) -> Option<usize> {
        // Step 1: find a unique PID not used by any existing task.
        'retry: loop {
            self.last_pid = self.last_pid.wrapping_add(1);
            if self.last_pid == 0 {
                self.last_pid = 1;
            }
            for task in self.tasks.iter().flatten() {
                if task.pcb.pid == self.last_pid {
                    continue 'retry;
                }
            }
            break;
        }

        // Step 2: find an empty slot in tasks[1..].
        (1..TASK_NUM).find(|&i| self.tasks[i].is_none())
    }
}

lazy_static! {
    pub static ref TASK_MANAGER: KernelCell<TaskManager> = unsafe {
        // Initialize the static memory for task 0.
        let init_task_ptr = addr_of_mut!(INIT_TASK_PAGE).cast::<TaskPage>();
        let init_task_addr = init_task_ptr as u32;

        // Zero the whole task page.
        write_bytes(init_task_ptr.cast::<u8>(), 0, PAGE_SIZE as usize);

        // Then initialize only the PCB.
        addr_of_mut!((*init_task_ptr).pcb).write(TaskControlBlock::new(
            0, // pid = 0
            TaskControlBlockInner {
                sched: TaskSchedInfo {
                    state: TaskState::Running,
                    counter: 15,
                    priority: 15,
                },
                memory_space: Some(MemorySpace::new(0)), // task 0
                exit_code: 0,
                ldt: LocalDescriptorTable::new(0, 0x9f),
                tss: TaskStateSegment {
                    back_link: 0,
                    esp0: init_task_addr + PAGE_SIZE,
                    ss0: KERNEL_DS.as_u32(),
                    esp1: 0,
                    ss1: 0,
                    esp2: 0,
                    ss2: 0,
                    cr3: &pg_dir as *const u8 as u32,
                    eip: 0,
                    eflags: 0,
                    eax: 0,
                    ecx: 0,
                    edx: 0,
                    ebx: 0,
                    esp: 0,
                    ebp: 0,
                    esi: 0,
                    edi: 0,
                    es: USER_DS.as_u32(),
                    cs: USER_CS.as_u32(),
                    ss: USER_DS.as_u32(),
                    ds: USER_DS.as_u32(),
                    fs: USER_DS.as_u32(),
                    gs: USER_DS.as_u32(),
                    ldt: selectors::ldt_selector(0).as_u32(),
                    trace_bitmap: 0x8000_0000,
                    i387: I387Struct::empty(),
                },
            },
        ));

        // Create Task from the static address.
        let task0 = Task::from_static_addr(init_task_addr);

        // Initialize task array with task 0.
        let mut tasks: [Option<Task>; TASK_NUM] = [const { None }; TASK_NUM];
        tasks[0] = Some(task0);

        KernelCell::new(TaskManager { tasks, current: 0, last_pid: 0 })
    };
}
