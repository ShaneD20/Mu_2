//! Newline layout (spec §2.6).
//!
//! The raw lexer emits one `Newline` per `\n`. This pass keeps a single
//! statement terminator where a newline is meaningful and drops it everywhere
//! the rules say it is not:
//!
//! | Rule | Effect here |
//! |------|-------------|
//! | N1 | a run of newlines collapses to one |
//! | N2 | dropped after an operator that needs a right operand |
//! | N3 | dropped right after `(`, `[`, `{` |
//! | N4 | one newline after `,` is dropped; two or more is an error |
//! | N5 | dropped before `)` or `]` (never before `}`) |
//! | N6 | dropped after the `)` closing a `so` condition and after `else` |
//! | N7 | parser rule: `}` also terminates a statement |
//! | N8 | `=>` is not an operator, so the newline after it is kept |
//!
//! The same-line rule is enforced for `else` and `until`. The case exits
//! `break` and `then` are checked by the parser, because a `break` statement on
//! its own line after a `}` is legal outside a `switch`.
//!
//! Deliberate generalization: N6 drops the newline after *every* `else`, not
//! only in `so` expressions. An `else` can never end a statement, so this
//! cannot change the meaning of a valid program.

use super::token::{Keyword, Token, TokenKind};
use crate::diag::Diagnostic;

pub(super) fn apply(raw: Vec<Token>, diags: &mut Vec<Diagnostic>) -> Vec<Token> {
    let mut out: Vec<Token> = Vec::with_capacity(raw.len());
    // One entry per open bracket: true when it is the `(` of a `so(...)` condition.
    let mut brackets: Vec<bool> = Vec::new();
    // True when the last token emitted is the `)` closing a `so` condition.
    let mut after_so_condition = false;

    let mut i = 0;
    while i < raw.len() {
        if raw[i].kind != TokenKind::Newline {
            let token = raw[i].clone();
            let mut closes_so = false;
            match token.kind {
                TokenKind::LParen => {
                    let is_so =
                        matches!(out.last(), Some(t) if t.kind == TokenKind::Keyword(Keyword::So));
                    brackets.push(is_so);
                }
                TokenKind::LBracket | TokenKind::LBrace => brackets.push(false),
                TokenKind::RParen => closes_so = brackets.pop().unwrap_or(false),
                TokenKind::RBracket | TokenKind::RBrace => {
                    brackets.pop();
                }
                _ => {}
            }
            after_so_condition = closes_so;
            out.push(token);
            i += 1;
            continue;
        }

        // A run of raw newlines (N1).
        let first = i;
        while i < raw.len() && raw[i].kind == TokenKind::Newline {
            i += 1;
        }
        let count = i - first;
        let next = raw.get(i).map(|t| &t.kind);

        let mut keep = match out.last().map(|t| &t.kind) {
            None => false, // leading newlines
            Some(TokenKind::Comma) => {
                if count > 1 {
                    diags.push(
                        Diagnostic::error(
                            raw[first + 1].span,
                            "a blank line after `,` is not allowed",
                        )
                        .rule("N4")
                        .label("second newline after the comma")
                        .note("after `,` at most one newline is skipped; remove the blank line"),
                    );
                }
                false
            }
            Some(TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace) => false, // N3
            Some(kind) if kind.expects_right_operand() => false,                        // N2
            Some(TokenKind::Keyword(Keyword::Else)) => false,                           // N6
            Some(TokenKind::RParen) if after_so_condition => false,                     // N6
            Some(_) => true,
        };

        if keep && matches!(next, Some(TokenKind::RParen | TokenKind::RBracket)) {
            keep = false; // N5
        }

        if keep && let Some(TokenKind::Keyword(kw @ (Keyword::Else | Keyword::Until))) = next {
            let after_brace = matches!(out.last(), Some(t) if t.kind == TokenKind::RBrace);
            let d = if after_brace {
                Diagnostic::error(
                    raw[i].span,
                    format!(
                        "`{}` must start on the same line as the preceding `}}`",
                        kw.as_str()
                    ),
                )
                .note(format!(
                    "write `}} {} ...` on one line; a newline after `}}` ends the statement",
                    kw.as_str()
                ))
            } else {
                Diagnostic::error(
                    raw[i].span,
                    "`else` must start on the same line as the branch before it",
                )
                .note("in a `so` expression the newline before `else` ends the expression")
            };
            diags.push(d.label("move this up to the previous line"));
            keep = false; // recover as if it were on the same line
        }

        if keep {
            out.push(Token {
                kind: TokenKind::Newline,
                span: raw[first].span,
            });
        }
    }
    out
}
