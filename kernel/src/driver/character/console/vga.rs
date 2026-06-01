//! VGA text-mode driver with VT102/ANSI escape sequence parser.
//!
//! Drives the display through direct memory-mapped I/O at the VGA text
//! buffer address, using CRT controller registers for cursor positioning
//! and fast-scroll origin adjustments on EGA/VGA hardware.
//!
//! VGA text buffer layout (color mode at 0xB8000):
//!
//! ```text
//! ┌──────────┬──────────┬──────────┬──────────┬───
//! │ char0    │ attr0    │ char1    │ attr1    │ ...
//! │ (byte 0) │ (byte 1) │ (byte 2) │ (byte 3) │
//! └──────────┴──────────┴──────────┴──────────┴───
//! ```
//!
//! Each cell is 2 bytes: ASCII character + attribute byte.
//! Attribute format: `[blink:1][bg:3][fg:4]`.

use core::ptr;

use crate::{
    pmio::{inb_p, outb, outb_p},
    sync::KernelCell,
};

/// Global VGA console state guarded by a `KernelCell`.
pub static CONSOLE: KernelCell<VgaConsole> = KernelCell::new(VgaConsole::uninitialized());

/// BIOS data area address holding the early-boot cursor column.
pub const ORIG_X: *const u8 = 0x90000 as *const u8;
/// BIOS data area address holding the early-boot cursor row.
pub const ORIG_Y: *const u8 = 0x90001 as *const u8;

/// VGA text-mode console state.
///
/// Manages the entire screen geometry, cursor position, scroll region, attribute
/// state, and the ANSI escape parser. Protected by a global `KernelCell`.
pub struct VgaConsole {
    /// Detected display adapter type.
    display_type: DisplayType,
    /// Visible columns per row.
    columns: usize,
    /// Visible rows on screen.
    lines: usize,
    /// Bytes occupied by one screen row.
    row_bytes: usize,
    /// Start address of the text-mode video memory.
    mem_start: usize,
    /// End address of the text-mode video memory.
    mem_end: usize,
    /// CRT controller index register port.
    port_reg: u16,
    /// CRT controller data register port.
    port_val: u16,
    /// Cell value (char + attribute) written when erasing.
    erase_cell: u16,

    /// Current display origin address.
    origin: usize,
    /// Address one past the last visible cell.
    screen_end: usize,
    /// Memory address of the current cursor cell.
    cursor_pos: usize,
    /// Cursor column.
    cursor_x: usize,
    /// Cursor row.
    cursor_y: usize,
    /// Top row of the active scroll region.
    scroll_top: usize,
    /// Bottom row (exclusive) of the active scroll region.
    scroll_bottom: usize,

    /// Current character attribute byte.
    attribute: u8,
    /// Saved cursor column for save/restore sequences.
    saved_x: usize,
    /// Saved cursor row for save/restore sequences.
    saved_y: usize,

    /// ANSI escape sequence parser state.
    parser: AnsiParser,
}

/// BIOS data area address with the original video mode word.
const ORIG_VIDEO_MODE: *const u16 = 0x90006 as *const u16;
/// BIOS data area address with the EGA/VGA configuration word.
const ORIG_VIDEO_EGA_BX: *const u16 = 0x9000a as *const u16;

/// Maximum number of CSI parameters accumulated per escape sequence.
const MAX_ANSI_PARAMS: usize = 16;

/// CRT controller index of the display start-address high byte (low byte at +1).
const CRTC_START_HIGH: u8 = 12;
/// CRT controller index of the cursor-location high byte (low byte at +1).
const CRTC_CURSOR_HIGH: u8 = 14;

/// Normal foreground-on-background text attribute.
const ATTR_NORMAL: u8 = 0x07;
/// Bright (bold) foreground attribute.
const ATTR_BOLD: u8 = 0x0f;
/// Inverse-video attribute.
const ATTR_INVERSE: u8 = 0x70;

/// Blank cell written when erasing: a space in the normal attribute.
const ERASE_CELL: u16 = ((ATTR_NORMAL as u16) << 8) | b' ' as u16;

/// BIOS video mode value indicating a monochrome display.
const MONO_VIDEO_MODE: u16 = 7;
/// EGA configuration sentinel: a low byte of `0x10` means no EGA present.
const NO_EGA_SENTINEL: u16 = 0x10;
/// Visible rows on a standard text-mode screen.
const SCREEN_LINES: usize = 25;

/// System control port B, whose low bits gate the PC speaker.
const SPEAKER_PORT: u16 = 0x61;
/// Speaker-enable bits (gate + data) in [`SPEAKER_PORT`].
const SPEAKER_ENABLE: u8 = 0b11;
/// PIT mode/command register.
const PIT_COMMAND: u16 = 0x43;
/// PIT channel 2 data register.
const PIT_CH2_DATA: u16 = 0x42;
/// PIT command: channel 2, low+high byte, square-wave mode.
const PIT_CH2_SQUARE_WAVE: u8 = 0xb6;
/// PIT reload divisor for the bell tone (1193182 Hz / 1591 ≈ 750 Hz).
const BELL_DIVISOR: u16 = 1591;

/// C0 control bytes handled directly by the console.
mod control {
    pub const BELL: u8 = 0x07;
    pub const BACKSPACE: u8 = 0x08;
    pub const TAB: u8 = b'\t';
    pub const LINE_FEED: u8 = b'\n';
    pub const VERTICAL_TAB: u8 = 0x0b;
    pub const FORM_FEED: u8 = 0x0c;
    pub const CARRIAGE_RETURN: u8 = b'\r';
    pub const ESCAPE: u8 = 0x1b;
    pub const DELETE: u8 = 0x7f;
    /// First printable ASCII byte.
    pub const FIRST_PRINTABLE: u8 = 0x20;
    /// Last printable ASCII byte.
    pub const LAST_PRINTABLE: u8 = 0x7e;
}

/// Display adapter type detected during initialization.
#[derive(Clone, Copy, PartialEq, Eq)]
enum DisplayType {
    /// Monochrome Display Adapter.
    Mda,
    /// Color Graphics Adapter.
    Cga,
    /// EGA in monochrome mode.
    EgaMonochrome,
    /// EGA in color mode.
    EgaColor,
}

/// VT102/ANSI escape sequence parser state machine.
///
/// ```text
///  Normal ──ESC──► Escape ──'['──► CsiEntry/CsiParam ──final──► Normal
///                    │                                            ▲
///                    └──other──────────────────────────────────────┘
/// ```
#[derive(Clone, Copy, PartialEq, Eq)]
enum AnsiState {
    /// Passing bytes through to the screen.
    Normal,
    /// Saw ESC, awaiting the next byte.
    Escape,
    /// Saw `ESC [`, awaiting the first parameter byte.
    CsiEntry,
    /// Accumulating CSI parameters.
    CsiParam,
}

/// ANSI CSI parameter accumulator.
struct AnsiParser {
    /// Current parser state.
    state: AnsiState,
    /// Numeric parameters parsed from the current sequence.
    params: [u32; MAX_ANSI_PARAMS],
    /// Number of parameters accumulated so far.
    param_count: usize,
}

impl DisplayType {
    /// Whether this adapter supports the EGA/VGA fast-scroll origin register.
    fn is_ega(self) -> bool {
        matches!(self, Self::EgaMonochrome | Self::EgaColor)
    }
}

impl AnsiParser {
    /// Create a parser in the normal pass-through state.
    const fn new() -> Self {
        Self {
            state: AnsiState::Normal,
            params: [0; MAX_ANSI_PARAMS],
            param_count: 0,
        }
    }

    /// Reset accumulated parameters before a new escape sequence.
    fn reset_params(&mut self) {
        self.params = [0; MAX_ANSI_PARAMS];
        self.param_count = 0;
    }

    /// Parameter `index`, treating a zero or absent value as `default`.
    ///
    /// CSI cursor-movement parameters default to 1 (move one cell), so an
    /// omitted count behaves like an explicit `1`.
    fn param_or(&self, index: usize, default: u32) -> u32 {
        match self.params.get(index).copied().unwrap_or(0) {
            0 => default,
            value => value,
        }
    }
}

impl VgaConsole {
    const fn uninitialized() -> Self {
        Self {
            display_type: DisplayType::Cga,
            columns: 80,
            lines: SCREEN_LINES,
            row_bytes: 160,
            mem_start: 0xb8000,
            mem_end: 0xba000,
            port_reg: 0x3d4,
            port_val: 0x3d5,
            erase_cell: ERASE_CELL,
            origin: 0xb8000,
            screen_end: 0xb8000 + SCREEN_LINES * 160,
            cursor_pos: 0xb8000,
            cursor_x: 0,
            cursor_y: 0,
            scroll_top: 0,
            scroll_bottom: SCREEN_LINES,
            attribute: ATTR_NORMAL,
            saved_x: 0,
            saved_y: 0,
            parser: AnsiParser::new(),
        }
    }

    /// Probe the display hardware (using BIOS data written by setup.s) and
    /// initialize all VGA state fields.
    pub fn detect_and_init(&mut self) {
        let mode = unsafe { ptr::read_volatile(ORIG_VIDEO_MODE) } & 0xff;
        let cols = (unsafe { ptr::read_volatile(ORIG_VIDEO_MODE) } >> 8) as usize;
        let ega_bx = unsafe { ptr::read_volatile(ORIG_VIDEO_EGA_BX) };

        self.columns = cols;
        self.lines = SCREEN_LINES;
        self.row_bytes = cols * 2;
        self.erase_cell = ERASE_CELL;

        if mode == MONO_VIDEO_MODE {
            // Monochrome display.
            self.mem_start = 0xb0000;
            self.port_reg = 0x3b4;
            self.port_val = 0x3b5;
            if (ega_bx & 0xff) != NO_EGA_SENTINEL {
                self.display_type = DisplayType::EgaMonochrome;
                self.mem_end = 0xb8000;
            } else {
                self.display_type = DisplayType::Mda;
                self.mem_end = 0xb2000;
            }
        } else {
            // Color display.
            self.mem_start = 0xb8000;
            self.port_reg = 0x3d4;
            self.port_val = 0x3d5;
            if (ega_bx & 0xff) != NO_EGA_SENTINEL {
                self.display_type = DisplayType::EgaColor;
                self.mem_end = 0xbc000;
            } else {
                self.display_type = DisplayType::Cga;
                self.mem_end = 0xba000;
            }
        }

        self.origin = self.mem_start;
        self.screen_end = self.mem_start + self.lines * self.row_bytes;
        self.scroll_top = 0;
        self.scroll_bottom = self.lines;
        self.attribute = ATTR_NORMAL;
        self.parser = AnsiParser::new();

        // Continue from wherever early-boot VGA output left off, while clearing
        // untouched cells below it so later shorter lines do not reveal stale
        // firmware/setup text.
        let (x, y) = crate::logging::early_vga_cursor();
        self.move_cursor(x, y);
        self.erase_display(0);
        self.sync_hardware_cursor();
    }

    /// Write one byte through the VT102 parser state machine.
    pub fn write_byte(&mut self, byte: u8) {
        match self.parser.state {
            AnsiState::Normal => self.handle_normal(byte),
            AnsiState::Escape => self.handle_escape(byte),
            AnsiState::CsiEntry => self.handle_csi_entry(byte),
            AnsiState::CsiParam => self.handle_csi_param(byte),
        }
    }

    fn handle_normal(&mut self, byte: u8) {
        if (control::FIRST_PRINTABLE..=control::LAST_PRINTABLE).contains(&byte) {
            if self.cursor_x >= self.columns {
                self.cursor_x -= self.columns;
                self.cursor_pos -= self.row_bytes;
                self.line_feed();
            }
            self.put_char(byte);
            self.cursor_pos += 2;
            self.cursor_x += 1;
            return;
        }

        match byte {
            control::ESCAPE => self.parser.state = AnsiState::Escape,
            control::LINE_FEED | control::VERTICAL_TAB | control::FORM_FEED => self.line_feed(),
            control::CARRIAGE_RETURN => self.carriage_return(),
            control::DELETE => self.delete(),
            control::BACKSPACE => self.backspace(),
            control::TAB => self.tab(),
            control::BELL => self.bell(),
            _ => {}
        }
    }

    fn handle_escape(&mut self, byte: u8) {
        self.parser.state = AnsiState::Normal;
        match byte {
            b'[' => {
                self.parser.reset_params();
                self.parser.state = AnsiState::CsiEntry;
            }
            b'E' => self.move_cursor(0, self.cursor_y + 1),
            b'M' => self.reverse_index(),
            b'D' => self.line_feed(),
            b'7' => self.save_cursor(),
            b'8' => self.restore_cursor(),
            _ => {}
        }
    }

    fn handle_csi_entry(&mut self, byte: u8) {
        self.parser.state = AnsiState::CsiParam;
        // Swallow the DEC private-sequence marker (e.g. `ESC [ ? 25 h`); the
        // private modes it introduces are not implemented.
        if byte != b'?' {
            self.handle_csi_param(byte);
        }
    }

    fn handle_csi_param(&mut self, byte: u8) {
        if byte == b';' && self.parser.param_count < MAX_ANSI_PARAMS - 1 {
            self.parser.param_count += 1;
            return;
        }
        if byte.is_ascii_digit() {
            let param_index = self.parser.param_count;
            self.parser.params[param_index] =
                self.parser.params[param_index] * 10 + (byte - b'0') as u32;
            return;
        }
        // Final character — dispatch CSI command.
        self.parser.state = AnsiState::Normal;
        self.dispatch_csi(byte);
    }

    /// Dispatch a completed CSI sequence based on the final character.
    fn dispatch_csi(&mut self, final_ch: u8) {
        // Movement counts default to 1; absolute positions default to 1 then
        // become 0-based, so an omitted parameter means row/column 0.
        let count = self.parser.param_or(0, 1) as usize;
        let col = self.parser.param_or(0, 1) as usize - 1;
        let row = self.parser.param_or(0, 1) as usize - 1;

        match final_ch {
            b'G' | b'`' => self.move_cursor(col, self.cursor_y),
            b'A' => self.move_cursor(self.cursor_x, self.cursor_y.saturating_sub(count)),
            b'B' | b'e' => self.move_cursor(self.cursor_x, self.cursor_y + count),
            b'C' | b'a' => self.move_cursor(self.cursor_x + count, self.cursor_y),
            b'D' => self.move_cursor(self.cursor_x.saturating_sub(count), self.cursor_y),
            b'E' => self.move_cursor(0, self.cursor_y + count),
            b'F' => self.move_cursor(0, self.cursor_y.saturating_sub(count)),
            b'd' => self.move_cursor(self.cursor_x, row),
            b'H' | b'f' => {
                let target_col = self.parser.param_or(1, 1) as usize - 1;
                self.move_cursor(target_col, row);
            }
            b'J' => self.erase_display(self.parser.param_or(0, 0)),
            b'K' => self.erase_line(self.parser.param_or(0, 0)),
            b'L' => self.insert_lines(count),
            b'M' => self.delete_lines(count),
            b'P' => self.delete_chars(count),
            b'@' => self.insert_chars(count),
            b'm' => self.set_graphic_rendition(),
            b'r' => self.set_scroll_region(),
            b's' => self.save_cursor(),
            b'u' => self.restore_cursor(),
            _ => {}
        }
    }

    /// CSI r — set the top and bottom rows of the scroll region.
    fn set_scroll_region(&mut self) {
        let top = self.parser.param_or(0, 1) as usize - 1;
        let bottom = match self.parser.param_or(1, 0) {
            0 => self.lines,
            value => value as usize,
        };
        if top < bottom && bottom <= self.lines {
            self.scroll_top = top;
            self.scroll_bottom = bottom;
        }
    }

    // ---- Cursor ----

    fn move_cursor(&mut self, new_x: usize, new_y: usize) {
        if new_x > self.columns || new_y >= self.lines {
            return;
        }
        self.cursor_x = new_x;
        self.cursor_y = new_y;
        self.cursor_pos = self.origin + new_y * self.row_bytes + (new_x << 1);
    }

    /// Program the CRT controller hardware cursor to match `cursor_pos`.
    pub fn sync_hardware_cursor(&self) {
        let offset = (self.cursor_pos - self.mem_start) >> 1;
        self.write_crtc_word(CRTC_CURSOR_HIGH, offset);
    }

    fn save_cursor(&mut self) {
        self.saved_x = self.cursor_x;
        self.saved_y = self.cursor_y;
    }

    fn restore_cursor(&mut self) {
        self.move_cursor(self.saved_x, self.saved_y);
    }

    // ---- Character output ----

    /// Write a character + current attribute to VGA memory at the cursor position.
    fn put_char(&self, byte: u8) {
        let addr = self.cursor_pos as *mut u16;
        let cell = ((self.attribute as u16) << 8) | byte as u16;
        unsafe { ptr::write_volatile(addr, cell) };
    }

    // ---- Line operations ----

    fn line_feed(&mut self) {
        if self.cursor_y + 1 < self.scroll_bottom {
            self.cursor_y += 1;
            self.cursor_pos += self.row_bytes;
        } else {
            self.scroll_up();
        }
    }

    fn reverse_index(&mut self) {
        if self.cursor_y > self.scroll_top {
            self.cursor_y -= 1;
            self.cursor_pos -= self.row_bytes;
        } else {
            self.scroll_down();
        }
    }

    fn carriage_return(&mut self) {
        self.cursor_pos -= self.cursor_x << 1;
        self.cursor_x = 0;
    }

    fn delete(&mut self) {
        if self.cursor_x > 0 {
            self.cursor_pos -= 2;
            self.cursor_x -= 1;
            let addr = self.cursor_pos as *mut u16;
            unsafe { ptr::write_volatile(addr, self.erase_cell) };
        }
    }

    fn backspace(&mut self) {
        if self.cursor_x > 0 {
            self.cursor_x -= 1;
            self.cursor_pos -= 2;
        }
    }

    fn tab(&mut self) {
        let spaces = 8 - (self.cursor_x & 7);
        self.cursor_x += spaces;
        self.cursor_pos += spaces << 1;
        if self.cursor_x > self.columns {
            self.cursor_x -= self.columns;
            self.cursor_pos -= self.row_bytes;
            self.line_feed();
        }
    }

    fn bell(&mut self) {
        // Drive PIT channel 2 to beep the PC speaker at roughly 750 Hz.
        let speaker = inb_p(SPEAKER_PORT);
        outb_p(speaker | SPEAKER_ENABLE, SPEAKER_PORT);
        outb_p(PIT_CH2_SQUARE_WAVE, PIT_COMMAND);
        outb_p(BELL_DIVISOR as u8, PIT_CH2_DATA);
        outb((BELL_DIVISOR >> 8) as u8, PIT_CH2_DATA);
    }

    // ---- Scrolling ----

    /// Scroll the display up one line within the scroll region.
    fn scroll_up(&mut self) {
        let is_ega = self.display_type.is_ega();

        if is_ega && self.scroll_top == 0 && self.scroll_bottom == self.lines {
            // EGA/VGA fast scroll: adjust the display origin register.
            self.origin += self.row_bytes;
            self.cursor_pos += self.row_bytes;
            self.screen_end += self.row_bytes;

            if self.screen_end > self.mem_end {
                // Wrap: copy visible lines back to start of video memory.
                let line_count = self.lines - 1;
                let copy_words = line_count * self.columns;
                unsafe {
                    ptr::copy(
                        self.origin as *const u16,
                        self.mem_start as *mut u16,
                        copy_words,
                    );
                }
                // Fill the new last line with erase cells.
                let last_line = self.mem_start + line_count * self.row_bytes;
                self.fill_erase(last_line, self.columns);

                self.screen_end -= self.origin - self.mem_start;
                self.cursor_pos -= self.origin - self.mem_start;
                self.origin = self.mem_start;
            } else {
                // Just fill the new last line.
                let last_line = self.screen_end - self.row_bytes;
                self.fill_erase(last_line, self.columns);
            }
            self.set_origin();
        } else {
            // Non-EGA or partial scroll region: copy lines up.
            let dst = self.origin + self.row_bytes * self.scroll_top;
            let src = dst + self.row_bytes;
            let line_count = self.scroll_bottom - self.scroll_top - 1;
            let copy_words = line_count * self.columns;
            unsafe {
                ptr::copy(src as *const u16, dst as *mut u16, copy_words);
            }
            let last = self.origin + self.row_bytes * (self.scroll_bottom - 1);
            self.fill_erase(last, self.columns);
        }
    }

    /// Scroll the display down one line within the scroll region.
    fn scroll_down(&mut self) {
        let line_count = self.scroll_bottom - self.scroll_top - 1;
        let copy_words = line_count * self.columns;

        // Copy from bottom to top to avoid overlap corruption.
        unsafe {
            let src = (self.origin + self.row_bytes * self.scroll_top) as *const u16;
            let dst = (self.origin + self.row_bytes * (self.scroll_top + 1)) as *mut u16;
            ptr::copy(src, dst, copy_words);
        }

        // Fill the top line of the scroll region.
        let top_line = self.origin + self.row_bytes * self.scroll_top;
        self.fill_erase(top_line, self.columns);
    }

    /// Program the CRT controller display origin for EGA/VGA fast scroll.
    fn set_origin(&self) {
        let offset = (self.origin - self.mem_start) >> 1;
        self.write_crtc_word(CRTC_START_HIGH, offset);
    }

    /// Write a 16-bit value across a high/low CRT controller register pair.
    ///
    /// `index_high` selects the high-byte register; the low-byte register is
    /// always the next index.
    fn write_crtc_word(&self, index_high: u8, value: usize) {
        outb_p(index_high, self.port_reg);
        outb_p((value >> 8) as u8, self.port_val);
        outb_p(index_high + 1, self.port_reg);
        outb_p(value as u8, self.port_val);
    }

    // ---- Erase operations ----

    /// Fill `count` cells starting at `addr` with the erase cell.
    fn fill_erase(&self, addr: usize, count: usize) {
        let p = addr as *mut u16;
        for i in 0..count {
            unsafe { ptr::write_volatile(p.add(i), self.erase_cell) };
        }
    }

    /// CSI J — erase in display.
    fn erase_display(&self, mode: u32) {
        match mode {
            0 => {
                let count = (self.screen_end - self.cursor_pos) >> 1;
                self.fill_erase(self.cursor_pos, count);
            }
            1 => {
                let count = (self.cursor_pos - self.origin) >> 1;
                self.fill_erase(self.origin, count);
            }
            2 => {
                let count = self.columns * self.lines;
                self.fill_erase(self.origin, count);
            }
            _ => {}
        }
    }

    /// CSI K — erase in line.
    fn erase_line(&self, mode: u32) {
        match mode {
            0 => {
                if self.cursor_x >= self.columns {
                    return;
                }
                let count = self.columns - self.cursor_x;
                self.fill_erase(self.cursor_pos, count);
            }
            1 => {
                let start = self.cursor_pos - (self.cursor_x << 1);
                let count = if self.cursor_x < self.columns {
                    self.cursor_x
                } else {
                    self.columns
                };
                self.fill_erase(start, count);
            }
            2 => {
                let start = self.cursor_pos - (self.cursor_x << 1);
                self.fill_erase(start, self.columns);
            }
            _ => {}
        }
    }

    // ---- Insert / delete ----

    fn insert_chars(&mut self, count: usize) {
        for _ in 0..count.clamp(1, self.columns) {
            self.insert_char_at_cursor();
        }
    }

    fn insert_char_at_cursor(&mut self) {
        let row_base = self.cursor_pos as *mut u16;
        let mut column = self.cursor_x;
        let mut carry = self.erase_cell;
        while column < self.columns {
            let offset = column - self.cursor_x;
            let displaced = unsafe { ptr::read_volatile(row_base.wrapping_add(offset)) };
            unsafe { ptr::write_volatile(row_base.wrapping_add(offset), carry) };
            carry = displaced;
            column += 1;
        }
    }

    fn delete_chars(&mut self, count: usize) {
        for _ in 0..count.clamp(1, self.columns) {
            self.delete_char_at_cursor();
        }
    }

    fn delete_char_at_cursor(&mut self) {
        if self.cursor_x >= self.columns {
            return;
        }
        let p = self.cursor_pos as *mut u16;
        let mut i = self.cursor_x;
        while i + 1 < self.columns {
            let next = unsafe { ptr::read_volatile(p.wrapping_add(i + 1 - self.cursor_x)) };
            unsafe { ptr::write_volatile(p.wrapping_add(i - self.cursor_x), next) };
            i += 1;
        }
        unsafe { ptr::write_volatile(p.wrapping_add(i - self.cursor_x), self.erase_cell) };
    }

    fn insert_lines(&mut self, count: usize) {
        self.scroll_region_temporarily(|console| {
            for _ in 0..count.clamp(1, console.lines) {
                console.scroll_down();
            }
        });
    }

    fn delete_lines(&mut self, count: usize) {
        self.scroll_region_temporarily(|console| {
            for _ in 0..count.clamp(1, console.lines) {
                console.scroll_up();
            }
        });
    }

    /// Run `body` with the scroll region temporarily set from the cursor row to
    /// the bottom of the screen, restoring the previous region afterward.
    ///
    /// CSI insert-line / delete-line operate within this transient region.
    fn scroll_region_temporarily(&mut self, body: impl FnOnce(&mut Self)) {
        let saved = (self.scroll_top, self.scroll_bottom);
        self.scroll_top = self.cursor_y;
        self.scroll_bottom = self.lines;
        body(self);
        (self.scroll_top, self.scroll_bottom) = saved;
    }

    // ---- SGR (Select Graphic Rendition) ----

    fn set_graphic_rendition(&mut self) {
        for index in 0..=self.parser.param_count {
            self.attribute = match self.parser.params[index] {
                0 | 27 => ATTR_NORMAL, // reset / inverse off
                1 | 4 => ATTR_BOLD,    // bold; underline rendered as bold
                7 => ATTR_INVERSE,     // inverse video
                _ => self.attribute,
            };
        }
    }
}
