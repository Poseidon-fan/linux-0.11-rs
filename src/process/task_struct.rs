#![allow(dead_code)]

struct ProcessControlBlock {
    state: TaskState,
    counter: u32,
    priority: u32,
    exit_code: i32,
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
