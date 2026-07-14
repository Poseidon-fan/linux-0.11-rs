//! Fixed-capacity byte ring buffer backing the TTY input and output queues.
//!
//! Every TTY device keeps three of these: one for raw bytes arriving from
//! hardware, one for line-discipline output ready to be read, and one for
//! bytes awaiting transmission. All index arithmetic is bitwise masking, so
//! [`CAPACITY`] must be a power of two (checked at compile time).

/// A fixed-size, single-producer / single-consumer byte ring buffer.
///
/// `head` is the next slot to write; `tail` the next slot to read. The buffer
/// is empty when `head == tail`, and one slot is always left unused so that the
/// full and empty states stay distinguishable — usable capacity is therefore
/// `CAPACITY - 1`.
pub struct RingBuffer {
    /// Backing byte storage.
    buf: [u8; CAPACITY],
    /// Index of the next slot to write.
    head: usize,
    /// Index of the next slot to read.
    tail: usize,
}

const _: () = assert!(
    CAPACITY.is_power_of_two(),
    "RingBuffer CAPACITY must be a power of two"
);

/// Number of bytes each ring buffer can store (must be a power of two).
const CAPACITY: usize = 1024;

/// Bit mask applied to every index to wrap it within the backing storage.
const MASK: usize = CAPACITY - 1;

impl RingBuffer {
    /// Create an empty ring buffer, usable in `const` / `static` context.
    pub const fn new() -> Self {
        Self {
            buf: [0; CAPACITY],
            head: 0,
            tail: 0,
        }
    }

    /// Number of bytes currently stored.
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.head.wrapping_sub(self.tail) & MASK
    }

    /// Free space available for writing.
    #[inline]
    #[must_use]
    pub fn remaining(&self) -> usize {
        MASK - self.len()
    }

    /// Whether the buffer holds no bytes.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.head == self.tail
    }

    /// Whether the buffer has no free space.
    #[inline]
    #[must_use]
    pub fn is_full(&self) -> bool {
        self.remaining() == 0
    }

    /// Append one byte, returning `false` if the buffer was full.
    #[inline]
    pub fn push(&mut self, byte: u8) -> bool {
        if self.is_full() {
            return false;
        }
        self.buf[self.head] = byte;
        self.head = (self.head + 1) & MASK;
        true
    }

    /// Remove and return the oldest byte, or `None` if empty.
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
    ///
    /// Canonical-mode editing uses this to inspect the last character before
    /// deciding how a backspace should rub it out.
    #[inline]
    #[must_use]
    pub fn last(&self) -> Option<u8> {
        if self.is_empty() {
            return None;
        }
        Some(self.buf[self.head.wrapping_sub(1) & MASK])
    }

    /// Remove and return the most recently pushed byte, undoing a [`push`].
    ///
    /// Canonical-mode ERASE/KILL uses this to retract characters that have not
    /// yet been handed to a reader.
    ///
    /// [`push`]: Self::push
    #[inline]
    pub fn pop_last(&mut self) -> Option<u8> {
        if self.is_empty() {
            return None;
        }
        self.head = self.head.wrapping_sub(1) & MASK;
        Some(self.buf[self.head])
    }

    /// Discard all buffered bytes.
    #[inline]
    pub fn clear(&mut self) {
        self.tail = self.head;
    }
}
