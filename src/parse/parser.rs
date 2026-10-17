//! The scan position jq uses for line and column numbers.

pub(in crate::parse) struct ParseErr {
    phrase: &'static str,
    at_eof: bool,
    line: usize,
    column: usize,
}

impl ParseErr {
    pub(in crate::parse) fn ran_out(&self) -> bool {
        self.at_eof
    }

    pub(in crate::parse) fn line(&self) -> usize {
        self.line
    }

    pub(in crate::parse) fn text(&self) -> String {
        if self.at_eof {
            format!(
                "{} at EOF at line {}, column {}",
                self.phrase, self.line, self.column
            )
        } else {
            format!(
                "{} at line {}, column {}",
                self.phrase, self.line, self.column
            )
        }
    }
}

pub(in crate::parse) struct Parser<'a> {
    pub(in crate::parse) src: &'a [u8],
    pub(in crate::parse) index: usize,
    /// jq counts the line from 1.
    pub(in crate::parse) line: usize,
    /// jq counts bytes, starts at 0, and returns to 0 after a newline.
    column: usize,
    /// Nesting of arrays and objects. jq rejects the 257th level.
    pub(in crate::parse) depth: usize,
    /// First parse-buffer index of each file after the first. A NUL hides the
    /// rest of its own file, and the next file continues the token.
    pub(in crate::parse) boundaries: &'a [usize],
    /// EOF does not finish a number or a literal. More bytes may follow.
    pub(in crate::parse) open_ended: bool,
}

impl<'a> Parser<'a> {
    pub(in crate::parse) fn new(src: &'a [u8]) -> Self {
        Self {
            src,
            index: 0,
            line: 1,
            column: 0,
            depth: 0,
            boundaries: &[],
            open_ended: false,
        }
    }

    pub(in crate::parse) fn set_boundaries(&mut self, boundaries: &'a [usize]) {
        self.boundaries = boundaries;
    }

    /// Continue at `index`, keeping the column jq had already counted.
    pub(in crate::parse) fn seek_at(&mut self, index: usize, line: usize, column: usize) {
        self.index = index;
        self.line = line;
        self.column = column;
    }

    pub(in crate::parse) fn at_boundary(&self) -> bool {
        self.boundaries.binary_search(&self.index).is_ok()
    }

    pub(in crate::parse) fn eof(&self) -> bool {
        self.index >= self.src.len()
    }

    /// Continue a scan that already passed `index` bytes. `line` is jq's
    /// count at that byte, and the column starts over at 0.
    pub(in crate::parse) fn seek(&mut self, index: usize, line: usize) {
        self.index = index;
        self.line = line;
        self.column = 0;
    }

    pub(in crate::parse) fn column(&self) -> usize {
        self.column
    }

    pub(in crate::parse) fn peek(&self) -> Option<char> {
        self.char_at(self.index)
    }

    pub(in crate::parse) fn char_at(&self, index: usize) -> Option<char> {
        decode_at(self.src, index).map(|(c, _)| c)
    }

    pub(in crate::parse) fn bump(&mut self) -> Option<char> {
        let (c, width) = decode_at(self.src, self.index)?;
        self.index += width;
        if c == '\n' {
            self.line += 1;
            self.column = 0;
        } else {
            // Width is the input bytes, not U+FFFD's own three-byte encoding.
            self.column += width;
        }
        Some(c)
    }

    /// One source byte. Columns stay on the original bytes inside a string,
    /// where a later sanitizer may turn several of those bytes into one scalar.
    pub(in crate::parse) fn take_byte(&mut self) -> Option<u8> {
        if self.index >= self.src.len() {
            return None;
        }
        let byte = self.src[self.index];
        self.index += 1;
        if byte == b'\n' {
            self.line += 1;
            self.column = 0;
        } else {
            self.column += 1;
        }
        Some(byte)
    }

    pub(in crate::parse) fn skip_ws(&mut self) {
        while self.peek().is_some_and(is_ws) {
            self.bump();
        }
    }

    pub(in crate::parse) fn fail(&self, phrase: &'static str, at_eof: bool) -> ParseErr {
        ParseErr {
            phrase,
            at_eof,
            line: self.line,
            column: self.column,
        }
    }

    pub(in crate::parse) fn bump_fail(&mut self, phrase: &'static str) -> ParseErr {
        self.bump();
        self.fail(phrase, false)
    }
}

/// One scalar at `index`, and how many input bytes it occupied.
///
/// Used outside string contents. A following byte that cannot continue the
/// sequence stays put, so a quote or space is still a delimiter.
fn decode_at(src: &[u8], index: usize) -> Option<(char, usize)> {
    let rest = src.get(index..)?;
    if rest.is_empty() {
        return None;
    }
    Some(decode_utf8(rest, false))
}

/// String contents after escapes. The slice ends where the string ends, so a
/// lead with too few bytes left takes the whole tail, not only its continuations.
pub(crate) fn sanitize_content(bytes: &[u8]) -> String {
    let mut out = String::new();
    let mut index = 0;
    while index < bytes.len() {
        let (ch, width) = decode_utf8(&bytes[index..], true);
        out.push(ch);
        index += width;
    }
    out
}

fn decode_utf8(bytes: &[u8], hard_end: bool) -> (char, usize) {
    let lead = bytes[0];
    let (width, min_value, mut value) = match lead {
        0x00..=0x7F => return (lead as char, 1),
        0xC2..=0xDF => (2, 0x80u32, u32::from(lead & 0x1F)),
        0xE0..=0xEF => (3, 0x800, u32::from(lead & 0x0F)),
        0xF0..=0xF4 => (4, 0x10000, u32::from(lead & 0x07)),
        _ => return ('\u{FFFD}', 1),
    };
    // The end of a string is a hard end: jq folds a short tail into one
    // U+FFFD, including bytes that are not continuations.
    if hard_end && bytes.len() < width {
        return ('\u{FFFD}', bytes.len());
    }
    let available = bytes.len().min(width);
    for i in 1..available {
        if !is_continuation(bytes[i]) {
            return ('\u{FFFD}', i);
        }
        value = (value << 6) | u32::from(bytes[i] & 0x3F);
    }
    if available < width
        || value < min_value
        || value > 0x10FFFF
        || (0xD800..=0xDFFF).contains(&value)
    {
        let taken = if available < width { available } else { width };
        return ('\u{FFFD}', taken);
    }
    match char::from_u32(value) {
        Some(c) => (c, width),
        None => ('\u{FFFD}', width),
    }
}

fn is_continuation(byte: u8) -> bool {
    byte & 0xC0 == 0x80
}

fn is_ws(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r')
}

pub(in crate::parse) fn is_delimiter(c: char) -> bool {
    is_ws(c) || matches!(c, '"' | '[' | ']' | '{' | '}' | ',' | ':')
}
