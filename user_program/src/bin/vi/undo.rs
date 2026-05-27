//! Undo stack — coarse-grained snapshots.
//!
//! A real vi tracks insert deltas and merges contiguous typing into a
//! single undo step. We do the simple thing: every command that mutates
//! the buffer pushes a full snapshot of the line table + cursor before
//! running. `u` pops the most recent snapshot. The trade-off is memory
//! — a 100 KB buffer with 50 snapshots eats ~5 MB — but it's the right
//! call for a teaching editor and matches what `busybox vi` does too.

use alloc::{string::String, vec::Vec};

use crate::buffer::Buffer;

/// One pre-mutation snapshot.
pub struct Snapshot {
    lines: Vec<String>,
    row: usize,
    col: usize,
}

/// Bounded LIFO stack.
pub struct Undo {
    stack: Vec<Snapshot>,
    cap: usize,
}

impl Undo {
    pub fn new() -> Self {
        Self {
            stack: Vec::new(),
            cap: 200,
        }
    }

    /// Captures `buf`'s state. Call this **before** mutating.
    pub fn snapshot(&mut self, buf: &Buffer) {
        if self.stack.len() == self.cap {
            self.stack.remove(0);
        }
        self.stack.push(Snapshot {
            lines: buf.lines.clone(),
            row: buf.row,
            col: buf.col,
        });
    }

    /// Reverts `buf` to the most recent snapshot. Returns `true` if
    /// something was actually restored.
    pub fn pop_into(&mut self, buf: &mut Buffer) -> bool {
        let Some(s) = self.stack.pop() else {
            return false;
        };
        buf.lines = s.lines;
        buf.row = s.row;
        buf.col = s.col;
        buf.dirty = true;
        true
    }
}
