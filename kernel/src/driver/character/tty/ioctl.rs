//! Terminal `ioctl` request handling.
//!
//! Splits the `termios`/`termio` get-set matrix, queue queries, and process
//! group control out of the core so [`super::TtyDevice`] stays focused on the
//! data path. User-space pointers are validated before each access.

use user_lib::syscall::tty::{FlushSelector, Termio, Termios, TtyRequest};

use super::{TtyDevice, TtyState};
use crate::{
    error::{Errno, Result},
    mm,
    segment::uaccess,
};

/// Dispatch one terminal ioctl request for `device`.
pub fn dispatch(device: &'static TtyDevice, channel: usize, request: u32, arg: u32) -> Result<u32> {
    let request = TtyRequest::try_from(request).map_err(|()| Errno::INVAL)?;
    match request {
        TtyRequest::GetTermios => get_termios(device, arg),
        TtyRequest::SetTermios => set_termios(device, channel, arg),
        TtyRequest::SetTermiosWait => {
            device.backend.start_output(channel);
            set_termios(device, channel, arg)
        }
        TtyRequest::SetTermiosFlush => {
            device.state.exclusive(TtyState::flush_input);
            device.backend.start_output(channel);
            set_termios(device, channel, arg)
        }
        TtyRequest::GetTermio => get_termio(device, arg),
        TtyRequest::SetTermio => set_termio(device, channel, arg),
        TtyRequest::SetTermioWait => {
            device.backend.start_output(channel);
            set_termio(device, channel, arg)
        }
        TtyRequest::SetTermioFlush => {
            device.state.exclusive(TtyState::flush_input);
            device.backend.start_output(channel);
            set_termio(device, channel, arg)
        }
        TtyRequest::GetPgrp => {
            let pgrp = device.state.exclusive(|state| state.foreground_group);
            write_user_u32(arg, pgrp as u32)
        }
        TtyRequest::SetPgrp => {
            let pgrp = uaccess::read_u32(arg as *const u32) as i32;
            device
                .state
                .exclusive(|state| state.foreground_group = pgrp);
            Ok(0)
        }
        TtyRequest::Flush => flush(device, arg),
        TtyRequest::OutputQueueBytes => {
            let count = device.state.exclusive(|state| state.output.len());
            write_user_u32(arg, count as u32)
        }
        TtyRequest::InputQueueBytes => {
            let count = device.state.exclusive(|state| state.cooked_input.len());
            write_user_u32(arg, count as u32)
        }
    }
}

/// Copy the current `termios` out to a user pointer.
fn get_termios(device: &'static TtyDevice, user_ptr: u32) -> Result<u32> {
    let termios = device.state.exclusive(|state| state.termios);
    mm::ensure_user_area_writable(user_ptr, size_of::<Termios>());
    uaccess::write_struct(&termios, user_ptr as *mut Termios);
    Ok(0)
}

/// Load `termios` from a user pointer and reconfigure the backend.
fn set_termios(device: &'static TtyDevice, channel: usize, user_ptr: u32) -> Result<u32> {
    let termios = uaccess::read_struct(user_ptr as *const Termios);
    device.state.exclusive(|state| state.termios = termios);
    device.reconfigure(channel);
    Ok(0)
}

/// Copy the current `termios` out to a user pointer in legacy `termio` form.
fn get_termio(device: &'static TtyDevice, user_ptr: u32) -> Result<u32> {
    let termio = device.state.exclusive(|state| state.termios.to_termio());
    mm::ensure_user_area_writable(user_ptr, size_of::<Termio>());
    uaccess::write_struct(&termio, user_ptr as *mut Termio);
    Ok(0)
}

/// Apply a legacy `termio` from a user pointer and reconfigure the backend.
fn set_termio(device: &'static TtyDevice, channel: usize, user_ptr: u32) -> Result<u32> {
    let termio = uaccess::read_struct(user_ptr as *const Termio);
    device
        .state
        .exclusive(|state| state.termios.apply_termio(termio));
    device.reconfigure(channel);
    Ok(0)
}

/// Flush input and/or output queues for a [`TtyRequest::Flush`] request.
fn flush(device: &'static TtyDevice, arg: u32) -> Result<u32> {
    let selector = FlushSelector::try_from(arg).map_err(|()| Errno::INVAL)?;
    device.state.exclusive(|state| match selector {
        FlushSelector::Input => state.flush_input(),
        FlushSelector::Output => state.flush_output(),
        FlushSelector::Both => {
            state.flush_input();
            state.flush_output();
        }
    });
    Ok(0)
}

/// Validate and write a `u32` to a user-space pointer.
fn write_user_u32(user_ptr: u32, value: u32) -> Result<u32> {
    mm::ensure_user_area_writable(user_ptr, size_of::<u32>());
    uaccess::write_u32(value, user_ptr as *mut u32);
    Ok(0)
}
