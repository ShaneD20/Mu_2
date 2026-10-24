//! `muc`: the Mu compiler.
//!
//! Pipeline (see `docs/ROADMAP.md`):
//! source → lexer → parser → declaration collection → name resolution →
//! type and safety checks → MIR (interpretable) → prerun → backend → ELF.
//!
//! Implemented so far: source management, diagnostics, the lexer and the parser.

pub mod ast;
pub mod diag;
pub mod lexer;
pub mod parser;
pub mod sexp;
pub mod source;

use diag::Diagnostic;
use source::{FileId, SourceMap};
use std::collections::HashSet;

/// Lexes and parses a set of files that form one project. The type-name
/// pre-pass runs over every file before any file is parsed (§6.7, §14.2).
pub fn parse_project(
    sources: &SourceMap,
    files: &[FileId],
    diags: &mut Vec<Diagnostic>,
) -> Vec<ast::File> {
    let token_streams: Vec<_> = files
        .iter()
        .map(|&id| lexer::lex(id, &sources.get(id).text, diags))
        .collect();
    let mut type_names = HashSet::new();
    for tokens in &token_streams {
        parser::collect_type_names(tokens, &mut type_names);
    }
    token_streams
        .iter()
        .map(|tokens| parser::parse_file(tokens, &type_names, diags))
        .collect()
}
