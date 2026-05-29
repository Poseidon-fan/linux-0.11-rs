//! `/pattern` and `?pattern` search.
//!
//! Backed by the `regex` crate — it's already in the workspace for
//! `grep`, and re-using it gets us extended POSIX syntax for free. vi's
//! historic regex dialect is close enough that most one-off searches
//! Just Work. For obscure literal characters (`.`, `*`, `[`) users have
//! to remember to backslash-escape, same as in modern vim with
//! `:set magic`.

use alloc::string::{String, ToString};

use regex::Regex;

use crate::buffer::Buffer;

/// Compiled search state shared between `/`, `?`, `n`, `N`.
pub struct Search {
    /// The last pattern the user actually typed, kept so `n`/`N` can
    /// recompile if needed (currently unused but cheap to store).
    pub pattern: String,
    /// The compiled regex, lazily built once per pattern change.
    pub compiled: Option<Regex>,
    /// Direction of the most recent search: `true` for forward, `false`
    /// for backward.
    pub forward: bool,
}

impl Search {
    pub fn new() -> Self {
        Self {
            pattern: String::new(),
            compiled: None,
            forward: true,
        }
    }

    /// Stores a new pattern and tries to compile it. Returns `Err` with
    /// the regex error message if it fails to compile, so the editor can
    /// show it on the status line.
    pub fn set(&mut self, pattern: &str, forward: bool) -> Result<(), String> {
        if pattern.is_empty() {
            return Err("empty pattern".to_string());
        }
        let re = Regex::new(pattern).map_err(|e| e.to_string())?;
        self.pattern = pattern.to_string();
        self.compiled = Some(re);
        self.forward = forward;
        Ok(())
    }

    /// Moves `buf.row` / `buf.col` to the next match in the chosen
    /// direction. Wraps around the end of the buffer. Returns `false`
    /// if no match exists anywhere.
    ///
    /// `from_cursor` controls whether a match at the exact cursor
    /// position counts (`true`, used for the initial `/pat` lookup) or
    /// whether the search must advance past it first (`false`, used by
    /// `n` / `N`).
    pub fn find_next(&self, buf: &mut Buffer) -> bool {
        self.find(buf, self.forward, false)
    }

    /// Same as [`find_next`] but matches at the current cursor count.
    /// Used immediately after `/pat` / `?pat` so the cursor sits on the
    /// first match, matching `vim`'s default behaviour.
    pub fn find_first(&self, buf: &mut Buffer) -> bool {
        self.find(buf, self.forward, true)
    }

    /// Searches in the opposite direction (used by `N`).
    pub fn find_prev(&self, buf: &mut Buffer) -> bool {
        self.find(buf, !self.forward, false)
    }

    fn find(&self, buf: &mut Buffer, forward: bool, include_cursor: bool) -> bool {
        let Some(re) = self.compiled.as_ref() else {
            return false;
        };
        let start_row = buf.row;
        let start_col = buf.col;
        let line_count = buf.line_count();

        if forward {
            // From the current cursor to the end of the current line,
            // then through subsequent lines, then wrap to the top.
            let line = buf.line(start_row);
            let scan_from = if include_cursor {
                start_col.min(line.len())
            } else {
                (start_col + 1).min(line.len())
            };
            if let Some(m) = re.find_at(line, scan_from) {
                buf.col = m.start();
                return true;
            }
            for offset in 1..=line_count {
                let r = (start_row + offset) % line_count;
                let hit = re.find(buf.line(r)).map(|m| m.start());
                if let Some(col) = hit {
                    buf.row = r;
                    buf.col = col;
                    return true;
                }
                if r == start_row {
                    break;
                }
            }
        } else {
            // Backward: scan the current line up to (but not including)
            // the cursor, then previous lines, then wrap.
            let cutoff = if include_cursor {
                (start_col + 1).min(buf.line(start_row).len())
            } else {
                start_col
            };
            if cutoff > 0 {
                if let Some(m) = last_match_in(re, &buf.line(start_row)[..cutoff]) {
                    buf.col = m;
                    return true;
                }
            }
            for offset in 1..=line_count {
                let r = (start_row + line_count - offset) % line_count;
                let hit = last_match_in(re, buf.line(r));
                if let Some(col) = hit {
                    buf.row = r;
                    buf.col = col;
                    return true;
                }
                if r == start_row {
                    break;
                }
            }
        }
        false
    }
}

/// Returns the byte offset of the last match in `haystack`, or `None`.
fn last_match_in(re: &Regex, haystack: &str) -> Option<usize> {
    let mut last = None;
    for m in re.find_iter(haystack) {
        last = Some(m.start());
    }
    last
}
