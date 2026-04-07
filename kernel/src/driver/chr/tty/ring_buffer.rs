//! Fixed-capacity ring buffer for TTY byte queues.
//!
//! Each TTY device uses three ring buffers:
//!
//! - `raw_rx`   — raw bytes received from hardware (keyboard, serial)
//! - `cooked_rx` — bytes processed by the line discipline, ready for user reads
//! - `tx`        — bytes awaiting transmission to the output device
//!
//! All index arithmetic uses bitwise masking, so [`CAPACITY`] must be a power
//! of two (enforced at compile time).

/// Number of bytes each ring buffer can store (must be a power of two).
pub const CAPACITY: usize = 1024;

const _: () = assert!(
    CAPACITY.is_power_of_two(),
    "RingBuffer CAPACITY must be a power of two"
);

const MASK: usize = CAPACITY - 1;

/// A fixed-size, single-producer / single-consumer byte ring buffer.
///
/// `head` is the next position to write (push) into.
/// `tail` is the next position to read (pop) from.
/// The buffer is empty when `head == tail` and can hold at most
/// `CAPACITY - 1` bytes (one slot is always unused to distinguish
/// full from empty).
pub struct RingBuffer {
    buf: [u8; CAPACITY],
    head: usize,
    tail: usize,
}

impl RingBuffer {
    /// Create an empty ring buffer. Usable in `const` / `static` context.
    pub const fn new() -> Self {
        Self {
            buf: [0; CAPACITY],
            head: 0,
            tail: 0,
        }
    }

    /// Number of bytes currently stored.
    #[inline]
    pub fn len(&self) -> usize {
        self.head.wrapping_sub(self.tail) & MASK
    }

    /// Free space available for writing.
    #[inline]
    pub fn remaining(&self) -> usize {
        (self.tail.wrapping_sub(self.head).wrapping_sub(1)) & MASK
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.head == self.tail
    }

    #[inline]
    pub fn is_full(&self) -> bool {
        self.remaining() == 0
    }

    /// Append one byte. Returns `true` on success, `false` if full.
    #[inline]
    pub fn push(&mut self, byte: u8) -> bool {
        if self.is_full() {
            return false;
        }
        self.buf[self.head] = byte;
        self.head = (self.head + 1) & MASK;
        true
    }

    /// Remove and return the oldest byte. Returns `None` if empty.
    #[inline]
    pub fn pop(&mut self) -> Option<u8> {
        if self.is_empty() {
            return None;
        }
        let byte = self.buf[self.tail];
        self.tail = (self.tail + 1) & MASK;
        Some(byte)
    }

    /// Peek at the most recently pushed byte without removing it.
    /// Used by line editing to inspect the last character before a backspace.
    #[inline]
    pub fn peek_last(&self) -> Option<u8> {
        if self.is_empty() {
            return None;
        }
        Some(self.buf[(self.head.wrapping_sub(1)) & MASK])
    }

    /// Remove and return the most recently pushed byte (undo a `push`).
    /// Used by canonical-mode ERASE/KILL to retract characters from the
    /// cooked input queue.
    #[inline]
    pub fn unpush(&mut self) -> Option<u8> {
        if self.is_empty() {
            return None;
        }
        self.head = (self.head.wrapping_sub(1)) & MASK;
        Some(self.buf[self.head])
    }

    /// Discard all buffered data.
    #[inline]
    pub fn flush(&mut self) {
        self.tail = self.head;
    }
}
