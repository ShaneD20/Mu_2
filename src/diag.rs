//! Diagnostics: errors and warnings with source locations, rendered in the
//! familiar rustc style.
//!
//! The spec asks for diagnostics that *name the rule* being enforced ("an error
//! naming the rule"), so every diagnostic can carry the spec rule id (`N4`,
//! `S17`, ...) and notes explaining the fix.

use crate::source::{SourceMap, Span};
use std::fmt::Write;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub severity: Severity,
    /// The spec rule this diagnostic enforces, such as `"N4"` or `"S17"`.
    pub rule: Option<&'static str>,
    pub message: String,
    pub span: Span,
    /// Short text printed next to the caret.
    pub label: Option<String>,
    pub notes: Vec<String>,
}

impl Diagnostic {
    pub fn error(span: Span, message: impl Into<String>) -> Diagnostic {
        Diagnostic {
            severity: Severity::Error,
            rule: None,
            message: message.into(),
            span,
            label: None,
            notes: Vec::new(),
        }
    }

    pub fn warning(span: Span, message: impl Into<String>) -> Diagnostic {
        Diagnostic {
            severity: Severity::Warning,
            ..Diagnostic::error(span, message)
        }
    }

    pub fn rule(mut self, rule: &'static str) -> Diagnostic {
        self.rule = Some(rule);
        self
    }

    pub fn label(mut self, label: impl Into<String>) -> Diagnostic {
        self.label = Some(label.into());
        self
    }

    pub fn note(mut self, note: impl Into<String>) -> Diagnostic {
        self.notes.push(note.into());
        self
    }

    /// Renders the diagnostic with the offending source line and a caret.
    pub fn render(&self, sources: &SourceMap) -> String {
        let file = sources.get(self.span.file);
        let (line, col) = file.line_col(self.span.lo as usize);
        let severity = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };

        let mut out = String::new();
        match self.rule {
            Some(rule) => writeln!(out, "{severity}[{rule}]: {}", self.message),
            None => writeln!(out, "{severity}: {}", self.message),
        }
        .unwrap();

        let pad = " ".repeat(line.to_string().len());
        let text = file.line_text(line);
        writeln!(out, "{pad}--> {}:{line}:{col}", file.name).unwrap();
        writeln!(out, "{pad} |").unwrap();
        writeln!(out, "{line} | {text}").unwrap();

        // Underline the span on its first line; keep tabs so the caret lines up.
        let (end_line, end_col) = file.line_col(self.span.hi as usize);
        let width = if end_line == line && end_col > col {
            end_col - col
        } else {
            1
        };
        let indent: String = text
            .chars()
            .take(col - 1)
            .map(|c| if c == '\t' { '\t' } else { ' ' })
            .collect();
        write!(out, "{pad} | {indent}{}", "^".repeat(width)).unwrap();
        if let Some(label) = &self.label {
            write!(out, " {label}").unwrap();
        }
        out.push('\n');
        for note in &self.notes {
            writeln!(out, "{pad} = note: {note}").unwrap();
        }
        out
    }
}

pub fn has_errors(diags: &[Diagnostic]) -> bool {
    diags.iter().any(|d| d.severity == Severity::Error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_location_caret_and_notes() {
        let mut sources = SourceMap::new();
        let file = sources.add("demo.mu", "as x: 1\nas y: x + oops\n");
        let lo = 18;
        let d = Diagnostic::error(Span::new(file, lo, lo + 4), "unknown name `oops`")
            .rule("S1")
            .label("not found in this scope")
            .note("names are resolved on (sigil, text)");
        let text = d.render(&sources);
        assert_eq!(
            text,
            "error[S1]: unknown name `oops`\n \
             --> demo.mu:2:11\n  \
             |\n\
             2 | as y: x + oops\n  \
             |           ^^^^ not found in this scope\n  \
             = note: names are resolved on (sigil, text)\n"
        );
    }
}
