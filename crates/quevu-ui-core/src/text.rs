//! Turning the bytes of a chunk into a line a console can draw.

/// Width of a tab stop, in cells.
const TAB: usize = 8;

/// Makes the data of a chunk drawable on one line of a monospace grid.
///
/// - Invalid UTF-8 becomes the replacement character.
/// - Tabs are expanded to the next tab stop.
/// - ANSI escape sequences (colors, cursor movement) are dropped: the
///   applications that end up here were written for a terminal.
/// - Any other control character, line ends included, becomes a middle
///   dot, so a chunk is always exactly one line.
pub fn display_text(data: &[u8]) -> String {
    let src = String::from_utf8_lossy(data);
    let mut out = String::with_capacity(src.len());
    let mut col = 0usize;
    let mut chars = src.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\t' => {
                let n = TAB - (col % TAB);
                for _ in 0..n {
                    out.push(' ');
                }
                col += n;
            }
            '\u{1b}' => {
                if chars.peek() == Some(&'[') {
                    chars.next();
                    // Parameters and intermediates, then one final byte.
                    for f in chars.by_ref() {
                        if ('\u{40}'..='\u{7e}').contains(&f) {
                            break;
                        }
                    }
                } else {
                    // A two-character escape.
                    chars.next();
                }
            }
            c if c.is_control() => {
                out.push('\u{b7}');
                col += 1;
            }
            c => {
                out.push(c);
                col += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::display_text;

    #[test]
    fn plain_text_is_untouched() {
        assert_eq!(display_text("GET /index 200 áñ".as_bytes()), "GET /index 200 áñ");
    }

    #[test]
    fn tabs_go_to_the_next_stop() {
        assert_eq!(display_text(b"a\tb"), "a       b");
        assert_eq!(display_text(b"12345678\tb"), "12345678        b");
        assert_eq!(display_text(b"\t"), "        ");
    }

    #[test]
    fn ansi_sequences_are_dropped() {
        assert_eq!(display_text(b"\x1b[31mred\x1b[0m plain"), "red plain");
        assert_eq!(display_text(b"\x1b[1;32;40mok"), "ok");
        assert_eq!(display_text(b"a\x1bMb"), "ab");
        // A sequence cut short swallows the rest and nothing breaks.
        assert_eq!(display_text(b"a\x1b[12"), "a");
    }

    #[test]
    fn a_chunk_is_always_one_line() {
        assert_eq!(display_text(b"two\nlines\r\n"), "two\u{b7}lines\u{b7}\u{b7}");
        assert_eq!(display_text(b"nul\x00"), "nul\u{b7}");
    }

    #[test]
    fn invalid_utf8_does_not_break() {
        assert_eq!(display_text(b"a\xffb"), "a\u{fffd}b");
    }
}
