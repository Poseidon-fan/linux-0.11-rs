#![allow(dead_code)]
use core::{
    arch::asm,
    cell::{RefCell, RefMut},
    ops::{Deref, DerefMut},
};

#[inline]
pub fn sti() {
    unsafe {
        asm!("sti");
    }
}

#[inline]
pub fn cli() {
    unsafe {
        asm!("cli");
    }
}

pub struct IrqSafeCell<T> {
    /// inner data
    inner: RefCell<T>,
}

unsafe impl<T> Sync for IrqSafeCell<T> {}

pub struct IrqSafeRefMut<'a, T>(Option<RefMut<'a, T>>);

impl<T> IrqSafeCell<T> {
    pub const unsafe fn new(value: T) -> Self {
        Self {
            inner: RefCell::new(value),
        }
    }

    pub fn exclusive_access(&self) -> IrqSafeRefMut<'_, T> {
        // cli();
        IrqSafeRefMut(Some(self.inner.borrow_mut()))
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
        self.0 = None;
        // sti();
    }
}

impl<T> Deref for IrqSafeRefMut<'_, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.0.as_ref().unwrap().deref()
    }
}
impl<T> DerefMut for IrqSafeRefMut<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0.as_mut().unwrap().deref_mut()
    }
}
