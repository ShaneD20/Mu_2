//! Token definitions.

use crate::source::Span;
use std::fmt;

macro_rules! keywords {
    ($($variant:ident => $text:literal,)*) => {
        /// Reserved words (spec §2.5). `start` is contextual and lexes as an identifier.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum Keyword { $($variant,)* }

        impl Keyword {
            pub const ALL: &'static [Keyword] = &[$(Keyword::$variant,)*];

            pub fn lookup(text: &str) -> Option<Keyword> {
                match text {
                    $($text => Some(Keyword::$variant),)*
                    _ => None,
                }
            }

            pub fn as_str(self) -> &'static str {
                match self {
                    $(Keyword::$variant => $text,)*
                }
            }
        }
    };
}

keywords! {
    So => "so", As => "as", Do => "do", Until => "until", While => "while",
    Break => "break", Redo => "redo", Leap => "leap",
    Struct => "struct", Union => "union", Interface => "interface", Word => "word",
    Type => "type", Import => "import",
    Switch => "switch", Case => "case", Default => "default", Then => "then",
    If => "if", Else => "else", When => "when", Is => "is", Where => "where",
    Share => "share", Prerun => "prerun", Cycle => "cycle", Script => "script", Yield => "yield",
    F => "f", Ref => "ref", Copy => "copy",
    Effect => "effect", True => "true", False => "false", Done => "done", Fail => "fail",
}

/// The overflow behavior written after an arithmetic operator or cast (spec §6.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Modifier {
    /// No suffix: the type's default (trap for integers, saturate for fixed point).
    Default,
    /// `%`: wrap modulo the declared width.
    Wrap,
    /// `|`: saturate at the type's minimum or maximum.
    Saturate,
    /// `^`: trap.
    Trap,
}

impl Modifier {
    pub fn as_str(self) -> &'static str {
        match self {
            Modifier::Default => "",
            Modifier::Wrap => "%",
            Modifier::Saturate => "|",
            Modifier::Trap => "^",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArithOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
}

impl ArithOp {
    pub fn as_str(self) -> &'static str {
        match self {
            ArithOp::Add => "+",
            ArithOp::Sub => "-",
            ArithOp::Mul => "*",
            ArithOp::Div => "/",
            ArithOp::Rem => "%",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // ---- Names. A sigil is part of the name (§2.2); the string excludes it. ----
    /// `value`
    Ident(String),
    /// `#value`: mutable storage.
    MutIdent(String),
    /// `value#`: a mutating function.
    MutFnIdent(String),
    /// `#value#`: a rebindable binding holding a mutating function.
    MutMutFnIdent(String),
    /// `*value`: a pointer name, fused only where an expression cannot end.
    PtrIdent(String),
    /// `_`
    Wildcard,

    // ---- Literals. Numbers stay as text until sema knows the target type (§2.3). ----
    /// Digits without separators or radix prefix.
    Int {
        digits: String,
        radix: u32,
    },
    /// Normalized text without separators, e.g. `1.5e-3`.
    Float(String),
    Ascii(u8),
    /// Bytes after escape processing. The string type itself is open (T2).
    Str(Vec<u8>),

    Keyword(Keyword),

    // ---- Operators ----
    /// `+ - * / %`, optionally suffixed: `+%`, `*|`, `-^`, ...
    Arith(ArithOp, Modifier),
    /// `+=`, `+%=`, `*|=`, ...
    ArithAssign(ArithOp, Modifier),
    /// `:=`
    ColonEq,
    ShlEq,
    ShrEq,
    AmpEq,
    PipeEq,
    CaretEq,
    EqEq,
    NotEq,
    Lt,
    Gt,
    LtEq,
    GtEq,
    AndAnd,
    OrOr,
    Pipe,
    Caret,
    /// `&`: bitwise and, or address-of.
    Amp,
    Shl,
    Shr,
    Tilde,
    Bang,

    // ---- Punctuation ----
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,
    Comma,
    Dot,
    /// `..`: reserved for slices (T9).
    DotDot,
    Colon,
    Semi,
    /// `->`
    Arrow,
    /// `=>`: return.
    FatArrow,
    /// `\`: starts a lambda.
    Backslash,

    /// A statement-terminating newline (after layout, §2.6).
    Newline,
    Eof,
}

impl TokenKind {
    /// Whether this token can be the last token of an expression. Drives the
    /// `*` fusion rule: after such a token, `*` is multiplication (§2.2).
    pub fn can_end_expr(&self) -> bool {
        use TokenKind::*;
        matches!(
            self,
            Ident(_)
                | MutIdent(_)
                | MutFnIdent(_)
                | MutMutFnIdent(_)
                | PtrIdent(_)
                | Int { .. }
                | Float(_)
                | Ascii(_)
                | Str(_)
                | RParen
                | RBracket
                | Keyword(self::Keyword::True | self::Keyword::False)
                | Keyword(self::Keyword::Done | self::Keyword::Fail)
        )
    }

    /// Operators that need a right operand, so a newline after them cannot end
    /// a statement (rule N2).
    pub fn expects_right_operand(&self) -> bool {
        use TokenKind::*;
        matches!(
            self,
            Arith(..)
                | ArithAssign(..)
                | ColonEq
                | ShlEq
                | ShrEq
                | AmpEq
                | PipeEq
                | CaretEq
                | EqEq
                | NotEq
                | Lt
                | Gt
                | LtEq
                | GtEq
                | AndAnd
                | OrOr
                | Pipe
                | Caret
                | Amp
                | Shl
                | Shr
                | Tilde
                | Bang
        )
    }

    /// A short category name, used by `muc lex`.
    pub fn category(&self) -> &'static str {
        use TokenKind::*;
        match self {
            Ident(_) => "ident",
            MutIdent(_) => "mut-ident",
            MutFnIdent(_) => "mutfn-ident",
            MutMutFnIdent(_) => "mut-mutfn",
            PtrIdent(_) => "ptr-ident",
            Wildcard => "wildcard",
            Int { .. } => "int",
            Float(_) => "float",
            Ascii(_) => "ascii",
            Str(_) => "string",
            Keyword(_) => "keyword",
            Arith(..) | ArithAssign(..) | ColonEq | ShlEq | ShrEq | AmpEq | PipeEq | CaretEq
            | EqEq | NotEq | Lt | Gt | LtEq | GtEq | AndAnd | OrOr | Pipe | Caret | Amp | Shl
            | Shr | Tilde | Bang => "op",
            Newline => "newline",
            Eof => "eof",
            _ => "punct",
        }
    }
}

fn escape_byte(b: u8, quote: u8) -> String {
    match b {
        b'\n' => "\\n".into(),
        b'\t' => "\\t".into(),
        b'\r' => "\\r".into(),
        0 => "\\0".into(),
        7 => "\\a".into(),
        8 => "\\b".into(),
        11 => "\\v".into(),
        12 => "\\f".into(),
        b'\\' => "\\\\".into(),
        b if b == quote => format!("\\{}", b as char),
        b => (b as char).to_string(),
    }
}

impl fmt::Display for TokenKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use TokenKind::*;
        match self {
            Ident(s) => write!(f, "{s}"),
            MutIdent(s) => write!(f, "#{s}"),
            MutFnIdent(s) => write!(f, "{s}#"),
            MutMutFnIdent(s) => write!(f, "#{s}#"),
            PtrIdent(s) => write!(f, "*{s}"),
            Wildcard => write!(f, "_"),
            Int { digits, radix } => match radix {
                16 => write!(f, "0x{digits}"),
                2 => write!(f, "0b{digits}"),
                _ => write!(f, "{digits}"),
            },
            Float(s) => write!(f, "{s}"),
            Ascii(b) => write!(f, "'{}'", escape_byte(*b, b'\'')),
            Str(bytes) => {
                let body: String = bytes.iter().map(|&b| escape_byte(b, b'"')).collect();
                write!(f, "\"{body}\"")
            }
            Keyword(k) => write!(f, "{}", k.as_str()),
            Arith(op, m) => write!(f, "{}{}", op.as_str(), m.as_str()),
            ArithAssign(op, m) => write!(f, "{}{}=", op.as_str(), m.as_str()),
            ColonEq => write!(f, ":="),
            ShlEq => write!(f, "<<="),
            ShrEq => write!(f, ">>="),
            AmpEq => write!(f, "&="),
            PipeEq => write!(f, "|="),
            CaretEq => write!(f, "^="),
            EqEq => write!(f, "=="),
            NotEq => write!(f, "!="),
            Lt => write!(f, "<"),
            Gt => write!(f, ">"),
            LtEq => write!(f, "<="),
            GtEq => write!(f, ">="),
            AndAnd => write!(f, "&&"),
            OrOr => write!(f, "||"),
            Pipe => write!(f, "|"),
            Caret => write!(f, "^"),
            Amp => write!(f, "&"),
            Shl => write!(f, "<<"),
            Shr => write!(f, ">>"),
            Tilde => write!(f, "~"),
            Bang => write!(f, "!"),
            LParen => write!(f, "("),
            RParen => write!(f, ")"),
            LBracket => write!(f, "["),
            RBracket => write!(f, "]"),
            LBrace => write!(f, "{{"),
            RBrace => write!(f, "}}"),
            Comma => write!(f, ","),
            Dot => write!(f, "."),
            DotDot => write!(f, ".."),
            Colon => write!(f, ":"),
            Semi => write!(f, ";"),
            Arrow => write!(f, "->"),
            FatArrow => write!(f, "=>"),
            Backslash => write!(f, "\\"),
            Newline => write!(f, "newline"),
            Eof => write!(f, "end of file"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}
