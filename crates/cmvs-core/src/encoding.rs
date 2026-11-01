//! The double-byte encoding of game text.
//!
//! The engine reads text as Shift-JIS: 0x140008080 tests lead bytes and the
//! layout code names a few punctuation codes. Chinese patches hook those to
//! GBK and append GBK strings to the scripts, so a host running such a game
//! switches to GBK on the thread that runs it, before it loads the game.

use std::cell::Cell;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextEncoding {
    ShiftJis,
    Gbk,
}

thread_local! {
    static CURRENT: Cell<TextEncoding> = const { Cell::new(TextEncoding::ShiftJis) };
}

/// Sets the encoding of the games run on this thread (Shift-JIS by default).
pub fn set(encoding: TextEncoding) {
    CURRENT.with(|c| c.set(encoding));
}

pub fn current() -> TextEncoding {
    CURRENT.with(Cell::get)
}

/// 0x140008080: whether `byte` starts a double-byte character (the trail
/// byte is not checked).
pub fn lead(byte: u8) -> bool {
    match current() {
        TextEncoding::ShiftJis => (0x81..=0x9f).contains(&byte) || (0xe0..=0xfc).contains(&byte),
        TextEncoding::Gbk => (0x81..=0xfe).contains(&byte),
    }
}

/// 、。」』: kept on the full line instead of wrapping.
pub fn no_break(code: u16) -> bool {
    match current() {
        TextEncoding::ShiftJis => matches!(code, 0x8141 | 0x8142 | 0x8176 | 0x8178),
        TextEncoding::Gbk => matches!(code, 0xa1a2 | 0xa1a3 | 0xa1b9 | 0xa1bb),
    }
}

/// ゛, drawn a fifth of the size to the left with a shorter advance.
pub fn voiced_mark() -> u16 {
    match current() {
        TextEncoding::ShiftJis => 0x814a,
        TextEncoding::Gbk => 0xa961,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lead_bytes_and_marks_follow_the_encoding() {
        assert!(lead(0x82) && !lead(0xa1) && !lead(0x41));
        assert!(no_break(0x8142) && voiced_mark() == 0x814a);
        set(TextEncoding::Gbk);
        let gbk = (
            lead(0xa1),
            lead(0xd5),
            lead(0xfe),
            lead(0x41),
            no_break(0xa1a3),
            voiced_mark(),
        );
        set(TextEncoding::ShiftJis);
        assert_eq!(gbk, (true, true, true, false, true, 0xa961));
    }
}
