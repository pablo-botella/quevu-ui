//! Turning the bytes of a chunk into a line a console can draw.
//!
//! Applications written for a terminal color their output with ANSI
//! escape sequences (ECMA-48, "SGR"): `ESC [ 31 m` switches to red,
//! `ESC [ 0 m` back to normal. They are a standard, and they travel inside
//! the text. Here they are taken out of the text and turned into spans of
//! color, so a console draws clean text in color, and anything that looks
//! at the text (a selection, a copy, a search) never sees an escape code.

/// Width of a tab stop, in cells.
const TAB: usize = 8;

/// A run of colored text inside a line. Columns are UTF-16 units, the
/// unit the native toolkits count in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: u32,
    pub len: u32,
    /// The color of the text, as 0xRRGGBB.
    pub rgb: u32,
}

/// The sixteen named colors, chosen to read well on a dark background.
/// Black is drawn as a dark gray: true black would be invisible.
const PALETTE: [u32; 16] = [
    0x666666, 0xcd3131, 0x0dbc79, 0xe5e510, 0x3b8eea, 0xbc3fbc, 0x11a8cd, 0xe5e5e5, //
    0x808080, 0xf14c4c, 0x23d18b, 0xf5f543, 0x6cb6ff, 0xd670d6, 0x29b8db, 0xffffff,
];

/// The 256-color table: the sixteen named ones, a 6x6x6 cube and a ramp
/// of grays.
fn indexed(n: u32) -> u32 {
    match n {
        0..=15 => PALETTE[n as usize],
        16..=231 => {
            let n = n - 16;
            let level = |v: u32| if v == 0 { 0 } else { 55 + v * 40 };
            (level(n / 36) << 16) | (level((n / 6) % 6) << 8) | level(n % 6)
        }
        232..=255 => {
            let g = 8 + (n - 232) * 10;
            (g << 16) | (g << 8) | g
        }
        _ => PALETTE[7],
    }
}

/// The color state an SGR sequence acts on.
#[derive(Clone, Copy, Default)]
struct Pen {
    /// `None` is the default color of the console.
    color: Option<Named>,
    bold: bool,
}

#[derive(Clone, Copy)]
enum Named {
    /// One of the eight basic colors: bold makes it bright.
    Basic(u32),
    Fixed(u32),
}

impl Pen {
    fn rgb(self) -> Option<u32> {
        match self.color {
            Some(Named::Basic(n)) => Some(PALETTE[(n + if self.bold { 8 } else { 0 }) as usize]),
            Some(Named::Fixed(rgb)) => Some(rgb),
            // Bold with no color is drawn as bright white.
            None if self.bold => Some(PALETTE[15]),
            None => None,
        }
    }

    /// Applies the parameters of one `ESC [ ... m` sequence.
    fn apply(&mut self, params: &str) {
        let mut it = params.split(';').map(|p| p.parse::<u32>().unwrap_or(0));
        while let Some(p) = it.next() {
            match p {
                0 => *self = Pen::default(),
                1 => self.bold = true,
                22 => self.bold = false,
                30..=37 => self.color = Some(Named::Basic(p - 30)),
                39 => self.color = None,
                90..=97 => self.color = Some(Named::Fixed(PALETTE[(p - 90 + 8) as usize])),
                // Extended colors carry their own parameters. Background
                // ones (48) are read and ignored, so they do not get
                // mistaken for something else.
                38 | 48 => {
                    let rgb = match it.next() {
                        Some(5) => it.next().map(indexed),
                        Some(2) => {
                            let (r, g, b) = (it.next(), it.next(), it.next());
                            match (r, g, b) {
                                (Some(r), Some(g), Some(b)) => {
                                    Some(((r & 0xff) << 16) | ((g & 0xff) << 8) | (b & 0xff))
                                }
                                _ => None,
                            }
                        }
                        _ => None,
                    };
                    if p == 38 {
                        if let Some(rgb) = rgb {
                            self.color = Some(Named::Fixed(rgb));
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

/// Makes the data of a chunk drawable on one line of a monospace grid,
/// and returns the colors it asked for.
///
/// - Invalid UTF-8 becomes the replacement character.
/// - Tabs are expanded to the next tab stop.
/// - ANSI color sequences become spans; every other escape sequence
///   (cursor movement and the like) is dropped.
/// - Any other control character, line ends included, becomes a middle
///   dot, so a chunk is always exactly one line.
///
/// Color does not carry over from one chunk to the next: every line
/// starts in the default color.
pub fn display_line(data: &[u8]) -> (String, Vec<Span>) {
    let src = String::from_utf8_lossy(data);
    let mut out = String::with_capacity(src.len());
    let mut spans: Vec<Span> = Vec::new();
    let mut pen = Pen::default();
    let mut col = 0usize; // in cells, for tab stops
    let mut units = 0u32; // in UTF-16 units, for spans

    let mut push = |out: &mut String, c: char, pen: Pen, units: &mut u32| {
        out.push(c);
        let n = c.len_utf16() as u32;
        if let Some(rgb) = pen.rgb() {
            match spans.last_mut() {
                Some(s) if s.rgb == rgb && s.start + s.len == *units => s.len += n,
                _ => spans.push(Span { start: *units, len: n, rgb }),
            }
        }
        *units += n;
    };

    let mut chars = src.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\t' => {
                let n = TAB - (col % TAB);
                for _ in 0..n {
                    push(&mut out, ' ', pen, &mut units);
                }
                col += n;
            }
            '\u{1b}' => {
                if chars.peek() == Some(&'[') {
                    chars.next();
                    // Parameters and intermediates, then one final byte.
                    let mut params = String::new();
                    let mut last = None;
                    for f in chars.by_ref() {
                        if ('\u{40}'..='\u{7e}').contains(&f) {
                            last = Some(f);
                            break;
                        }
                        params.push(f);
                    }
                    if last == Some('m') {
                        pen.apply(&params);
                    }
                } else {
                    // A two-character escape.
                    chars.next();
                }
            }
            c if c.is_control() => {
                push(&mut out, '\u{b7}', pen, &mut units);
                col += 1;
            }
            c => {
                push(&mut out, c, pen, &mut units);
                col += 1;
            }
        }
    }
    (out, spans)
}

/// The text of a chunk alone, without its colors.
pub fn display_text(data: &[u8]) -> String {
    display_line(data).0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_is_untouched() {
        let (text, spans) = display_line("GET /index 200 áñ".as_bytes());
        assert_eq!(text, "GET /index 200 áñ");
        assert!(spans.is_empty());
    }

    #[test]
    fn tabs_go_to_the_next_stop() {
        assert_eq!(display_text(b"a\tb"), "a       b");
        assert_eq!(display_text(b"12345678\tb"), "12345678        b");
        assert_eq!(display_text(b"\t"), "        ");
    }

    #[test]
    fn ansi_sequences_leave_the_text_clean() {
        assert_eq!(display_text(b"\x1b[31mred\x1b[0m plain"), "red plain");
        assert_eq!(display_text(b"\x1b[1;32;40mok"), "ok");
        assert_eq!(display_text(b"a\x1bMb"), "ab");
        // Cursor movement is dropped, not drawn.
        assert_eq!(display_text(b"a\x1b[2Kb\x1b[10;5Hc"), "abc");
        // A sequence cut short swallows the rest and nothing breaks.
        assert_eq!(display_text(b"a\x1b[12"), "a");
    }

    #[test]
    fn colors_become_spans() {
        let (text, spans) = display_line(b"12:00 \x1b[31mERROR\x1b[0m disk \x1b[33mfull\x1b[39m!");
        assert_eq!(text, "12:00 ERROR disk full!");
        assert_eq!(
            spans,
            vec![
                Span { start: 6, len: 5, rgb: PALETTE[1] },
                Span { start: 17, len: 4, rgb: PALETTE[3] },
            ]
        );
    }

    #[test]
    fn bold_brightens_and_resets() {
        let (_, spans) = display_line(b"\x1b[1;31mA\x1b[22mB\x1b[0mC\x1b[1mD");
        assert_eq!(
            spans,
            vec![
                Span { start: 0, len: 1, rgb: PALETTE[9] },
                Span { start: 1, len: 1, rgb: PALETTE[1] },
                Span { start: 3, len: 1, rgb: PALETTE[15] },
            ]
        );
    }

    #[test]
    fn bright_indexed_and_true_colors() {
        let (_, spans) = display_line(b"\x1b[92mA\x1b[38;5;196mB\x1b[38;2;1;2;3mC\x1b[38;5;244mD");
        assert_eq!(
            spans,
            vec![
                Span { start: 0, len: 1, rgb: PALETTE[10] },
                Span { start: 1, len: 1, rgb: 0xff0000 },
                Span { start: 2, len: 1, rgb: 0x010203 },
                Span { start: 3, len: 1, rgb: 0x808080 },
            ]
        );
    }

    #[test]
    fn background_colors_are_read_and_ignored() {
        // The parameters of a background color must not be taken for
        // foreground ones: 48;5;31 is not "31, red".
        let (text, spans) = display_line(b"\x1b[48;5;31mA\x1b[48;2;31;32;33mB\x1b[41mC");
        assert_eq!(text, "ABC");
        assert!(spans.is_empty(), "{spans:?}");
    }

    #[test]
    fn spans_count_in_utf16_units_and_merge() {
        // An emoji outside the basic plane takes two units.
        let (text, spans) = display_line("\x1b[32m\u{1f600}ok\x1b[32m!\x1b[0m".as_bytes());
        assert_eq!(text, "\u{1f600}ok!");
        assert_eq!(spans, vec![Span { start: 0, len: 5, rgb: PALETTE[2] }]);
    }

    #[test]
    fn color_spans_cover_expanded_tabs() {
        let (text, spans) = display_line(b"\x1b[34ma\tb");
        assert_eq!(text, "a       b");
        assert_eq!(spans, vec![Span { start: 0, len: 9, rgb: PALETTE[4] }]);
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
