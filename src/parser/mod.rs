//! The Mu parser.
//!
//! Hand-written recursive descent for items and statements, with precedence
//! climbing (Pratt) for binary operators following the table in spec §7.1.
//!
//! Before parsing, [`collect_type_names`] scans the token streams of every
//! file in the project for user type names. The parser needs them to decide
//! whether `(T) x` is a cast or a parenthesized expression (§6.7), and the scan
//! is what makes declaration order irrelevant.
//!
//! On an error the parser reports a diagnostic, skips to the end of the
//! statement (or struct member, or item), and continues, so one run reports
//! every independent syntax error.

#[cfg(test)]
mod tests;

use crate::ast::*;
use crate::diag::Diagnostic;
use crate::lexer::{ArithOp, Keyword as Kw, Modifier, Token, TokenKind as T};
use crate::source::Span;
use std::collections::HashSet;

/// Marker for "a diagnostic has already been reported; recover".
#[derive(Debug)]
pub struct Reported;

type PResult<T> = Result<T, Reported>;

// ============================================================ type-name pre-pass

/// Collects the user type names declared at the top level of one file: structs
/// (including union variants such as `Shape.Circle`), words, unions,
/// interfaces and type aliases. Call it for every file before parsing any.
pub fn collect_type_names(tokens: &[Token], out: &mut HashSet<String>) {
    let mut depth = 0i32;
    for (i, token) in tokens.iter().enumerate() {
        match &token.kind {
            T::LBrace => depth += 1,
            T::RBrace => depth -= 1,
            T::Keyword(kw @ (Kw::Struct | Kw::Word | Kw::Union | Kw::Interface | Kw::Type))
                if depth == 0 =>
            {
                let mut j = i + 1;
                let mut name = String::new();
                while let Some(T::Ident(segment)) = tokens.get(j).map(|t| &t.kind) {
                    if !name.is_empty() {
                        name.push('.');
                    }
                    name.push_str(segment);
                    j += 1;
                    if tokens.get(j).map(|t| &t.kind) == Some(&T::Dot) {
                        j += 1;
                    } else {
                        break;
                    }
                }
                if name.is_empty() {
                    continue;
                }
                let next = tokens.get(j).map(|t| &t.kind);
                if *kw == Kw::Type && next != Some(&T::Colon) {
                    continue;
                }
                if *kw == Kw::Union && next == Some(&T::LParen) {
                    // union Shape(Circle, Square): also record Shape.Circle, ...
                    for t in &tokens[j + 1..] {
                        match &t.kind {
                            T::Ident(variant) => {
                                out.insert(format!("{name}.{variant}"));
                            }
                            T::Comma => {}
                            _ => break,
                        }
                    }
                }
                out.insert(name);
            }
            _ => {}
        }
    }
}

/// Primitive type names (§4.1): `i8`..`i128` and `u8`..`u128` in steps of 8,
/// `f32`, `f64`, `bool`, `ascii`, and fixed point `qN` / `qM.N`.
pub fn is_primitive_type_name(name: &str) -> bool {
    match name {
        "f32" | "f64" | "bool" | "ascii" => return true,
        _ => {}
    }
    let bytes = name.as_bytes();
    match bytes.first() {
        Some(b'i' | b'u') => match name[1..].parse::<u32>() {
            Ok(width) => {
                width % 8 == 0 && (8..=128).contains(&width) && !name[1..].starts_with('0')
            }
            Err(_) => false,
        },
        Some(b'q') => {
            let rest = &name[1..];
            let mut parts = rest.splitn(2, '.');
            let all_digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
            match (parts.next(), parts.next()) {
                (Some(a), None) => all_digits(a),
                (Some(a), Some(b)) => all_digits(a) && all_digits(b),
                _ => false,
            }
        }
        _ => false,
    }
}

/// Parses one file. `type_names` comes from [`collect_type_names`] over the
/// whole project.
pub fn parse_file(
    tokens: &[Token],
    type_names: &HashSet<String>,
    diags: &mut Vec<Diagnostic>,
) -> File {
    Parser::new(tokens, type_names, diags).file()
}

struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
    type_names: &'a HashSet<String>,
    diags: &'a mut Vec<Diagnostic>,
}

impl<'a> Parser<'a> {
    fn new(
        tokens: &'a [Token],
        type_names: &'a HashSet<String>,
        diags: &'a mut Vec<Diagnostic>,
    ) -> Self {
        assert!(
            matches!(tokens.last(), Some(t) if t.kind == T::Eof),
            "token stream must end with Eof"
        );
        Parser {
            tokens,
            pos: 0,
            type_names,
            diags,
        }
    }

    // ======================================================== token helpers

    fn peek(&self) -> &'a Token {
        &self.tokens[self.pos]
    }

    fn kind(&self) -> &'a T {
        &self.tokens[self.pos].kind
    }

    fn nth(&self, n: usize) -> &'a T {
        let i = (self.pos + n).min(self.tokens.len() - 1);
        &self.tokens[i].kind
    }

    fn span(&self) -> Span {
        self.peek().span
    }

    fn prev_span(&self) -> Span {
        self.tokens[self.pos.saturating_sub(1)].span
    }

    fn at(&self, kind: &T) -> bool {
        self.kind() == kind
    }

    fn at_kw(&self, kw: Kw) -> bool {
        *self.kind() == T::Keyword(kw)
    }

    fn at_terminator(&self) -> bool {
        matches!(self.kind(), T::Newline | T::Semi)
    }

    fn bump(&mut self) -> &'a Token {
        let token = &self.tokens[self.pos];
        if token.kind != T::Eof {
            self.pos += 1;
        }
        token
    }

    fn eat(&mut self, kind: &T) -> bool {
        if self.at(kind) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn eat_kw(&mut self, kw: Kw) -> bool {
        self.eat(&T::Keyword(kw))
    }

    fn skip_terminators(&mut self) {
        while self.at_terminator() {
            self.bump();
        }
    }

    /// How the current token reads in a message: "`x`", "a newline".
    fn describe(&self) -> String {
        match self.kind() {
            T::Newline => "a newline".into(),
            T::Eof => "the end of the file".into(),
            kind => format!("`{kind}`"),
        }
    }

    fn report(&mut self, diag: Diagnostic) {
        self.diags.push(diag);
    }

    fn error(&mut self, span: Span, message: impl Into<String>) {
        self.report(Diagnostic::error(span, message));
    }

    fn error_expected(&mut self, what: &str) -> Reported {
        let message = format!("expected {what}, found {}", self.describe());
        let span = self.span();
        self.error(span, message);
        Reported
    }

    fn expect(&mut self, kind: &T, what: &str) -> PResult<Span> {
        if self.at(kind) {
            Ok(self.bump().span)
        } else {
            Err(self.error_expected(what))
        }
    }

    fn expect_kw(&mut self, kw: Kw, what: &str) -> PResult<Span> {
        self.expect(&T::Keyword(kw), what)
    }

    /// The end of a simple statement: a newline or `;`, or nothing before `}`
    /// (N7) or at the end of the file.
    fn end_stmt(&mut self, what: &str) -> PResult<()> {
        match self.kind() {
            T::Newline | T::Semi => {
                self.bump();
                Ok(())
            }
            T::RBrace | T::Eof => Ok(()),
            _ => Err(self.error_expected(&format!("a newline or `;` after {what}"))),
        }
    }

    /// A comma-separated list ending in `close`, with the opener already eaten.
    fn list<R>(
        &mut self,
        close: &T,
        trailing_ok: bool,
        what: &str,
        mut item: impl FnMut(&mut Self) -> PResult<R>,
    ) -> PResult<Vec<R>> {
        let mut out = Vec::new();
        if self.eat(close) {
            return Ok(out);
        }
        loop {
            out.push(item(self)?);
            if self.at(&T::Comma) {
                let comma = self.bump().span;
                if self.at(close) {
                    if !trailing_ok {
                        self.error(comma, format!("a trailing comma is not allowed in {what}"));
                    }
                    self.bump();
                    return Ok(out);
                }
                continue;
            }
            self.expect(close, &format!("`,` or `{close}` in {what}"))?;
            return Ok(out);
        }
    }

    /// Skips the rest of a statement after an error: up to and including the
    /// next newline or `;` at this nesting level, or up to (not including) the
    /// `}` that closes the enclosing block.
    fn sync_stmt(&mut self) {
        self.sync(|_| false);
    }

    /// Like `sync_stmt`, but also stops after a `,` (struct members).
    fn sync_member(&mut self) {
        self.sync(|k| *k == T::Comma);
    }

    fn sync(&mut self, also_stop_after: impl Fn(&T) -> bool) {
        let mut depth = 0usize;
        loop {
            match self.kind() {
                T::Eof => return,
                T::RBrace | T::RParen | T::RBracket if depth == 0 => {
                    if *self.kind() == T::RBrace {
                        return;
                    }
                    self.bump();
                }
                T::LBrace | T::LParen | T::LBracket => {
                    depth += 1;
                    self.bump();
                }
                T::RBrace | T::RParen | T::RBracket => {
                    depth -= 1;
                    self.bump();
                }
                k if depth == 0 && (matches!(k, T::Newline | T::Semi) || also_stop_after(k)) => {
                    self.bump();
                    return;
                }
                _ => {
                    self.bump();
                }
            }
        }
    }

    /// Skips to the next top-level line after an error in an item.
    fn sync_item(&mut self) {
        if self.at(&T::RBrace) {
            self.bump();
        }
        self.sync_stmt();
    }

    // ======================================================== names

    fn take_name(&mut self, what: &str) -> PResult<Name> {
        let token = self.peek();
        let (sigil, text) = match &token.kind {
            T::Ident(s) => (Sigil::None, s),
            T::MutIdent(s) => (Sigil::Mut, s),
            T::MutFnIdent(s) => (Sigil::MutFn, s),
            T::MutMutFnIdent(s) => (Sigil::MutMutFn, s),
            T::PtrIdent(s) => (Sigil::Ptr, s),
            _ => return Err(self.error_expected(what)),
        };
        self.bump();
        Ok(Name {
            sigil,
            text: text.clone(),
            span: token.span,
        })
    }

    /// A plain identifier with no sigil.
    fn ident(&mut self, what: &str) -> PResult<Name> {
        if matches!(self.kind(), T::Ident(_)) {
            self.take_name(what)
        } else {
            Err(self.error_expected(what))
        }
    }

    /// `a.b.c` with plain identifiers.
    fn path(&mut self, what: &str) -> PResult<Path> {
        let first = self.ident(what)?;
        let mut span = first.span;
        let mut segments = vec![first];
        while self.at(&T::Dot) && matches!(self.nth(1), T::Ident(_)) {
            self.bump();
            let seg = self.ident("a name after `.`")?;
            span = span.to(seg.span);
            segments.push(seg);
        }
        Ok(Path { segments, span })
    }

    fn path_list(&mut self, what: &str) -> PResult<Vec<Path>> {
        let mut out = vec![self.path(what)?];
        while self.eat(&T::Comma) {
            out.push(self.path(what)?);
        }
        Ok(out)
    }

    fn is_user_type(&self, path: &Path) -> bool {
        let texts: Vec<&str> = path.segments.iter().map(|s| s.text.as_str()).collect();
        (0..texts.len()).any(|k| self.type_names.contains(&texts[k..].join(".")))
    }

    // ======================================================== file and items

    fn file(&mut self) -> File {
        let mut imports = Vec::new();
        let mut items = Vec::new();
        loop {
            self.skip_terminators();
            match self.kind() {
                T::Eof => break,
                T::Keyword(Kw::Import) => {
                    let start = self.span();
                    match self.import() {
                        Ok(import) => {
                            if !items.is_empty() {
                                self.report(
                                    Diagnostic::error(
                                        start,
                                        "imports must come before all definitions",
                                    )
                                    .note("imports are file-wide and come first (§3.1)"),
                                );
                            }
                            imports.push(import);
                        }
                        Err(Reported) => self.sync_item(),
                    }
                }
                _ => match self.item() {
                    Ok(item) => items.push(item),
                    Err(Reported) => self.sync_item(),
                },
            }
        }
        File { imports, items }
    }

    fn import(&mut self) -> PResult<Import> {
        let lo = self.bump().span;
        let path = self.path("a module name after `import`")?;
        self.end_stmt("an import")?;
        Ok(Import {
            span: lo.to(path.span),
            path,
        })
    }

    /// `share prerun cycle`, in that order.
    fn modifiers(&mut self) -> (Option<Span>, Option<Span>, Option<Span>) {
        let (mut share, mut prerun, mut cycle) = (None, None, None);
        let mut last_rank = 0;
        loop {
            let (slot, rank) = match self.kind() {
                T::Keyword(Kw::Share) => (&mut share, 1),
                T::Keyword(Kw::Prerun) => (&mut prerun, 2),
                T::Keyword(Kw::Cycle) => (&mut cycle, 3),
                _ => break,
            };
            let span = self.tokens[self.pos].span;
            let duplicate = slot.is_some();
            *slot = Some(span);
            self.bump();
            if duplicate {
                self.error(span, "this modifier is repeated");
            } else if rank < last_rank {
                self.report(
                    Diagnostic::error(
                        span,
                        "modifiers must appear in the order `share prerun cycle`",
                    )
                    .note("see §9.1"),
                );
            }
            last_rank = last_rank.max(rank);
        }
        (share, prerun, cycle)
    }

    fn reject_fn_modifiers(&mut self, prerun: Option<Span>, cycle: Option<Span>) {
        if let Some(span) = prerun {
            self.error(span, "`prerun` applies only to functions");
        }
        if let Some(span) = cycle {
            self.error(span, "`cycle` applies only to functions");
        }
    }

    fn at_fn_def(&self) -> bool {
        matches!(self.kind(), T::Ident(_) | T::MutFnIdent(_)) && *self.nth(1) == T::Keyword(Kw::F)
    }

    fn item(&mut self) -> PResult<Item> {
        let lo = self.span();
        let (share, prerun, cycle) = self.modifiers();
        let kind = match self.kind() {
            T::Keyword(Kw::As) => {
                self.reject_fn_modifiers(prerun, cycle);
                let bindings = self.bindings()?;
                self.end_stmt("a binding")?;
                ItemKind::Bindings(bindings)
            }
            T::Keyword(Kw::Struct) => {
                self.reject_fn_modifiers(prerun, cycle);
                ItemKind::Struct(self.struct_def()?)
            }
            T::Keyword(Kw::Word) => {
                self.reject_fn_modifiers(prerun, cycle);
                ItemKind::Word(self.word_def()?)
            }
            T::Keyword(Kw::Union) => {
                self.reject_fn_modifiers(prerun, cycle);
                ItemKind::Union(self.union_def()?)
            }
            T::Keyword(Kw::Interface) => {
                self.reject_fn_modifiers(prerun, cycle);
                ItemKind::Interface(self.interface_def()?)
            }
            T::Keyword(Kw::Type) => {
                self.reject_fn_modifiers(prerun, cycle);
                ItemKind::Alias(self.type_alias()?)
            }
            _ if self.at_fn_def() => ItemKind::Fn(self.fn_def(prerun.is_some(), cycle.is_some())?),
            _ => {
                let span = self.span();
                let looks_like_statement =
                    !matches!(self.kind(), T::RBrace | T::RParen | T::RBracket);
                if looks_like_statement {
                    self.report(
                        Diagnostic::error(span, "statements are only allowed inside functions")
                            .note("execution starts at `main`; nothing runs before it (§3.1)"),
                    );
                    return Err(Reported);
                }
                return Err(self.error_expected("a definition"));
            }
        };
        Ok(Item {
            share: share.is_some(),
            kind,
            span: lo.to(self.prev_span()),
        })
    }

    /// `name f(params)[captures] -> type { body }`; modifiers already eaten.
    fn fn_def(&mut self, prerun: bool, cycle: bool) -> PResult<FnDef> {
        let lo = self.span();
        let name = match self.kind() {
            T::Ident(_) | T::MutFnIdent(_) => self.take_name("a function name")?,
            _ => return Err(self.error_expected("a function name")),
        };
        self.check_value_name(&name);
        self.expect_kw(Kw::F, "`f` after the function name")?;
        self.expect(&T::LParen, "`(` to start the parameter list")?;
        let params = self.list(&T::RParen, false, "a parameter list", |p| p.param())?;
        let captures = if self.at(&T::LBracket) {
            Some(self.capture_group()?)
        } else {
            None
        };
        if !self.at(&T::Arrow) {
            let span = self.span();
            self.report(
                Diagnostic::error(span, format!("missing return type for `{name}`"))
                    .note("the return type is mandatory (§9.1); use `-> effect` when there is nothing to return"),
            );
            return Err(Reported);
        }
        self.bump();
        let ret = self.ty()?;
        let body = self.block()?;
        Ok(FnDef {
            prerun,
            cycle,
            name,
            params,
            captures,
            ret,
            span: lo.to(body.span),
            body,
        })
    }

    fn param(&mut self) -> PResult<Param> {
        let lo = self.span();
        let mode = if self.eat_kw(Kw::Ref) {
            ParamMode::Ref
        } else if self.eat_kw(Kw::Copy) {
            ParamMode::Copy
        } else {
            ParamMode::Value
        };
        let (name, shape) = match self.kind() {
            T::PtrIdent(_) => {
                let name = self.take_name("a parameter")?;
                (name, self.ptr_shape()?)
            }
            T::Ident(_) | T::MutIdent(_) | T::MutFnIdent(_) | T::MutMutFnIdent(_) => {
                let name = self.take_name("a parameter")?;
                self.check_value_name(&name);
                if self.at(&T::LBracket) {
                    (name, self.array_shape()?)
                } else {
                    self.expect(&T::Colon, "`:` and a type after the parameter name")?;
                    (name, Shape::Typed(self.ty()?))
                }
            }
            _ => return Err(self.error_expected("a parameter")),
        };
        Ok(Param {
            mode,
            name,
            shape,
            span: lo.to(self.prev_span()),
        })
    }

    /// Whether the bracket just entered holds a comma at its own level, which
    /// distinguishes `*q[n, T]` (fat) from `*p[T]` (thin).
    fn bracket_has_top_level_comma(&self) -> bool {
        let mut depth = 0usize;
        for token in &self.tokens[self.pos..] {
            match token.kind {
                T::LParen | T::LBracket | T::LBrace => depth += 1,
                T::RParen | T::RBrace => {
                    if depth == 0 {
                        return false;
                    }
                    depth -= 1;
                }
                T::RBracket => {
                    if depth == 0 {
                        return false;
                    }
                    depth -= 1;
                }
                T::Comma if depth == 0 => return true,
                T::Newline | T::Semi | T::Eof if depth == 0 => return false,
                _ => {}
            }
        }
        false
    }

    /// `[T]` or `[n, T]` after a pointer name.
    fn ptr_shape(&mut self) -> PResult<Shape> {
        self.expect(&T::LBracket, "`[` and a pointee type after a pointer name")?;
        let shape = if self.bracket_has_top_level_comma() {
            let len = self.expr()?;
            self.expect(&T::Comma, "`,`")?;
            Shape::FatPtr {
                len,
                elem: self.ty()?,
            }
        } else {
            Shape::ThinPtr(self.ty()?)
        };
        self.expect(&T::RBracket, "`]`")?;
        Ok(shape)
    }

    /// `[n, T]` after an array name.
    fn array_shape(&mut self) -> PResult<Shape> {
        self.expect(&T::LBracket, "`[`")?;
        let len = self.expr()?;
        if !self.at(&T::Comma) {
            let span = self.span();
            self.report(
                Diagnostic::error(
                    span,
                    format!(
                        "expected `,` in an array declarator, found {}",
                        self.describe()
                    ),
                )
                .note("array declarators have two slots: `name[length, type]`"),
            );
            return Err(Reported);
        }
        self.bump();
        let elem = self.ty()?;
        self.expect(&T::RBracket, "`]` to close the array declarator")?;
        Ok(Shape::Array { len, elem })
    }

    /// `[a, #b, x.#f]`, `[]`, `[_]`.
    fn capture_group(&mut self) -> PResult<Vec<Capture>> {
        self.expect(&T::LBracket, "`[`")?;
        self.list(&T::RBracket, false, "a capture group", |p| p.capture())
    }

    fn capture(&mut self) -> PResult<Capture> {
        if self.at(&T::Wildcard) {
            return Ok(Capture::Wildcard(self.bump().span));
        }
        let first = match self.kind() {
            T::Ident(_) | T::MutIdent(_) => self.take_name("a captured name")?,
            _ => return Err(self.error_expected("a captured name")),
        };
        let mut span = first.span;
        let mut path = vec![first];
        while self.eat(&T::Dot) {
            let seg = match self.kind() {
                T::Ident(_) | T::MutIdent(_) => self.take_name("a field name")?,
                _ => return Err(self.error_expected("a field name after `.`")),
            };
            span = span.to(seg.span);
            path.push(seg);
        }
        Ok(Capture::Path(path, span))
    }

    fn struct_def(&mut self) -> PResult<StructDef> {
        self.bump(); // struct
        let name = self.path("a struct name")?;
        let implements = if self.eat(&T::Colon) {
            self.path_list("an interface name")?
        } else {
            Vec::new()
        };
        let open = self.expect(&T::LBrace, "`{` to start the struct body")?;

        let mut fields = Vec::new();
        let mut methods = Vec::new();
        // The last field, when it was not followed by a comma.
        let mut missing_comma: Option<Span> = None;
        loop {
            self.skip_terminators();
            match self.kind() {
                T::RBrace => {
                    self.bump();
                    break;
                }
                T::Eof => {
                    self.report(
                        Diagnostic::error(open, "unclosed struct body").label("opened here"),
                    );
                    return Err(Reported);
                }
                _ => {}
            }
            let is_method = self.at_kw(Kw::Cycle)
                || self.at_kw(Kw::Share)
                || self.at_kw(Kw::Prerun)
                || self.at_fn_def();
            if is_method {
                missing_comma = None;
                match self.method() {
                    Ok(method) => methods.push(method),
                    Err(Reported) => self.sync_member(),
                }
                continue;
            }
            if let Some(span) = missing_comma.take() {
                self.report(
                    Diagnostic::error(span, "expected `,` after this field").note(
                        "fields are separated by commas; a newline does not separate them (§10.1)",
                    ),
                );
            }
            match self.field() {
                Ok((field, had_comma)) => {
                    if !had_comma {
                        missing_comma = Some(field.span);
                    }
                    fields.push(field);
                }
                Err(Reported) => self.sync_member(),
            }
        }
        Ok(StructDef {
            name,
            implements,
            fields,
            methods,
        })
    }

    fn method(&mut self) -> PResult<FnDef> {
        let (share, prerun, cycle) = self.modifiers();
        if let Some(span) = share {
            self.error(span, "methods cannot be `share`; share the struct instead");
        }
        if let Some(span) = prerun {
            self.error(span, "methods cannot be `prerun`");
        }
        self.fn_def(false, cycle.is_some())
    }

    fn field(&mut self) -> PResult<(Field, bool)> {
        let lo = self.span();
        let name = match self.kind() {
            T::Ident(_) | T::MutIdent(_) => self.take_name("a field or method")?,
            _ => return Err(self.error_expected("a field or method")),
        };
        self.check_value_name(&name);
        let shape = if self.at(&T::LBracket) {
            self.array_shape()?
        } else {
            Shape::Inferred
        };
        if !self.at(&T::Colon) {
            let span = self.span();
            self.report(
                Diagnostic::error(
                    span,
                    format!("expected `:` and a default value for field `{name}`"),
                )
                .note("every field needs a constant default (§10.1)"),
            );
            return Err(Reported);
        }
        self.bump();
        let default = self.expr()?;
        let span = lo.to(default.span);
        let had_comma = self.eat(&T::Comma);
        Ok((
            Field {
                name,
                shape,
                default,
                span,
            },
            had_comma,
        ))
    }

    fn word_def(&mut self) -> PResult<WordDef> {
        self.bump(); // word
        let name = self.ident("a word name")?;
        let repr = if self.eat(&T::LParen) {
            let ty = self.ty()?;
            let start = if self.eat(&T::Comma) {
                if !matches!(self.kind(), T::Ident(s) if s == "start") {
                    return Err(self.error_expected("`start: value`"));
                }
                self.bump();
                self.expect(&T::Colon, "`:` after `start`")?;
                Some(self.expr()?)
            } else {
                None
            };
            self.expect(&T::RParen, "`)` to close the word representation")?;
            Some(WordRepr { ty, start })
        } else {
            None
        };

        let open = self.expect(&T::LBrace, "`{` to start the word's variants")?;
        let mut variants = Vec::new();
        loop {
            self.skip_terminators();
            if self.eat(&T::RBrace) {
                break;
            }
            let lo = self.span();
            let vname = self.ident("a variant name")?;
            self.check_value_name(&vname);
            let value = if self.eat(&T::Colon) {
                Some(self.expr()?)
            } else {
                None
            };
            variants.push(WordVariant {
                name: vname,
                value,
                span: lo.to(self.prev_span()),
            });
            if self.eat(&T::Comma) {
                continue;
            }
            self.skip_terminators();
            self.expect(&T::RBrace, "`,` or `}` after a variant")?;
            break;
        }
        if variants.is_empty() {
            self.error(open, format!("word `{name}` has no variants"));
        }
        Ok(WordDef {
            name,
            repr,
            variants,
        })
    }

    fn union_def(&mut self) -> PResult<UnionDef> {
        self.bump(); // union
        let name = self.ident("a union name")?;
        self.expect(&T::LParen, "`(` listing the union's variants")?;
        let variants = self.list(&T::RParen, true, "a union's variant list", |p| {
            p.ident("a variant name")
        })?;
        let implements = if self.eat(&T::Colon) {
            self.path_list("an interface name")?
        } else {
            Vec::new()
        };
        let contracts = if self.at(&T::LBrace) {
            self.contract_body()?
        } else {
            Vec::new()
        };
        Ok(UnionDef {
            name,
            variants,
            implements,
            contracts,
        })
    }

    fn interface_def(&mut self) -> PResult<InterfaceDef> {
        self.bump(); // interface
        let name = self.ident("an interface name")?;
        let extends = if self.eat(&T::Colon) {
            self.path_list("an interface name")?
        } else {
            Vec::new()
        };
        let contracts = self.contract_body()?;
        Ok(InterfaceDef {
            name,
            extends,
            contracts,
        })
    }

    fn contract_body(&mut self) -> PResult<Vec<Contract>> {
        let open = self.expect(&T::LBrace, "`{`")?;
        let mut contracts = Vec::new();
        loop {
            self.skip_terminators();
            match self.kind() {
                T::RBrace => {
                    self.bump();
                    return Ok(contracts);
                }
                T::Eof => {
                    self.report(
                        Diagnostic::error(open, "unclosed contract list").label("opened here"),
                    );
                    return Err(Reported);
                }
                T::Keyword(Kw::Type) => match self.contract() {
                    Ok(c) => contracts.push(c),
                    Err(Reported) => self.sync_member(),
                },
                _ => {
                    self.error_expected("a contract such as `type area: f() -> f64`");
                    self.sync_member();
                }
            }
        }
    }

    fn contract(&mut self) -> PResult<Contract> {
        let lo = self.bump().span; // type
        let name = match self.kind() {
            T::Ident(_) | T::MutFnIdent(_) => self.take_name("a method name")?,
            _ => return Err(self.error_expected("a method name")),
        };
        self.expect(&T::Colon, "`:` after the contract name")?;
        if !self.at_kw(Kw::F) {
            return Err(self.error_expected("a function type such as `f() -> f64`"));
        }
        let ty = self.ty()?;
        let span = lo.to(ty.span);
        self.end_stmt("a contract")?;
        Ok(Contract { name, ty, span })
    }

    fn type_alias(&mut self) -> PResult<TypeAlias> {
        self.bump(); // type
        let name = self.ident("a type name")?;
        self.expect(&T::Colon, "`:` after the type name")?;
        let ty = self.ty()?;
        self.end_stmt("a type alias")?;
        Ok(TypeAlias { name, ty })
    }

    // ======================================================== types

    fn ty(&mut self) -> PResult<Type> {
        let lo = self.span();
        let kind = match self.kind() {
            T::Keyword(Kw::Effect) => {
                self.bump();
                TypeKind::Effect
            }
            T::Keyword(Kw::F) => return self.fn_type(),
            T::Ident(_) => TypeKind::Named(self.path("a type")?),
            T::LParen => {
                self.bump();
                let first = self.ty()?;
                if !self.at(&T::Comma) {
                    return Err(self.error_expected("`,`: a tuple type has at least two elements"));
                }
                self.bump();
                let mut elems = vec![first];
                elems.extend(self.list(&T::RParen, false, "a tuple type", |p| p.ty())?);
                TypeKind::Tuple(elems)
            }
            T::LBracket => {
                self.bump();
                let len = self.expr()?;
                self.expect(&T::Comma, "`,` in an array type `[length, type]`")?;
                let elem = self.ty()?;
                self.expect(&T::RBracket, "`]`")?;
                TypeKind::Array {
                    len: Box::new(len),
                    elem: Box::new(elem),
                }
            }
            T::Arith(ArithOp::Mul, Modifier::Default) if *self.nth(1) == T::LBracket => {
                self.bump();
                match self.ptr_shape()? {
                    Shape::ThinPtr(t) => TypeKind::ThinPtr(Box::new(t)),
                    Shape::FatPtr { len, elem } => TypeKind::FatPtr {
                        len: Box::new(len),
                        elem: Box::new(elem),
                    },
                    _ => unreachable!("ptr_shape returns a pointer shape"),
                }
            }
            _ => return Err(self.error_expected("a type")),
        };
        Ok(Type {
            kind,
            span: lo.to(self.prev_span()),
        })
    }

    /// `f(i64, i64)[bound] -> i64`
    fn fn_type(&mut self) -> PResult<Type> {
        let lo = self.bump().span; // f
        self.expect(&T::LParen, "`(` after `f` in a function type")?;
        let params = self.list(&T::RParen, false, "a function type", |p| p.ty())?;
        let bound = if self.eat(&T::LBracket) {
            if self.eat(&T::Bang) {
                self.expect(&T::RBracket, "`]` after `!`")?;
                Some(CaptureBound::Pure)
            } else {
                Some(CaptureBound::List(self.list(
                    &T::RBracket,
                    false,
                    "a capture bound",
                    |p| p.capture(),
                )?))
            }
        } else {
            None
        };
        if !self.at(&T::Arrow) {
            return Err(
                self.error_expected("`->` and a return type; function types always have one")
            );
        }
        self.bump();
        let ret = self.ty()?;
        let span = lo.to(ret.span);
        Ok(Type {
            kind: TypeKind::Fn {
                params,
                bound,
                ret: Box::new(ret),
            },
            span,
        })
    }

    /// Whether a parenthesized type really names a type, which makes `(T)` a
    /// cast. Values may not be spelled like types (§7.3), so a known type name
    /// is never a value and there is no ambiguity left (§6.7).
    fn is_type(&self, ty: &Type) -> bool {
        match &ty.kind {
            TypeKind::Named(path) => {
                (path.segments.len() == 1 && is_primitive_type_name(&path.segments[0].text))
                    || self.is_user_type(path)
            }
            TypeKind::Effect
            | TypeKind::ThinPtr(_)
            | TypeKind::FatPtr { .. }
            | TypeKind::Fn { .. } => true,
            TypeKind::Array { elem, .. } => self.is_type(elem),
            TypeKind::Tuple(elems) => elems.iter().all(|e| self.is_type(e)),
        }
    }

    /// A value may not share a name with a type (§6.7, §7.3). A sigil makes a
    /// different name, so only plain names are checked.
    fn check_value_name(&mut self, name: &Name) {
        if name.sigil != Sigil::None {
            return;
        }
        let what = if is_primitive_type_name(&name.text) {
            "a primitive type"
        } else if self.type_names.contains(&name.text) {
            "a type in this project"
        } else {
            return;
        };
        self.report(
            Diagnostic::error(name.span, format!("`{}` is the name of {what}", name.text))
                .label("a value cannot have this name")
                .note("a value may not share a name with a type, so `(T) x` is always a cast (§6.7, §7.3)"),
        );
    }

    // ======================================================== blocks and statements

    fn block(&mut self) -> PResult<Block> {
        let lo = self.expect(&T::LBrace, "`{`")?;
        let mut stmts = Vec::new();
        loop {
            self.skip_terminators();
            match self.kind() {
                T::RBrace => {
                    let hi = self.bump().span;
                    return Ok(Block {
                        stmts,
                        span: lo.to(hi),
                    });
                }
                T::Eof => {
                    self.report(
                        Diagnostic::error(lo, "unclosed `{`").label("this block is never closed"),
                    );
                    return Err(Reported);
                }
                _ => match self.stmt() {
                    Ok(stmt) => stmts.push(stmt),
                    Err(Reported) => self.sync_stmt(),
                },
            }
        }
    }

    fn stmt(&mut self) -> PResult<Stmt> {
        let lo = self.span();
        let kind = match self.kind() {
            T::Keyword(Kw::As) => {
                let bindings = self.bindings()?;
                self.end_stmt("a binding")?;
                StmtKind::Bindings(bindings)
            }
            T::Keyword(Kw::If) => self.if_stmt()?,
            T::Keyword(Kw::While) => {
                self.bump();
                let cond = self.condition("while")?;
                let body = self.block()?;
                StmtKind::While { cond, body }
            }
            T::Keyword(Kw::Do) => match self.nth(1) {
                T::LBrace => {
                    self.bump();
                    let body = self.block()?;
                    self.expect_kw(Kw::Until, "`until` after the `do` block")?;
                    let cond = self.condition("until")?;
                    self.end_stmt("`do ... until`")?;
                    StmtKind::DoUntil { body, cond }
                }
                T::Ident(_) => {
                    self.bump();
                    let name = self.ident("a script name")?;
                    self.end_stmt("`do`")?;
                    StmtKind::Do(name)
                }
                _ => {
                    self.bump();
                    return Err(self.error_expected("`{` (do ... until) or a script name"));
                }
            },
            T::Keyword(Kw::Switch) => self.switch_stmt()?,
            T::Keyword(Kw::When) => self.when_stmt()?,
            T::Keyword(Kw::Script) => {
                self.bump();
                let name = self.ident("a script name")?;
                let body = self.block()?;
                StmtKind::Script { name, body }
            }
            T::Keyword(Kw::Leap) => {
                self.bump();
                let name = self.ident("a label name after `leap`")?;
                self.end_stmt("`leap`")?;
                StmtKind::Leap(name)
            }
            T::Keyword(Kw::Break) => {
                self.bump();
                self.end_stmt("`break`")?;
                StmtKind::Break
            }
            T::Keyword(Kw::Redo) => {
                self.bump();
                self.end_stmt("`redo`")?;
                StmtKind::Redo
            }
            T::FatArrow => {
                let arrow = self.bump().span;
                if matches!(self.kind(), T::Newline | T::Semi | T::RBrace | T::Eof) {
                    self.report(
                        Diagnostic::error(arrow, "return requires a value")
                            .rule("N8")
                            .note("`=>` always returns a value; a function with nothing to return returns `effect`: `=> done`"),
                    );
                    self.end_stmt("`=>`")?;
                    StmtKind::Return(Expr {
                        kind: ExprKind::Error,
                        span: arrow,
                    })
                } else {
                    let value = self.expr()?;
                    self.end_stmt("a return")?;
                    StmtKind::Return(value)
                }
            }
            T::LBrace => StmtKind::Block(self.block()?),
            T::Ident(_) if *self.nth(1) == T::Colon => {
                let name = self.ident("a label")?;
                self.bump(); // :
                self.end_stmt("a label")?;
                StmtKind::Label(name)
            }
            T::Keyword(Kw::Share | Kw::Prerun | Kw::Cycle) => {
                let (share, prerun, cycle) = self.modifiers();
                for span in [share, prerun].into_iter().flatten() {
                    self.report(
                        Diagnostic::error(span, "nested definitions cannot be `share` or `prerun`")
                            .note("see §9.5"),
                    );
                }
                if !self.at_fn_def() {
                    return Err(self.error_expected("a function definition after the modifiers"));
                }
                StmtKind::Fn(Box::new(self.fn_def(false, cycle.is_some())?))
            }
            _ if self.at_fn_def() => StmtKind::Fn(Box::new(self.fn_def(false, false)?)),
            T::Keyword(
                kw
                @ (Kw::Else | Kw::Until | Kw::Case | Kw::Default | Kw::Then | Kw::Is | Kw::Where),
            ) => {
                let message = match kw {
                    Kw::Else => "`else` without a matching `if`".to_string(),
                    _ => format!("unexpected `{}` here", kw.as_str()),
                };
                let span = self.span();
                self.error(span, message);
                return Err(Reported);
            }
            T::Keyword(Kw::Import) => {
                let span = self.span();
                self.error(span, "imports belong at the top of the file");
                return Err(Reported);
            }
            T::Keyword(Kw::Struct | Kw::Word | Kw::Union | Kw::Interface | Kw::Type) => {
                let span = self.span();
                self.error(
                    span,
                    "type definitions are only allowed at the top level of a file",
                );
                return Err(Reported);
            }
            _ => {
                let expr = self.expr()?;
                self.end_stmt("an expression")?;
                StmtKind::Expr(expr)
            }
        };
        Ok(Stmt {
            kind,
            span: lo.to(self.prev_span()),
        })
    }

    /// `( expr )` after `if`, `while`, `until`, `switch`, `when`, `where`.
    fn condition(&mut self, keyword: &str) -> PResult<Expr> {
        if !self.at(&T::LParen) {
            let span = self.span();
            self.report(
                Diagnostic::error(
                    span,
                    format!("expected `(` after `{keyword}`, found {}", self.describe()),
                )
                .note("conditions are always parenthesized (§8.2)"),
            );
            return Err(Reported);
        }
        self.bump();
        let expr = self.expr()?;
        self.expect(&T::RParen, "`)` to close the condition")?;
        Ok(expr)
    }

    fn if_stmt(&mut self) -> PResult<StmtKind> {
        self.bump(); // if
        let cond = self.condition("if")?;
        let body = self.block()?;
        let mut arms = vec![(cond, body)];
        let mut otherwise = None;
        while self.eat_kw(Kw::Else) {
            if self.eat_kw(Kw::If) {
                let cond = self.condition("if")?;
                let body = self.block()?;
                arms.push((cond, body));
            } else {
                otherwise = Some(self.block()?);
                break;
            }
        }
        Ok(StmtKind::If { arms, otherwise })
    }

    fn switch_stmt(&mut self) -> PResult<StmtKind> {
        let kw_span = self.bump().span;
        let subject = self.condition("switch")?;
        let open = self.expect(&T::LBrace, "`{` to start the switch body")?;
        let mut cases = Vec::new();
        let mut default: Option<Block> = None;
        loop {
            self.skip_terminators();
            match self.kind() {
                T::RBrace => {
                    self.bump();
                    break;
                }
                T::Eof => {
                    self.report(
                        Diagnostic::error(open, "unclosed `switch` body").label("opened here"),
                    );
                    return Err(Reported);
                }
                T::Keyword(Kw::Case) => {
                    if default.is_some() {
                        let span = self.span();
                        self.report(
                            Diagnostic::error(
                                span,
                                "`default` must be the last clause in a `switch`",
                            )
                            .note("move this `case` above `default` (§8.3)"),
                        );
                    }
                    match self.case() {
                        Ok(case) => cases.push(case),
                        Err(Reported) => self.sync_stmt(),
                    }
                }
                T::Keyword(Kw::Default) => {
                    let span = self.bump().span;
                    if default.is_some() {
                        self.error(span, "a `switch` has only one `default`");
                    }
                    match self.block() {
                        Ok(block) => default = Some(block),
                        Err(Reported) => self.sync_stmt(),
                    }
                }
                _ => {
                    self.error_expected("`case` or `default`");
                    self.sync_stmt();
                }
            }
        }
        let default = match default {
            Some(block) => block,
            None => {
                self.report(
                    Diagnostic::error(kw_span, "this `switch` has no `default` clause")
                        .note("`default` is required and must be last; use `when` for exhaustive matching (§8.3)"),
                );
                Block {
                    stmts: Vec::new(),
                    span: kw_span,
                }
            }
        };
        Ok(StmtKind::Switch {
            subject,
            cases,
            default,
        })
    }

    fn case(&mut self) -> PResult<Case> {
        let lo = self.bump().span; // case
        let mut values = vec![self.expr()?];
        while self.eat(&T::Comma) {
            values.push(self.expr()?);
        }
        let body = self.block()?;
        let exit = match self.kind() {
            T::Keyword(Kw::Break) => {
                self.bump();
                CaseExit::Break
            }
            T::Keyword(Kw::Then) => {
                self.bump();
                CaseExit::Then
            }
            T::Newline if matches!(self.nth(1), T::Keyword(Kw::Break | Kw::Then)) => {
                self.bump();
                let exit_span = self.span();
                let exit = if self.at_kw(Kw::Break) {
                    CaseExit::Break
                } else {
                    CaseExit::Then
                };
                let word = if exit == CaseExit::Break {
                    "break"
                } else {
                    "then"
                };
                self.report(
                    Diagnostic::error(
                        exit_span,
                        format!("`{word}` must be on the same line as the `}}` closing the case"),
                    )
                    .label("move this up to the previous line")
                    .note("a newline after `}` ends the statement (§2.6)"),
                );
                self.bump();
                exit
            }
            _ => {
                self.report(
                    Diagnostic::error(body.span, "every `case` must end with `break` or `then`")
                        .note("there is no implicit fallthrough: `break` leaves the switch, `then` continues into the next case (§8.3)"),
                );
                CaseExit::Break
            }
        };
        let span = lo.to(self.prev_span());
        self.end_stmt("a case")?;
        Ok(Case {
            values,
            body,
            exit,
            span,
        })
    }

    fn when_stmt(&mut self) -> PResult<StmtKind> {
        self.bump(); // when
        let subject = self.condition("when")?;
        let open = self.expect(&T::LBrace, "`{` to start the `when` body")?;
        let mut arms = Vec::new();
        loop {
            self.skip_terminators();
            match self.kind() {
                T::RBrace => {
                    self.bump();
                    break;
                }
                T::Eof => {
                    self.report(
                        Diagnostic::error(open, "unclosed `when` body").label("opened here"),
                    );
                    return Err(Reported);
                }
                T::Keyword(Kw::Is) => match self.when_arm() {
                    Ok(arm) => arms.push(arm),
                    Err(Reported) => self.sync_stmt(),
                },
                _ => {
                    self.error_expected("`is` to start a `when` arm");
                    self.sync_stmt();
                }
            }
        }
        Ok(StmtKind::When { subject, arms })
    }

    fn when_arm(&mut self) -> PResult<WhenArm> {
        let lo = self.bump().span; // is
        let pattern = self.pattern()?;
        let guard = if self.eat_kw(Kw::Where) {
            Some(self.condition("where")?)
        } else {
            None
        };
        if self.at_kw(Kw::Where) {
            let span = self.span();
            self.report(
                Diagnostic::error(span, "an arm has at most one guard")
                    .note("combine conditions with `&&` inside one `where (...)` (§8.4)"),
            );
            return Err(Reported);
        }
        let body = self.block()?;
        Ok(WhenArm {
            pattern,
            guard,
            span: lo.to(body.span),
            body,
        })
    }

    fn pattern(&mut self) -> PResult<Pattern> {
        let lo = self.span();
        let rel = match self.kind() {
            T::EqEq => Some(RelOp::Eq),
            T::NotEq => Some(RelOp::Ne),
            T::Lt => Some(RelOp::Lt),
            T::Gt => Some(RelOp::Gt),
            T::LtEq => Some(RelOp::Le),
            T::GtEq => Some(RelOp::Ge),
            _ => None,
        };
        let kind = if let Some(op) = rel {
            self.bump();
            // Above comparison, so `is >= a + 1` reads as `>= (a + 1)`.
            PatternKind::Relational(op, self.binary(PREC_BITOR)?)
        } else {
            match self.kind() {
                T::Wildcard => {
                    self.bump();
                    PatternKind::Wildcard
                }
                T::LParen => {
                    self.bump();
                    PatternKind::Tuple(
                        self.list(&T::RParen, false, "a tuple pattern", |p| p.pattern())?,
                    )
                }
                T::Int { .. }
                | T::Float(_)
                | T::Ascii(_)
                | T::Str(_)
                | T::Keyword(Kw::True | Kw::False | Kw::Done | Kw::Fail) => {
                    PatternKind::Literal(self.primary()?)
                }
                T::Arith(ArithOp::Sub, Modifier::Default)
                    if matches!(self.nth(1), T::Int { .. } | T::Float(_)) =>
                {
                    PatternKind::Literal(self.unary()?)
                }
                T::Ident(_) => PatternKind::Path(self.path("a pattern")?),
                T::LBracket | T::Keyword(Kw::F | Kw::Effect) => PatternKind::Type(self.ty()?),
                T::Arith(ArithOp::Mul, Modifier::Default) if *self.nth(1) == T::LBracket => {
                    PatternKind::Type(self.ty()?)
                }
                _ => return Err(self.error_expected("a pattern")),
            }
        };
        Ok(Pattern {
            kind,
            span: lo.to(self.prev_span()),
        })
    }

    /// `as a: 1, as b: 2`
    fn bindings(&mut self) -> PResult<Vec<Binding>> {
        let mut out = vec![self.binding()?];
        while self.at(&T::Comma) {
            self.bump();
            if !self.at_kw(Kw::As) {
                let span = self.span();
                self.report(
                    Diagnostic::error(
                        span,
                        format!("expected `as` after `,`, found {}", self.describe()),
                    )
                    .note("each binding in a list starts with `as`: `as a: 1, as b: 2`"),
                );
                return Err(Reported);
            }
            out.push(self.binding()?);
        }
        Ok(out)
    }

    fn binding(&mut self) -> PResult<Binding> {
        let lo = self.expect_kw(Kw::As, "`as`")?;
        let (name, shape) = match self.kind() {
            T::PtrIdent(_) => {
                let name = self.take_name("a name")?;
                (name, self.ptr_shape()?)
            }
            T::Ident(_) | T::MutIdent(_) | T::MutFnIdent(_) | T::MutMutFnIdent(_) => {
                let name = self.take_name("a name")?;
                self.check_value_name(&name);
                let shape = if self.at(&T::LBracket) {
                    self.array_shape()?
                } else {
                    Shape::Inferred
                };
                (name, shape)
            }
            _ => return Err(self.error_expected("a name after `as`")),
        };
        if !self.at(&T::Colon) {
            let span = self.span();
            let d = if matches!(self.kind(), T::Newline | T::Semi | T::RBrace | T::Eof) {
                Diagnostic::error(span, format!("binding `{name}` needs an initializer"))
                    .note("every binding has a value; there is no declaration without one (§5.1)")
            } else {
                Diagnostic::error(
                    span,
                    format!("expected `:` after `{name}`, found {}", self.describe()),
                )
            };
            self.report(d);
            return Err(Reported);
        }
        self.bump();
        let value = self.expr()?;
        Ok(Binding {
            span: lo.to(value.span),
            name,
            shape,
            value,
        })
    }

    // ======================================================== expressions

    pub(crate) fn expr(&mut self) -> PResult<Expr> {
        if self.at_kw(Kw::So) {
            let e = self.so_expr()?;
            if self.assign_op().is_some() {
                let span = self.span();
                self.error(span, "a `so` expression cannot be assigned to");
                return Err(Reported);
            }
            return Ok(e);
        }
        let lhs = self.binary(0)?;
        if let Some(op) = self.assign_op() {
            self.bump();
            let value = self.expr()?; // right-associative
            let span = lhs.span.to(value.span);
            return Ok(Expr {
                kind: ExprKind::Assign {
                    op,
                    target: Box::new(lhs),
                    value: Box::new(value),
                },
                span,
            });
        }
        Ok(lhs)
    }

    fn assign_op(&self) -> Option<AssignOp> {
        Some(match self.kind() {
            T::ColonEq => AssignOp::Set,
            T::ArithAssign(op, m) => AssignOp::Arith(*op, *m),
            T::ShlEq => AssignOp::Shl,
            T::ShrEq => AssignOp::Shr,
            T::AmpEq => AssignOp::BitAnd,
            T::PipeEq => AssignOp::BitOr,
            T::CaretEq => AssignOp::BitXor,
            _ => return None,
        })
    }

    /// `so(cond) a else b`. Branches are expressions; a branch may itself be
    /// a `so` expression, so chains flatten without parentheses (§7.2).
    fn so_expr(&mut self) -> PResult<Expr> {
        let lo = self.bump().span; // so
        self.expect(&T::LParen, "`(` after `so`")?;
        let cond = self.expr()?;
        self.expect(&T::RParen, "`)` to close the `so` condition")?;
        let then = self.so_branch()?;
        if !self.at_kw(Kw::Else) {
            let span = self.span();
            self.report(
                Diagnostic::error(
                    span,
                    format!(
                        "expected `else` in a `so` expression, found {}",
                        self.describe()
                    ),
                )
                .note("`else` is mandatory; there is no one-armed `so` (§7.2)"),
            );
            return Err(Reported);
        }
        self.bump();
        let otherwise = self.so_branch()?;
        let span = lo.to(otherwise.span);
        Ok(Expr {
            kind: ExprKind::So {
                cond: Box::new(cond),
                then: Box::new(then),
                otherwise: Box::new(otherwise),
            },
            span,
        })
    }

    fn so_branch(&mut self) -> PResult<Expr> {
        if self.at_kw(Kw::So) {
            return self.so_expr();
        }
        let branch = self.binary(0)?;
        if self.assign_op().is_some() {
            let span = self.span();
            self.report(
                Diagnostic::error(span, "assignment is not allowed inside a `so` branch")
                    .note("branches are expressions (§7.2)"),
            );
            return Err(Reported);
        }
        Ok(branch)
    }

    fn binop(&self) -> Option<(BinOp, u8)> {
        Some(match self.kind() {
            T::OrOr => (BinOp::Or, PREC_OR),
            T::AndAnd => (BinOp::And, PREC_AND),
            T::EqEq => (BinOp::Rel(RelOp::Eq), PREC_CMP),
            T::NotEq => (BinOp::Rel(RelOp::Ne), PREC_CMP),
            T::Lt => (BinOp::Rel(RelOp::Lt), PREC_CMP),
            T::Gt => (BinOp::Rel(RelOp::Gt), PREC_CMP),
            T::LtEq => (BinOp::Rel(RelOp::Le), PREC_CMP),
            T::GtEq => (BinOp::Rel(RelOp::Ge), PREC_CMP),
            T::Pipe => (BinOp::BitOr, PREC_BITOR),
            T::Caret => (BinOp::BitXor, PREC_BITXOR),
            T::Amp => (BinOp::BitAnd, PREC_BITAND),
            T::Shl => (BinOp::Shl, PREC_SHIFT),
            T::Shr => (BinOp::Shr, PREC_SHIFT),
            T::Arith(op @ (ArithOp::Add | ArithOp::Sub), m) => (BinOp::Arith(*op, *m), PREC_ADD),
            T::Arith(op, m) => (BinOp::Arith(*op, *m), PREC_MUL),
            _ => return None,
        })
    }

    /// Precedence climbing over the binary operators (§7.1).
    fn binary(&mut self, min_prec: u8) -> PResult<Expr> {
        let mut lhs = self.unary()?;
        while let Some((op, prec)) = self.binop() {
            if prec < min_prec {
                break;
            }
            self.bump();
            let rhs = self.binary(prec + 1)?;
            let span = lhs.span.to(rhs.span);
            lhs = Expr {
                kind: ExprKind::Binary {
                    op,
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                },
                span,
            };
            if prec == PREC_CMP && matches!(self.binop(), Some((_, PREC_CMP))) {
                let span = self.span();
                self.report(
                    Diagnostic::error(span, "comparison operators do not chain")
                        .note("comparison is non-associative; write `a < b && b < c` (§6.9)"),
                );
                return Err(Reported);
            }
        }
        Ok(lhs)
    }

    fn unary(&mut self) -> PResult<Expr> {
        let lo = self.span();
        let op = match self.kind() {
            T::Arith(ArithOp::Add, Modifier::Default) => Some(UnaryOp::Plus),
            T::Arith(ArithOp::Sub, Modifier::Default) => Some(UnaryOp::Neg),
            T::Bang => Some(UnaryOp::Not),
            T::Tilde => Some(UnaryOp::BitNot),
            T::Amp => Some(UnaryOp::AddrOf),
            _ => None,
        };
        if let Some(op) = op {
            self.bump();
            let operand = self.unary()?;
            let span = lo.to(operand.span);
            return Ok(Expr {
                kind: ExprKind::Unary {
                    op,
                    operand: Box::new(operand),
                },
                span,
            });
        }
        match self.kind() {
            T::LParen => self.paren_or_cast(),
            T::Keyword(Kw::So) => {
                self.report(
                    Diagnostic::error(lo, "a `so` expression used as an operand must be parenthesized")
                        .note("write `(so(c) a else b)`; otherwise the `else` branch extends as far as it can (§7.2)"),
                );
                self.so_expr()
            }
            _ => {
                let primary = self.primary()?;
                if matches!(primary.kind, ExprKind::Name(_)) {
                    self.postfix(primary)
                } else {
                    Ok(primary)
                }
            }
        }
    }

    fn primary(&mut self) -> PResult<Expr> {
        let token = self.peek();
        let kind = match &token.kind {
            T::Int { digits, radix } => ExprKind::Int {
                digits: digits.clone(),
                radix: *radix,
            },
            T::Float(text) => ExprKind::Float(text.clone()),
            T::Ascii(b) => ExprKind::Ascii(*b),
            T::Str(bytes) => ExprKind::Str(bytes.clone()),
            T::Keyword(Kw::True) => ExprKind::Bool(true),
            T::Keyword(Kw::False) => ExprKind::Bool(false),
            T::Keyword(Kw::Done) => ExprKind::Effect(true),
            T::Keyword(Kw::Fail) => ExprKind::Effect(false),
            T::Ident(_)
            | T::MutIdent(_)
            | T::MutFnIdent(_)
            | T::MutMutFnIdent(_)
            | T::PtrIdent(_) => {
                let name = self.take_name("a name")?;
                let span = name.span;
                return Ok(Expr {
                    kind: ExprKind::Name(name),
                    span,
                });
            }
            T::LBracket => {
                self.bump();
                let elems = self.list(&T::RBracket, false, "an array literal", |p| p.expr())?;
                return Ok(Expr {
                    kind: ExprKind::Array(elems),
                    span: token.span.to(self.prev_span()),
                });
            }
            T::Backslash => return self.lambda(),
            T::Wildcard => {
                self.error(token.span, "`_` is a pattern, not a value");
                return Err(Reported);
            }
            _ => return Err(self.error_expected("an expression")),
        };
        self.bump();
        Ok(Expr {
            kind,
            span: token.span,
        })
    }

    /// `\(x, y)[captures] { body }`
    fn lambda(&mut self) -> PResult<Expr> {
        let lo = self.bump().span; // backslash
        self.expect(
            &T::LParen,
            "`(` after `\\` to start the lambda's parameters",
        )?;
        let params = self.list(&T::RParen, false, "lambda parameters", |p| match p.kind() {
            T::Ident(_) | T::MutIdent(_) => {
                let name = p.take_name("a parameter name")?;
                p.check_value_name(&name);
                Ok(name)
            }
            _ => Err(p.error_expected("a parameter name")),
        })?;
        let captures = if self.at(&T::LBracket) {
            Some(self.capture_group()?)
        } else {
            None
        };
        let body = self.block()?;
        let span = lo.to(body.span);
        Ok(Expr {
            kind: ExprKind::Lambda(Box::new(Lambda {
                params,
                captures,
                body,
            })),
            span,
        })
    }

    /// Field access, indexing, length and calls after a name.
    fn postfix(&mut self, mut expr: Expr) -> PResult<Expr> {
        loop {
            let postfix_start = matches!(self.kind(), T::Dot | T::LBracket | T::LParen);
            if postfix_start && matches!(expr.kind, ExprKind::Call { .. }) {
                let span = self.span();
                self.report(
                    Diagnostic::error(span, "call chaining is not supported").note(
                        "bind the result first, e.g. `as s: make()` and then `s.area()` (spec T14)",
                    ),
                );
                return Err(Reported);
            }
            match self.kind() {
                T::Dot => {
                    self.bump();
                    let field = match self.kind() {
                        T::Ident(_) | T::MutIdent(_) | T::MutFnIdent(_) | T::MutMutFnIdent(_) => {
                            self.take_name("a field name")?
                        }
                        _ => return Err(self.error_expected("a field or method name after `.`")),
                    };
                    let span = expr.span.to(field.span);
                    expr = Expr {
                        kind: ExprKind::Field {
                            base: Box::new(expr),
                            field,
                        },
                        span,
                    };
                }
                T::LBracket => {
                    self.bump();
                    if self.at(&T::RBracket) {
                        let hi = self.bump().span;
                        let span = expr.span.to(hi);
                        expr = Expr {
                            kind: ExprKind::Len(Box::new(expr)),
                            span,
                        };
                    } else {
                        let index = self.expr()?;
                        let hi = self.expect(&T::RBracket, "`]` to close the index")?;
                        let span = expr.span.to(hi);
                        expr = Expr {
                            kind: ExprKind::Index {
                                base: Box::new(expr),
                                index: Box::new(index),
                            },
                            span,
                        };
                    }
                }
                T::LParen => {
                    self.bump();
                    let args = self.args()?;
                    let span = expr.span.to(self.prev_span());
                    expr = Expr {
                        kind: ExprKind::Call {
                            callee: Box::new(expr),
                            args,
                        },
                        span,
                    };
                }
                _ => return Ok(expr),
            }
        }
    }

    /// Call arguments after `(`: positional first, then named (§7.3).
    fn args(&mut self) -> PResult<Vec<Arg>> {
        let mut seen_named = false;
        self.list(&T::RParen, false, "an argument list", |p| {
            let lo = p.span();
            let name = if matches!(p.kind(), T::Ident(_) | T::MutIdent(_)) && *p.nth(1) == T::Colon
            {
                let name = p.take_name("an argument name")?;
                p.bump(); // :
                Some(name)
            } else {
                None
            };
            let ref_span = p.at_kw(Kw::Ref).then(|| p.span());
            let is_ref = p.eat_kw(Kw::Ref);
            let value = p.expr()?;
            if let Some(span) = ref_span
                && value.is_literal()
            {
                p.report(
                    Diagnostic::error(span.to(value.span), "`ref` needs storage, not a literal")
                        .note("`ref` passes a view of existing storage (§7.3)"),
                );
            }
            match &name {
                Some(_) => seen_named = true,
                None if seen_named => p.report(
                    Diagnostic::error(
                        value.span,
                        "positional arguments must come before named ones",
                    )
                    .note("see §7.3"),
                ),
                None => {}
            }
            Ok(Arg {
                name,
                is_ref,
                span: lo.to(value.span),
                value,
            })
        })
    }

    /// `(` in operand position: a cast, a cast block, a parenthesized
    /// expression, a tuple, or a synthesized binding list (§6.7).
    fn paren_or_cast(&mut self) -> PResult<Expr> {
        let lo = self.span();
        let start = self.pos;
        let diag_count = self.diags.len();
        if let Ok(ty) = self.paren_type()
            && self.diags.len() == diag_count
            && self.is_type(&ty)
        {
            return self.cast_rest(lo, ty);
        }
        // Not a cast: rewind and parse as an expression.
        self.pos = start;
        self.diags.truncate(diag_count);
        self.paren_expr()
    }

    /// Speculatively parses `(type)` or a tuple type `(type, type, ...)`.
    fn paren_type(&mut self) -> PResult<Type> {
        let lo = self.bump().span; // (
        let first = self.ty()?;
        if self.eat(&T::Comma) {
            let mut elems = vec![first];
            elems.extend(self.list(&T::RParen, false, "a tuple type", |p| p.ty())?);
            return Ok(Type {
                kind: TypeKind::Tuple(elems),
                span: lo.to(self.prev_span()),
            });
        }
        self.expect(&T::RParen, "`)`")?;
        Ok(first)
    }

    /// Tokens that can never begin an operand: closers, separators,
    /// terminators and binary-only operators. `*` is excluded because it gets
    /// its own error.
    fn cannot_start_operand(&self) -> bool {
        matches!(
            self.kind(),
            T::RParen
                | T::RBracket
                | T::RBrace
                | T::Comma
                | T::Colon
                | T::Dot
                | T::Newline
                | T::Semi
                | T::Eof
                | T::Keyword(Kw::Else)
        ) || self.assign_op().is_some()
            || matches!(
                self.binop(),
                Some((BinOp::Arith(ArithOp::Div | ArithOp::Rem, _), _))
            )
            || matches!(
                self.binop(),
                Some((
                    BinOp::Rel(_) | BinOp::And | BinOp::Or | BinOp::Shl | BinOp::Shr,
                    _
                ))
            )
    }

    /// After `(T)`: an optional `% | ^` modifier, then a cast block or an operand.
    fn cast_rest(&mut self, lo: Span, ty: Type) -> PResult<Expr> {
        let modifier = match self.kind() {
            T::Arith(ArithOp::Rem, Modifier::Default) => Modifier::Wrap,
            T::Pipe => Modifier::Saturate,
            T::Caret => Modifier::Trap,
            _ => Modifier::Default,
        };
        if modifier != Modifier::Default {
            self.bump();
        }
        if self.at(&T::LBrace) {
            self.bump();
            self.skip_terminators();
            let body = self.expr()?;
            self.skip_terminators();
            let hi = self.expect(&T::RBrace, "`}` to close the cast block")?;
            return Ok(Expr {
                kind: ExprKind::CastBlock {
                    ty,
                    modifier,
                    body: Box::new(body),
                },
                span: lo.to(hi),
            });
        }
        if self.cannot_start_operand() {
            let span = self.span();
            self.report(
                Diagnostic::error(
                    span,
                    format!("a cast needs a value after it, found {}", self.describe()),
                )
                .note("a type is not a value; a type name in parentheses is always a cast (§6.7)"),
            );
            return Err(Reported);
        }
        if matches!(self.kind(), T::Arith(ArithOp::Mul, _)) {
            let span = self.span();
            self.report(
                Diagnostic::error(span, "`*` after a cast is read as multiplication").note(
                    "after `)`, `*` is multiplication (§2.2); to cast a pointee write `(T)(*p)`",
                ),
            );
            return Err(Reported);
        }
        let operand = self.unary()?;
        let span = lo.to(operand.span);
        Ok(Expr {
            kind: ExprKind::Cast {
                ty,
                modifier,
                operand: Box::new(operand),
            },
            span,
        })
    }

    /// `(expr)`, `(a, b, ...)` or `(as a: 1, as b: 2)`.
    fn paren_expr(&mut self) -> PResult<Expr> {
        let lo = self.bump().span; // (
        if self.at_kw(Kw::As) {
            let bindings = self.bindings()?;
            let hi = self.expect(&T::RParen, "`)`")?;
            return Ok(Expr {
                kind: ExprKind::Bindings(bindings),
                span: lo.to(hi),
            });
        }
        let first = self.expr()?;
        if self.eat(&T::Comma) {
            let mut elems = vec![first];
            elems.extend(self.list(&T::RParen, false, "a tuple", |p| p.expr())?);
            let span = lo.to(self.prev_span());
            if elems.len() < 2 {
                self.error(span, "a tuple needs at least two elements");
            }
            return Ok(Expr {
                kind: ExprKind::Tuple(elems),
                span,
            });
        }
        let hi = self.expect(&T::RParen, "`)`")?;
        Ok(Expr {
            span: lo.to(hi),
            ..first
        })
    }
}

// Binding powers, loosest to tightest (§7.1). Assignment and `so` are handled
// outside the climbing loop.
const PREC_OR: u8 = 1;
const PREC_AND: u8 = 2;
const PREC_CMP: u8 = 3;
const PREC_BITOR: u8 = 4;
const PREC_BITXOR: u8 = 5;
const PREC_BITAND: u8 = 6;
const PREC_SHIFT: u8 = 7;
const PREC_ADD: u8 = 8;
const PREC_MUL: u8 = 9;
