//! How much one recalculation may do, so no sheet — a 1.3 KB CSV of `=A1&A1` doubling down 40
//! rows, or a mind's `set_cell` chain — can take all the memory or the window's thread (security
//! review of #727).
//!
//! Three bounds: no text a formula makes is longer than a cell holds; all the text formulas make
//! in one recalculation together stays under [`MAX_TEXT_BYTES`]; and the cells read, and the
//! links between formulas (`sheet::recalc`), each stay under [`MAX_WORK`]. Past any of them the
//! formula shows `#VALUE!` with a sentence, and nothing is allocated first.

use std::cell::Cell;

use super::value::{Error, Kind};

/// The most characters one cell holds, typed or worked out.
pub const MAX_TEXT: usize = 32_768;
/// The most bytes of text the formulas of one sheet make, together.
pub const MAX_TEXT_BYTES: usize = 64 * 1024 * 1024;
/// The most cell reads one recalculation makes, and the most links between formulas it follows.
pub const MAX_WORK: usize = 20_000_000;

#[derive(Debug)]
pub struct Budget {
    work: Cell<usize>,
    text: Cell<usize>,
}

impl Default for Budget {
    fn default() -> Self {
        Self { work: Cell::new(MAX_WORK), text: Cell::new(MAX_TEXT_BYTES) }
    }
}

impl Budget {
    /// Spend `n` units of work, or say the sheet is too big to work out.
    pub fn work(&self, n: usize) -> Result<(), Error> {
        match self.work.get().checked_sub(n) {
            Some(left) => {
                self.work.set(left);
                Ok(())
            }
            None => {
                self.work.set(0);
                Err(Error::new(
                    Kind::Value,
                    format!("the sheet reads more than {MAX_WORK} cells to work out, so this one was left undone"),
                ))
            }
        }
    }

    /// Keep `bytes` of text a formula made, or refuse it once the sheet's text is spent.
    pub fn text(&self, bytes: usize) -> Result<(), Error> {
        match self.text.get().checked_sub(bytes) {
            Some(left) => {
                self.text.set(left);
                Ok(())
            }
            None => Err(Error::new(
                Kind::Value,
                format!("the sheet's formulas make more than {} MB of text", MAX_TEXT_BYTES / (1024 * 1024)),
            )),
        }
    }
}

/// Refuse a text of `chars` characters before it is built, if it is longer than a cell holds.
pub fn fits(chars: usize) -> Result<(), Error> {
    if chars > MAX_TEXT {
        return Err(Error::new(
            Kind::Value,
            format!("the text would be {chars} characters, more than the {MAX_TEXT} one cell holds"),
        ));
    }
    Ok(())
}
