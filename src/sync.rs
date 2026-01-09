#![allow(dead_code)]
use core::{
    arch::asm,
    cell::{RefCell, RefMut},
    ops::{Deref, DerefMut},
};

use crate::println;

#[inline]
pub fn sti() {
    unsafe {
        println!("sti");
        asm!("sti");
    }
}

#[inline]
pub fn cli() {
    unsafe {
        asm!("cli");
    }
}

/// Returns true if interrupts are currently enabled (IF flag is set).
#[inline]
fn interrupts_enabled() -> bool {
    let eflags: u32;
    unsafe {
        asm!("pushfd; pop {}", out(reg) eflags, options(nomem, preserves_flags));
    }
    // IF (Interrupt Flag) is bit 9
    (eflags & (1 << 9)) != 0
}

pub struct IrqSafeCell<T> {
    /// inner data
    inner: RefCell<T>,
}

unsafe impl<T> Sync for IrqSafeCell<T> {}

pub struct IrqSafeRefMut<'a, T> {
    inner: Option<RefMut<'a, T>>,
    /// Whether interrupts were enabled before we disabled them.
    /// Only restore interrupts on drop if this is true.
    irq_was_enabled: bool,
}

impl<T> IrqSafeCell<T> {
    pub const unsafe fn new(value: T) -> Self {
        Self {
            inner: RefCell::new(value),
        }
    }

    pub fn exclusive_access(&self) -> IrqSafeRefMut<'_, T> {
        let irq_was_enabled = interrupts_enabled();
        irq_was_enabled.then(cli);
        IrqSafeRefMut {
            inner: Some(self.inner.borrow_mut()),
            irq_was_enabled,
        }
    }

    pub fn exclusive_session<F, V>(&self, f: F) -> V
    where
        F: FnOnce(&mut T) -> V,
    {
        let mut inner = self.exclusive_access();
        f(inner.deref_mut())
    }
}

impl<T> Drop for IrqSafeRefMut<'_, T> {
    fn drop(&mut self) {
        self.inner = None;
        self.irq_was_enabled.then(sti);
    }
}

impl<T> Deref for IrqSafeRefMut<'_, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.inner.as_ref().unwrap().deref()
    }
}

impl<T> DerefMut for IrqSafeRefMut<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.inner.as_mut().unwrap().deref_mut()
    }
}
