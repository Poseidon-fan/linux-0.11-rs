use core::mem::MaybeUninit;
use core::ptr::addr_of_mut;

use lazy_static::lazy_static;

use crate::{
    mm::{frame::PAGE_SIZE, space::MemorySpace},
    segment::selectors::{self, KERNEL_DS, USER_CS, USER_DS},
    sync::KernelCell,
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
    /// Find an unused PID and an empty slot in the task table.
    ///
    /// Increments `last_pid` (wrapping to 1 on overflow) until a PID is
    /// found that no existing task uses, then scans `tasks[1..]` for the
    /// first empty slot.
    ///
    /// Returns the slot index on success (`self.last_pid` holds the new PID).
    /// Returns `None` if no empty slot is available.
    pub fn find_empty_process(&mut self) -> Option<usize> {
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

        // Write the initial task 0 data.
        init_task_ptr.write(TaskPage::new(
            TaskControlBlock::new(
                0, // pid = 0
                TaskControlBlockInner {
                    sched: TaskSchedInfo {
                        state: TaskState::Running,
                        counter: 15,
                        priority: 15,
                    },
                    memory_space: MemorySpace::new(),
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
            ),
        ));

        // Create Task from the static address.
        let task0 = Task::from_static_addr(init_task_addr);

        // Initialize task array with task 0.
        let mut tasks: [Option<Task>; TASK_NUM] = [const { None }; TASK_NUM];
        tasks[0] = Some(task0);

        KernelCell::new(TaskManager { tasks, current: 0, last_pid: 0 })
    };
}
