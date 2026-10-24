//! The Mu lexer (spec §2).
//!
//! Lexing runs in two passes:
//!
//! 1. [`RawLexer`] turns text into tokens. Every `\n` outside a comment becomes a
//!    [`TokenKind::Newline`]; sigils fuse with names (`#x`, `x#`, `#x#`, `*p`);
//!    literals are validated but numbers stay as text.
//! 2. [`layout::apply`] decides which newlines terminate statements (rules
//!    N1–N8) and enforces the same-line rule for `else` and `until`.
//!
//! The lexer never stops at the first error: it reports a diagnostic, recovers,
//! and keeps going so one run shows every lexical problem in the file.

mod layout;
pub mod token;

#[cfg(test)]
mod tests;

pub use token::{ArithOp, Keyword, Modifier, Token, TokenKind};

use crate::diag::Diagnostic;
use crate::source::{FileId, Span};

/// Lexes one file into tokens ending in [`TokenKind::Eof`].
pub fn lex(file: FileId, src: &str, diags: &mut Vec<Diagnostic>) -> Vec<Token> {
    let raw = RawLexer::new(file, src, diags).run();
    layout::apply(raw, diags)
}

const VALID_ESCAPES: &str = r#"valid escapes are \n \t \\ \' \" \0 \a \b \f \r \v"#;

fn is_ident_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_'
}

fn is_ident_continue(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// `q16` in `q16.16`: a `q` followed only by digits.
fn is_fixed_point_head(word: &str) -> bool {
    word.len() > 1 && word.starts_with('q') && word[1..].bytes().all(|b| b.is_ascii_digit())
}

struct RawLexer<'a> {
    file: FileId,
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
    tokens: Vec<Token>,
    diags: &'a mut Vec<Diagnostic>,
    /// Whether the previous token can end an expression (the `*` fusion rule).
    prev_ends_expr: bool,
}

impl<'a> RawLexer<'a> {
    fn new(file: FileId, src: &'a str, diags: &'a mut Vec<Diagnostic>) -> RawLexer<'a> {
        RawLexer {
            file,
            src,
            bytes: src.as_bytes(),
            pos: 0,
            tokens: Vec::new(),
            diags,
            prev_ends_expr: false,
        }
    }

    // ---- helpers ----

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn peek_at(&self, n: usize) -> Option<u8> {
        self.bytes.get(self.pos + n).copied()
    }

    fn span(&self, lo: usize, hi: usize) -> Span {
        Span::new(self.file, lo, hi)
    }

    fn push(&mut self, kind: TokenKind, lo: usize) {
        self.prev_ends_expr = kind.can_end_expr();
        let span = self.span(lo, self.pos);
        self.tokens.push(Token { kind, span });
    }

    fn report(&mut self, diag: Diagnostic) {
        self.diags.push(diag);
    }

    fn read_ident(&mut self) -> &'a str {
        let src = self.src;
        let lo = self.pos;
        while matches!(self.peek(), Some(c) if is_ident_continue(c)) {
            self.pos += 1;
        }
        &src[lo..self.pos]
    }

    /// Pushes `two` if the next byte is `second`, otherwise `one`. The first
    /// byte has not been consumed yet.
    fn op2(&mut self, lo: usize, second: u8, two: TokenKind, one: TokenKind) {
        self.pos += 1;
        if self.peek() == Some(second) {
            self.pos += 1;
            self.push(two, lo);
        } else {
            self.push(one, lo);
        }
    }

    // ---- main loop ----

    fn run(mut self) -> Vec<Token> {
        use TokenKind as T;
        loop {
            self.skip_trivia();
            let lo = self.pos;
            let Some(c) = self.peek() else {
                self.push(T::Eof, lo);
                return self.tokens;
            };
            match c {
                b'\n' => {
                    self.pos += 1;
                    self.push(T::Newline, lo);
                }
                b'#' => self.lex_hash(),
                b'*' => self.lex_star(),
                b'0'..=b'9' => self.lex_number(),
                c if is_ident_start(c) => self.lex_word(),
                b'\'' => self.lex_ascii(),
                b'"' => self.lex_string(),

                b'+' => {
                    self.pos += 1;
                    self.finish_arith(ArithOp::Add, lo);
                }
                b'-' => {
                    self.pos += 1;
                    if self.peek() == Some(b'>') {
                        self.pos += 1;
                        self.push(T::Arrow, lo);
                    } else {
                        self.finish_arith(ArithOp::Sub, lo);
                    }
                }
                b'/' => {
                    self.pos += 1;
                    self.finish_arith(ArithOp::Div, lo);
                }
                b'%' => {
                    self.pos += 1;
                    self.finish_arith(ArithOp::Rem, lo);
                }

                b':' => self.op2(lo, b'=', T::ColonEq, T::Colon),
                b'!' => self.op2(lo, b'=', T::NotEq, T::Bang),
                b'^' => self.op2(lo, b'=', T::CaretEq, T::Caret),
                b'.' => self.op2(lo, b'.', T::DotDot, T::Dot),
                b'=' => self.lex_equals(),
                b'<' => self.lex_angle(b'<', T::Lt, T::LtEq, T::Shl, T::ShlEq),
                b'>' => self.lex_angle(b'>', T::Gt, T::GtEq, T::Shr, T::ShrEq),
                b'&' => self.lex_doubled(b'&', T::Amp, T::AndAnd, T::AmpEq),
                b'|' => self.lex_doubled(b'|', T::Pipe, T::OrOr, T::PipeEq),

                b'~' => self.single(T::Tilde),
                b'(' => self.single(T::LParen),
                b')' => self.single(T::RParen),
                b'[' => self.single(T::LBracket),
                b']' => self.single(T::RBracket),
                b'{' => self.single(T::LBrace),
                b'}' => self.single(T::RBrace),
                b',' => self.single(T::Comma),
                b';' => self.single(T::Semi),
                b'\\' => self.single(T::Backslash),

                _ => self.unexpected_char(),
            }
        }
    }

    fn single(&mut self, kind: TokenKind) {
        let lo = self.pos;
        self.pos += 1;
        self.push(kind, lo);
    }

    /// Whitespace and comments. Newlines are tokens, not trivia (§2.1).
    fn skip_trivia(&mut self) {
        loop {
            match self.peek() {
                Some(b' ' | b'\t' | b'\r' | 0x0B | 0x0C) => self.pos += 1,
                Some(b'/') if self.peek_at(1) == Some(b'*') => self.skip_comment(),
                _ => return,
            }
        }
    }

    fn skip_comment(&mut self) {
        let lo = self.pos;
        self.pos += 2;
        match self.src[self.pos..].find("*/") {
            Some(offset) => self.pos += offset + 2,
            None => {
                self.pos = self.src.len();
                let d = Diagnostic::error(self.span(lo, lo + 2), "unterminated comment")
                    .label("comment starts here")
                    .note("comments are `/* ... */` and do not nest");
                self.report(d);
            }
        }
    }

    fn unexpected_char(&mut self) {
        let lo = self.pos;
        let ch = self.src[lo..].chars().next().unwrap_or('\u{FFFD}');
        self.pos += ch.len_utf8();
        let message = if ch.is_ascii() {
            format!("unexpected character `{}`", ch.escape_debug())
        } else {
            format!(
                "unexpected character `{ch}` (U+{:04X}); names and operators are ASCII",
                ch as u32
            )
        };
        let d = Diagnostic::error(self.span(lo, self.pos), message);
        self.report(d);
    }

    // ---- names and sigils (§2.2) ----

    fn lex_word(&mut self) {
        let lo = self.pos;
        let mut word = self.read_ident().to_string();

        // Fixed-point type names such as `q16.16` contain a dot; keep them whole.
        if is_fixed_point_head(&word)
            && self.peek() == Some(b'.')
            && matches!(self.peek_at(1), Some(b'0'..=b'9'))
        {
            self.pos += 1;
            let frac_lo = self.pos;
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
            word.push('.');
            word.push_str(&self.src[frac_lo..self.pos]);
        }

        let mutating = self.peek() == Some(b'#');
        if mutating {
            self.pos += 1;
        }

        if word == "_" {
            if mutating {
                let d = Diagnostic::error(self.span(lo, self.pos), "`_` cannot carry a `#`")
                    .note("`_` is the wildcard, not a name");
                self.report(d);
            }
            self.push(TokenKind::Wildcard, lo);
            return;
        }

        if let Some(kw) = Keyword::lookup(&word) {
            if mutating {
                let d = Diagnostic::error(
                    self.span(lo, self.pos),
                    format!("`{word}` is a keyword and cannot be a function name"),
                );
                self.report(d);
            }
            self.push(TokenKind::Keyword(kw), lo);
            return;
        }

        let kind = if mutating {
            TokenKind::MutFnIdent(word)
        } else {
            TokenKind::Ident(word)
        };
        self.push(kind, lo);
    }

    fn bare_hash(&mut self, lo: usize) {
        let d = Diagnostic::error(self.span(lo, lo + 1), "a bare `#` is not valid").note(
            "`#` is part of a name: `#count` is mutable storage, `bump#` is a mutating function",
        );
        self.report(d);
    }

    fn lex_hash(&mut self) {
        let lo = self.pos;
        self.pos += 1;
        match self.peek() {
            Some(c) if is_ident_start(c) => {}
            Some(b' ' | b'\t') => {
                // `# name`: report the space, then recover as if it were `#name`.
                let ws_lo = self.pos;
                while matches!(self.peek(), Some(b' ' | b'\t')) {
                    self.pos += 1;
                }
                if !matches!(self.peek(), Some(c) if is_ident_start(c)) {
                    self.bare_hash(lo);
                    return;
                }
                let d =
                    Diagnostic::error(self.span(ws_lo, self.pos), "no space is allowed after `#`")
                        .label("remove this space")
                        .note("the sigil is part of the name: write `#name`");
                self.report(d);
            }
            _ => {
                self.bare_hash(lo);
                return;
            }
        }

        let name_lo = self.pos;
        let word = self.read_ident().to_string();
        let mutating_fn = self.peek() == Some(b'#');
        if mutating_fn {
            self.pos += 1;
        }

        if word == "_" {
            let d = Diagnostic::error(self.span(lo, self.pos), "`_` cannot be a mutable name")
                .note("`_` is the wildcard, not a name");
            self.report(d);
            self.push(TokenKind::Wildcard, lo);
            return;
        }
        if Keyword::lookup(&word).is_some() {
            let d = Diagnostic::error(
                self.span(name_lo, name_lo + word.len()),
                format!("`{word}` is a keyword and cannot be a name"),
            );
            self.report(d);
        }

        let kind = if mutating_fn {
            TokenKind::MutMutFnIdent(word)
        } else {
            TokenKind::MutIdent(word)
        };
        self.push(kind, lo);
    }

    /// `*` is a pointer sigil when glued to a name and the previous token cannot
    /// end an expression; otherwise it is multiplication (§2.2).
    fn lex_star(&mut self) {
        let lo = self.pos;
        self.pos += 1;
        if !self.prev_ends_expr && matches!(self.peek(), Some(c) if is_ident_start(c)) {
            let word = self.read_ident().to_string();
            if word == "_" || Keyword::lookup(&word).is_some() {
                let d = Diagnostic::error(
                    self.span(lo, self.pos),
                    format!("`*{word}` is not a valid pointer name"),
                );
                self.report(d);
            }
            self.push(TokenKind::PtrIdent(word), lo);
            return;
        }
        self.finish_arith(ArithOp::Mul, lo);
    }

    // ---- operators ----

    /// After `+ - * / %`: an optional `% | ^` modifier, then an optional `=`.
    /// Longest match is safe because two binary operators are never adjacent.
    fn finish_arith(&mut self, op: ArithOp, lo: usize) {
        let modifier = match self.peek() {
            Some(b'%') => Modifier::Wrap,
            Some(b'|') if self.peek_at(1) != Some(b'|') => Modifier::Saturate,
            Some(b'^') => Modifier::Trap,
            _ => Modifier::Default,
        };
        if modifier != Modifier::Default {
            self.pos += 1;
        }
        let assign = self.peek() == Some(b'=') && !matches!(self.peek_at(1), Some(b'=' | b'>'));
        if assign {
            self.pos += 1;
            self.push(TokenKind::ArithAssign(op, modifier), lo);
        } else {
            self.push(TokenKind::Arith(op, modifier), lo);
        }
    }

    fn lex_equals(&mut self) {
        let lo = self.pos;
        self.pos += 1;
        match self.peek() {
            Some(b'=') => {
                self.pos += 1;
                self.push(TokenKind::EqEq, lo);
            }
            Some(b'>') => {
                self.pos += 1;
                self.push(TokenKind::FatArrow, lo);
            }
            _ => {
                let d =
                    Diagnostic::error(self.span(lo, self.pos), "`=` on its own is not an operator")
                        .note("use `:=` to assign, `==` to compare, or `=>` to return");
                self.report(d);
                self.push(TokenKind::ColonEq, lo);
            }
        }
    }

    /// `<`, `<=`, `<<`, `<<=` and the `>` family.
    fn lex_angle(
        &mut self,
        c: u8,
        one: TokenKind,
        eq: TokenKind,
        two: TokenKind,
        two_eq: TokenKind,
    ) {
        let lo = self.pos;
        self.pos += 1;
        if self.peek() == Some(c) {
            self.pos += 1;
            if self.peek() == Some(b'=') {
                self.pos += 1;
                self.push(two_eq, lo);
            } else {
                self.push(two, lo);
            }
        } else if self.peek() == Some(b'=') {
            self.pos += 1;
            self.push(eq, lo);
        } else {
            self.push(one, lo);
        }
    }

    /// `&`, `&&`, `&=` and the `|` family.
    fn lex_doubled(&mut self, c: u8, one: TokenKind, two: TokenKind, eq: TokenKind) {
        let lo = self.pos;
        self.pos += 1;
        match self.peek() {
            Some(n) if n == c => {
                self.pos += 1;
                self.push(two, lo);
            }
            Some(b'=') => {
                self.pos += 1;
                self.push(eq, lo);
            }
            _ => self.push(one, lo),
        }
    }

    // ---- numeric literals (§2.3) ----

    fn eat_decimal_group(&mut self) -> usize {
        let lo = self.pos;
        while matches!(self.peek(), Some(b'0'..=b'9' | b'_')) {
            self.pos += 1;
        }
        lo
    }

    /// Strips separators from a decimal digit group, reporting a trailing `_`.
    fn clean_group(&mut self, lo: usize, hi: usize) -> String {
        let group = &self.src[lo..hi];
        if group.ends_with('_') {
            let d = Diagnostic::error(self.span(hi - 1, hi), "a digit group cannot end with `_`")
                .note("`_` separates digits: `1_000`, never leading or trailing");
            self.report(d);
        }
        group.chars().filter(|&c| c != '_').collect()
    }

    fn lex_number(&mut self) {
        let lo = self.pos;
        if self.peek() == Some(b'0') && matches!(self.peek_at(1), Some(b'x' | b'b')) {
            self.lex_radix_number(lo);
            return;
        }

        let int_lo = self.eat_decimal_group();
        let mut text = self.clean_group(int_lo, self.pos);
        let mut is_float = false;

        // A fraction needs a digit after the dot, so `1..5` and `t[0].x` still lex.
        if self.peek() == Some(b'.') && matches!(self.peek_at(1), Some(b'0'..=b'9')) {
            is_float = true;
            self.pos += 1;
            let frac_lo = self.eat_decimal_group();
            text.push('.');
            let frac = self.clean_group(frac_lo, self.pos);
            text.push_str(&frac);
        }

        // An exponent needs at least one digit, optionally after a sign.
        if matches!(self.peek(), Some(b'e' | b'E')) {
            let signed = matches!(self.peek_at(1), Some(b'+' | b'-'));
            let digit_at = if signed { 2 } else { 1 };
            if matches!(self.peek_at(digit_at), Some(b'0'..=b'9')) {
                is_float = true;
                text.push('e');
                if signed {
                    text.push(self.bytes[self.pos + 1] as char);
                }
                self.pos += digit_at;
                let exp_lo = self.pos;
                while matches!(self.peek(), Some(b'0'..=b'9')) {
                    self.pos += 1;
                }
                text.push_str(&self.src[exp_lo..self.pos]);
            }
        }

        self.reject_suffix();
        let kind = if is_float {
            TokenKind::Float(text)
        } else {
            TokenKind::Int {
                digits: text,
                radix: 10,
            }
        };
        self.push(kind, lo);
    }

    fn lex_radix_number(&mut self, lo: usize) {
        let (radix, prefix) = if self.peek_at(1) == Some(b'x') {
            (16, "0x")
        } else {
            (2, "0b")
        };
        self.pos += 2;
        let body_lo = self.pos;
        while matches!(self.peek(), Some(c) if is_ident_continue(c)) {
            self.pos += 1;
        }
        let body = &self.src[body_lo..self.pos];

        if body.is_empty() {
            let d = Diagnostic::error(
                self.span(lo, self.pos),
                format!("`{prefix}` must be followed by digits"),
            );
            self.report(d);
        } else if body.starts_with('_') {
            let d = Diagnostic::error(
                self.span(body_lo, body_lo + 1),
                format!("`_` cannot follow the radix prefix `{prefix}`"),
            );
            self.report(d);
        } else if body.ends_with('_') {
            let d = Diagnostic::error(
                self.span(self.pos - 1, self.pos),
                "a numeric literal cannot end with `_`",
            );
            self.report(d);
        }
        if let Some((offset, bad)) = body
            .char_indices()
            .find(|&(_, c)| c != '_' && !c.is_digit(radix))
        {
            let at = body_lo + offset;
            let d = Diagnostic::error(
                self.span(at, at + 1),
                format!("`{bad}` is not a valid digit in a base-{radix} literal"),
            );
            self.report(d);
        }

        let digits: String = body.chars().filter(|&c| c != '_').collect();
        let digits = if digits.is_empty() {
            "0".to_string()
        } else {
            digits
        };
        self.push(TokenKind::Int { digits, radix }, lo);
    }

    /// Letters glued to a number (`12abc`, `1.5f`) are an error: Mu has no
    /// literal suffixes; a cast gives a literal its type.
    fn reject_suffix(&mut self) {
        if !matches!(self.peek(), Some(c) if is_ident_continue(c)) {
            return;
        }
        let lo = self.pos;
        while matches!(self.peek(), Some(c) if is_ident_continue(c)) {
            self.pos += 1;
        }
        let suffix = &self.src[lo..self.pos];
        let d = Diagnostic::error(
            self.span(lo, self.pos),
            format!("invalid suffix `{suffix}` on a numeric literal"),
        )
        .note("literals take their type from context or a cast, such as `(i64) 5`");
        self.report(d);
    }

    // ---- text literals (§2.4) ----

    /// One plain source character inside a literal. Non-ASCII is rejected
    /// until the spec decides what it means (T2).
    fn lex_plain_char(&mut self) -> u8 {
        let lo = self.pos;
        let ch = self.src[lo..].chars().next().unwrap_or('\u{FFFD}');
        self.pos += ch.len_utf8();
        if ch.is_ascii() {
            ch as u8
        } else {
            let d = Diagnostic::error(
                self.span(lo, self.pos),
                format!("non-ASCII character `{ch}` in a literal"),
            )
            .note(
                "`ascii` holds 0 to 127; non-ASCII text in literals is not yet specified (spec T2)",
            );
            self.report(d);
            b'?'
        }
    }

    /// An escape sequence; the cursor is on the backslash.
    fn lex_escape(&mut self) -> u8 {
        let lo = self.pos;
        self.pos += 1;
        let value = match self.peek() {
            Some(b'n') => b'\n',
            Some(b't') => b'\t',
            Some(b'r') => b'\r',
            Some(b'0') => 0,
            Some(b'a') => 7,
            Some(b'b') => 8,
            Some(b'v') => 11,
            Some(b'f') => 12,
            Some(b'\\') => b'\\',
            Some(b'\'') => b'\'',
            Some(b'"') => b'"',
            None | Some(b'\n') => {
                let d = Diagnostic::error(self.span(lo, self.pos), "incomplete escape sequence");
                self.report(d);
                return b'?';
            }
            Some(b'x') => {
                self.pos += 1;
                let d = Diagnostic::error(
                    self.span(lo, self.pos),
                    "byte escapes `\\xNN` are not yet specified",
                )
                .note("see spec T2; for now use one of the named escapes")
                .note(VALID_ESCAPES);
                self.report(d);
                return b'?';
            }
            Some(_) => {
                let ch = self.src[self.pos..].chars().next().unwrap_or('?');
                self.pos += ch.len_utf8();
                let d =
                    Diagnostic::error(self.span(lo, self.pos), format!("unknown escape `\\{ch}`"))
                        .note(VALID_ESCAPES);
                self.report(d);
                return b'?';
            }
        };
        self.pos += 1;
        value
    }

    fn lex_ascii(&mut self) {
        let lo = self.pos;
        self.pos += 1;
        let value = match self.peek() {
            None | Some(b'\n') => {
                let d = Diagnostic::error(self.span(lo, self.pos), "unterminated ascii literal");
                self.report(d);
                self.push(TokenKind::Ascii(0), lo);
                return;
            }
            Some(b'\'') => {
                self.pos += 1;
                let d = Diagnostic::error(self.span(lo, self.pos), "empty ascii literal")
                    .note("an ascii literal holds exactly one character, such as 'x'");
                self.report(d);
                self.push(TokenKind::Ascii(0), lo);
                return;
            }
            Some(b'\\') => self.lex_escape(),
            Some(_) => self.lex_plain_char(),
        };

        if self.peek() == Some(b'\'') {
            self.pos += 1;
        } else {
            let rest = &self.src[self.pos..];
            let line_end = rest.find('\n').unwrap_or(rest.len());
            match rest[..line_end].find('\'') {
                Some(close) => {
                    self.pos += close + 1;
                    let d = Diagnostic::error(
                        self.span(lo, self.pos),
                        "ascii literal holds more than one character",
                    )
                    .note("use a string literal \"...\" for text");
                    self.report(d);
                }
                None => {
                    let d =
                        Diagnostic::error(self.span(lo, self.pos), "unterminated ascii literal")
                            .label("missing closing `'`");
                    self.report(d);
                }
            }
        }
        self.push(TokenKind::Ascii(value), lo);
    }

    fn lex_string(&mut self) {
        let lo = self.pos;
        self.pos += 1;
        let mut bytes = Vec::new();
        loop {
            match self.peek() {
                None | Some(b'\n') => {
                    let d =
                        Diagnostic::error(self.span(lo, self.pos), "unterminated string literal")
                            .note("string literals cannot span lines");
                    self.report(d);
                    break;
                }
                Some(b'"') => {
                    self.pos += 1;
                    break;
                }
                Some(b'\\') => {
                    let b = self.lex_escape();
                    bytes.push(b);
                }
                Some(_) => {
                    let b = self.lex_plain_char();
                    bytes.push(b);
                }
            }
        }
        self.push(TokenKind::Str(bytes), lo);
    }
}
