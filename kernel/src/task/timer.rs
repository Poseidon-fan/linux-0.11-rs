//! PIT timer interrupt entry and tick handling.

use core::{
    arch::naked_asm,
    ptr,
    sync::atomic::{AtomicU32, Ordering},
};

use crate::{
    pmio::outb,
    segment::{USER_CS, USER_DS},
    signal::{self, DeliverAction, SignalDeliveryFrame, SignalSavedRegisters},
    task,
};

/// Timer ticks per second.
pub const HZ: u32 = 100;

/// PIT oscillator frequency (Hz).
const PIT_FREQUENCY: u32 = 1_193_180;

/// PIT reload value for the configured HZ.
pub const LATCH: u16 = (PIT_FREQUENCY / HZ) as u16;

/// Number of timer ticks since boot.
static JIFFIES: AtomicU32 = AtomicU32::new(0);

/// Returns current jiffies value.
#[inline]
pub fn jiffies() -> u32 {
    JIFFIES.load(Ordering::Relaxed)
}

/// Return frame layout at IRQ0 return site before `iret`.
///
/// This mirrors the push order in [`timer_interrupt`]:
/// `eax, ebx, ecx, edx, fs, es, ds` and then CPU-pushed return frame.
#[repr(C)]
pub struct TimerInterruptFrame {
    pub eax: u32,
    pub ebx: u32,
    pub ecx: u32,
    pub edx: u32,
    pub fs: u32,
    pub es: u32,
    pub ds: u32,
    pub eip: u32,
    pub cs: u32,
    pub eflags: u32,
}

impl TimerInterruptFrame {
    #[inline]
    fn user_esp_ptr(&self) -> *mut u32 {
        let base = self as *const Self as *mut u32;
        unsafe { base.add(core::mem::size_of::<Self>() / core::mem::size_of::<u32>()) }
    }

    #[inline]
    fn user_ss_ptr(&self) -> *const u32 {
        unsafe { self.user_esp_ptr().add(1).cast_const() }
    }
}

impl SignalDeliveryFrame for TimerInterruptFrame {
    #[inline]
    fn is_returning_to_user(&self) -> bool {
        if (self.cs & 0xffff) != USER_CS.as_u32() {
            return false;
        }
        let user_ss = unsafe { ptr::read(self.user_ss_ptr()) };
        (user_ss & 0xffff) == USER_DS.as_u32()
    }

    fn deliver_signal(&mut self, action: DeliverAction) -> bool {
        if !self.is_returning_to_user() {
            return false;
        }

        let user_esp_ptr = self.user_esp_ptr();
        let user_esp = unsafe { ptr::read(user_esp_ptr) };
        let regs = SignalSavedRegisters {
            eax: self.eax,
            ecx: self.ecx,
            edx: self.edx,
            eflags: self.eflags,
            old_eip: self.eip,
        };
        let new_esp = signal::push_user_signal_frame(
            user_esp,
            action.restorer,
            action.signr,
            action.blocked,
            action.sa_flags,
            regs,
        );

        unsafe {
            ptr::write(user_esp_ptr, new_esp);
        }
        self.eip = action.handler;
        true
    }
}

/// IRQ0 entry stub.
#[naked]
pub extern "C" fn timer_interrupt() {
    unsafe {
        naked_asm!(
            "push %ds",
            "push %es",
            "push %fs",
            "pushl %edx",
            "pushl %ecx",
            "pushl %ebx",
            "pushl %eax",
            "movl $0x10, %eax",
            "movw %ax, %ds",
            "movw %ax, %es",
            "movl $0x17, %eax",
            "movw %ax, %fs",
            "movl {saved_cs_off}(%esp), %eax",
            "andl $3, %eax",
            "movl %esp, %edx",
            "pushl %eax",
            "pushl %edx",
            "call {entry}",
            "addl $8, %esp",
            "popl %eax",
            "popl %ebx",
            "popl %ecx",
            "popl %edx",
            "pop %fs",
            "pop %es",
            "pop %ds",
            "iret",
            saved_cs_off = const 32,
            entry = sym handle_timer_tick,
            options(att_syntax),
        );
    }
}

/// Rust-side timer tick logic for IRQ0.
extern "C" fn handle_timer_tick(frame: *mut TimerInterruptFrame, cpl: u32) {
    JIFFIES.fetch_add(1, Ordering::Relaxed);

    // Send End-Of-Interrupt to master 8259A PIC.
    outb(0x20, 0x20);

    // Safety: IRQ0 runs through an interrupt gate, so hardware already
    // masked interrupts on entry. This satisfies `exclusive_unchecked`.
    let should_schedule = unsafe {
        task::current_task()
            .pcb
            .inner
            .exclusive_unchecked(|current| {
                if cpl != 0 {
                    current.acct.utime = current.acct.utime.wrapping_add(1);
                } else {
                    current.acct.stime = current.acct.stime.wrapping_add(1);
                }

                if current.sched.counter > 0 {
                    current.sched.counter -= 1;
                }

                current.sched.counter == 0 && cpl != 0
            })
    };

    if should_schedule {
        task::schedule();
    }

    // SAFETY: points to the active IRQ return frame created by this entry stub.
    signal::handle_pending_signal(unsafe { &mut *frame });
}
