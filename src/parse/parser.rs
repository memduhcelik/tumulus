//! The scan position jq uses for line and column numbers.

pub(in crate::parse) struct ParseErr {
    phrase: &'static str,
    at_eof: bool,
    line: usize,
    column: usize,
}

impl ParseErr {
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
    pub(in crate::parse) src: &'a str,
    pub(in crate::parse) index: usize,
    /// jq counts the line from 1.
    pub(in crate::parse) line: usize,
    /// jq counts bytes, starts at 0, and returns to 0 after a newline.
    column: usize,
    /// Nesting of arrays and objects. jq rejects the 257th level.
    pub(in crate::parse) depth: usize,
}

impl<'a> Parser<'a> {
    pub(in crate::parse) fn new(src: &'a str) -> Self {
        Self {
            src,
            index: 0,
            line: 1,
            column: 0,
            depth: 0,
        }
    }

    pub(in crate::parse) fn eof(&self) -> bool {
        self.index >= self.src.len()
    }

    pub(in crate::parse) fn peek(&self) -> Option<char> {
        self.src[self.index..].chars().next()
    }

    pub(in crate::parse) fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.index += c.len_utf8();
        if c == '\n' {
            self.line += 1;
            self.column = 0;
        } else {
            self.column += c.len_utf8();
        }
        Some(c)
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

fn is_ws(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r')
}

pub(in crate::parse) fn is_delimiter(c: char) -> bool {
    is_ws(c) || matches!(c, '"' | '[' | ']' | '{' | '}' | ',' | ':')
}
