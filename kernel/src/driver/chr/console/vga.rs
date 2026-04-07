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

use crate::pmio::{inb_p, outb_p};
use crate::sync::KernelCell;

/// BIOS data area addresses written by setup.s during boot.
const ORIG_X: *const u8 = 0x90000 as *const u8;
const ORIG_Y: *const u8 = 0x90001 as *const u8;
const ORIG_VIDEO_MODE: *const u16 = 0x90006 as *const u16;
const ORIG_VIDEO_EGA_BX: *const u16 = 0x9000a as *const u16;

const MAX_ANSI_PARAMS: usize = 16;

/// Display adapter type detected during initialization.
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum DisplayType {
    Mda = 0x10,
    Cga = 0x11,
    EgaMonochrome = 0x20,
    EgaColor = 0x21,
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
    Normal,
    Escape,
    CsiEntry,
    CsiParam,
}

/// ANSI CSI parameter accumulator.
struct AnsiParser {
    state: AnsiState,
    params: [u32; MAX_ANSI_PARAMS],
    param_count: usize,
    is_question_mark: bool,
}

impl AnsiParser {
    const fn new() -> Self {
        Self {
            state: AnsiState::Normal,
            params: [0; MAX_ANSI_PARAMS],
            param_count: 0,
            is_question_mark: false,
        }
    }

    fn reset_params(&mut self) {
        self.params = [0; MAX_ANSI_PARAMS];
        self.param_count = 0;
        self.is_question_mark = false;
    }
}

/// VGA text-mode console state.
///
/// Manages the entire screen geometry, cursor position, scroll region, attribute
/// state, and the ANSI escape parser. Protected by a global `KernelCell`.
pub struct VgaConsole {
    display_type: DisplayType,
    columns: usize,
    lines: usize,
    row_bytes: usize,
    mem_start: usize,
    mem_end: usize,
    port_reg: u16,
    port_val: u16,
    erase_cell: u16,

    origin: usize,
    screen_end: usize,
    cursor_pos: usize,
    cursor_x: usize,
    cursor_y: usize,
    scroll_top: usize,
    scroll_bottom: usize,

    attribute: u8,
    saved_x: usize,
    saved_y: usize,

    parser: AnsiParser,
}

pub static CONSOLE: KernelCell<VgaConsole> = KernelCell::new(VgaConsole::uninitialized());

impl VgaConsole {
    const fn uninitialized() -> Self {
        Self {
            display_type: DisplayType::Cga,
            columns: 80,
            lines: 25,
            row_bytes: 160,
            mem_start: 0xb8000,
            mem_end: 0xba000,
            port_reg: 0x3d4,
            port_val: 0x3d5,
            erase_cell: 0x0720,
            origin: 0xb8000,
            screen_end: 0xb8000 + 25 * 160,
            cursor_pos: 0xb8000,
            cursor_x: 0,
            cursor_y: 0,
            scroll_top: 0,
            scroll_bottom: 25,
            attribute: 0x07,
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
        self.lines = 25;
        self.row_bytes = cols * 2;
        self.erase_cell = 0x0720;

        if mode == 7 {
            // Monochrome display
            self.mem_start = 0xb0000;
            self.port_reg = 0x3b4;
            self.port_val = 0x3b5;
            if (ega_bx & 0xff) != 0x10 {
                self.display_type = DisplayType::EgaMonochrome;
                self.mem_end = 0xb8000;
            } else {
                self.display_type = DisplayType::Mda;
                self.mem_end = 0xb2000;
            }
        } else {
            // Color display
            self.mem_start = 0xb8000;
            self.port_reg = 0x3d4;
            self.port_val = 0x3d5;
            if (ega_bx & 0xff) != 0x10 {
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
        self.attribute = 0x07;
        self.parser = AnsiParser::new();

        // Continue from wherever early-boot VGA output left off, so all
        // previous output is preserved on screen.
        let (x, y) = crate::logging::early_vga_cursor();
        self.move_cursor(x, y);
        self.sync_hardware_cursor();
    }

    /// Write one byte through the VT102 parser state machine.
    pub fn write_byte(&mut self, c: u8) {
        match self.parser.state {
            AnsiState::Normal => self.handle_normal(c),
            AnsiState::Escape => self.handle_escape(c),
            AnsiState::CsiEntry => self.handle_csi_entry(c),
            AnsiState::CsiParam => self.handle_csi_param(c),
        }
    }

    fn handle_normal(&mut self, c: u8) {
        if c > 31 && c < 127 {
            if self.cursor_x >= self.columns {
                self.cursor_x -= self.columns;
                self.cursor_pos -= self.row_bytes;
                self.line_feed();
            }
            self.put_char(c);
            self.cursor_pos += 2;
            self.cursor_x += 1;
        } else {
            match c {
                0x1b => self.parser.state = AnsiState::Escape,
                b'\n' | 11 | 12 => self.line_feed(),
                b'\r' => self.carriage_return(),
                0x7f => self.delete(),
                8 => self.backspace(),
                b'\t' => self.tab(),
                7 => self.bell(),
                _ => {}
            }
        }
    }

    fn handle_escape(&mut self, c: u8) {
        self.parser.state = AnsiState::Normal;
        match c {
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

    fn handle_csi_entry(&mut self, c: u8) {
        if c == b'?' {
            self.parser.is_question_mark = true;
            self.parser.state = AnsiState::CsiParam;
            return;
        }
        self.parser.state = AnsiState::CsiParam;
        self.handle_csi_param(c);
    }

    fn handle_csi_param(&mut self, c: u8) {
        if c == b';' && self.parser.param_count < MAX_ANSI_PARAMS - 1 {
            self.parser.param_count += 1;
            return;
        }
        if c.is_ascii_digit() {
            let idx = self.parser.param_count;
            self.parser.params[idx] = self.parser.params[idx] * 10 + (c - b'0') as u32;
            return;
        }
        // Final character — dispatch CSI command.
        self.parser.state = AnsiState::Normal;
        self.dispatch_csi(c);
    }

    /// Dispatch a completed CSI sequence based on the final character.
    fn dispatch_csi(&mut self, final_ch: u8) {
        let p = self.parser.params;

        match final_ch {
            b'G' | b'`' => {
                let col = if p[0] > 0 { p[0] - 1 } else { 0 };
                self.move_cursor(col as usize, self.cursor_y);
            }
            b'A' => {
                let n = if p[0] == 0 { 1 } else { p[0] };
                self.move_cursor(self.cursor_x, self.cursor_y.saturating_sub(n as usize));
            }
            b'B' | b'e' => {
                let n = if p[0] == 0 { 1 } else { p[0] };
                self.move_cursor(self.cursor_x, self.cursor_y + n as usize);
            }
            b'C' | b'a' => {
                let n = if p[0] == 0 { 1 } else { p[0] };
                self.move_cursor(self.cursor_x + n as usize, self.cursor_y);
            }
            b'D' => {
                let n = if p[0] == 0 { 1 } else { p[0] };
                self.move_cursor(self.cursor_x.saturating_sub(n as usize), self.cursor_y);
            }
            b'E' => {
                let n = if p[0] == 0 { 1 } else { p[0] };
                self.move_cursor(0, self.cursor_y + n as usize);
            }
            b'F' => {
                let n = if p[0] == 0 { 1 } else { p[0] };
                self.move_cursor(0, self.cursor_y.saturating_sub(n as usize));
            }
            b'd' => {
                let row = if p[0] > 0 { p[0] - 1 } else { 0 };
                self.move_cursor(self.cursor_x, row as usize);
            }
            b'H' | b'f' => {
                let row = if p[0] > 0 { p[0] - 1 } else { 0 };
                let col = if p[1] > 0 { p[1] - 1 } else { 0 };
                self.move_cursor(col as usize, row as usize);
            }
            b'J' => self.erase_display(p[0]),
            b'K' => self.erase_line(p[0]),
            b'L' => self.insert_lines(p[0]),
            b'M' => self.delete_lines(p[0]),
            b'P' => self.delete_chars(p[0]),
            b'@' => self.insert_chars(p[0]),
            b'm' => self.set_graphic_rendition(),
            b'r' => {
                let top = if p[0] > 0 { p[0] - 1 } else { 0 };
                let bottom = if p[1] == 0 { self.lines as u32 } else { p[1] };
                if (top as usize) < (bottom as usize) && (bottom as usize) <= self.lines {
                    self.scroll_top = top as usize;
                    self.scroll_bottom = bottom as usize;
                }
            }
            b's' => self.save_cursor(),
            b'u' => self.restore_cursor(),
            _ => {}
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
        outb_p(14, self.port_reg);
        outb_p((offset >> 8) as u8, self.port_val);
        outb_p(15, self.port_reg);
        outb_p(offset as u8, self.port_val);
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
    fn put_char(&self, c: u8) {
        let addr = self.cursor_pos as *mut u16;
        let cell = ((self.attribute as u16) << 8) | c as u16;
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
        // PIT channel 2 beep: enable speaker, set frequency ~750 Hz.
        let port_b = inb_p(0x61);
        outb_p(port_b | 3, 0x61);
        outb_p(0xb6, 0x43);
        outb_p(0x37, 0x42);
        crate::pmio::outb(0x06, 0x42);
    }

    // ---- Scrolling ----

    /// Scroll the display up one line within the scroll region.
    fn scroll_up(&mut self) {
        let is_ega = self.display_type == DisplayType::EgaColor
            || self.display_type == DisplayType::EgaMonochrome;

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
        outb_p(12, self.port_reg);
        outb_p((offset >> 8) as u8, self.port_val);
        outb_p(13, self.port_reg);
        outb_p(offset as u8, self.port_val);
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

    fn insert_chars(&mut self, mut nr: u32) {
        if nr > self.columns as u32 {
            nr = self.columns as u32;
        }
        if nr == 0 {
            nr = 1;
        }
        for _ in 0..nr {
            self.insert_char_at_cursor();
        }
    }

    fn insert_char_at_cursor(&mut self) {
        let p = self.cursor_pos as *mut u16;
        let mut i = self.cursor_x;
        let mut old = self.erase_cell;
        while i < self.columns {
            let tmp = unsafe { ptr::read_volatile(p.wrapping_add(i - self.cursor_x)) };
            unsafe { ptr::write_volatile(p.wrapping_add(i - self.cursor_x), old) };
            old = tmp;
            i += 1;
        }
    }

    fn delete_chars(&mut self, mut nr: u32) {
        if nr > self.columns as u32 {
            nr = self.columns as u32;
        }
        if nr == 0 {
            nr = 1;
        }
        for _ in 0..nr {
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

    fn insert_lines(&mut self, mut nr: u32) {
        if nr > self.lines as u32 {
            nr = self.lines as u32;
        }
        if nr == 0 {
            nr = 1;
        }
        let old_top = self.scroll_top;
        let old_bottom = self.scroll_bottom;
        self.scroll_top = self.cursor_y;
        self.scroll_bottom = self.lines;
        for _ in 0..nr {
            self.scroll_down();
        }
        self.scroll_top = old_top;
        self.scroll_bottom = old_bottom;
    }

    fn delete_lines(&mut self, mut nr: u32) {
        if nr > self.lines as u32 {
            nr = self.lines as u32;
        }
        if nr == 0 {
            nr = 1;
        }
        let old_top = self.scroll_top;
        let old_bottom = self.scroll_bottom;
        self.scroll_top = self.cursor_y;
        self.scroll_bottom = self.lines;
        for _ in 0..nr {
            self.scroll_up();
        }
        self.scroll_top = old_top;
        self.scroll_bottom = old_bottom;
    }

    // ---- SGR (Select Graphic Rendition) ----

    fn set_graphic_rendition(&mut self) {
        for i in 0..=self.parser.param_count {
            match self.parser.params[i] {
                0 => self.attribute = 0x07,  // Reset / normal
                1 => self.attribute = 0x0f,  // Bold (bright foreground)
                4 => self.attribute = 0x0f,  // Underline (rendered as bold on VGA)
                7 => self.attribute = 0x70,  // Inverse video
                27 => self.attribute = 0x07, // Inverse off
                _ => {}
            }
        }
    }
}
