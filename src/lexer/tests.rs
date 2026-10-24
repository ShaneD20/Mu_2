//! Lexer tests. Most examples are taken directly from the specification.

use super::*;
use crate::source::FileId;
use TokenKind::*;

fn lex_str(src: &str) -> (Vec<TokenKind>, Vec<Diagnostic>) {
    let mut diags = Vec::new();
    let tokens = lex(FileId(0), src, &mut diags);
    let kinds = tokens
        .into_iter()
        .map(|t| t.kind)
        .filter(|k| *k != Eof)
        .collect();
    (kinds, diags)
}

/// Token kinds, asserting there were no diagnostics.
fn kinds(src: &str) -> Vec<TokenKind> {
    let (kinds, diags) = lex_str(src);
    let messages: Vec<_> = diags.iter().map(|d| d.message.as_str()).collect();
    assert!(
        diags.is_empty(),
        "unexpected diagnostics for {src:?}: {messages:?}"
    );
    kinds
}

/// Tokens printed and joined by spaces; a terminating newline shows as `⏎`.
/// Pointer names print as `*p` and multiplication as `*`, so the output shows
/// exactly how `*` was lexed.
fn shown(src: &str) -> String {
    kinds(src)
        .iter()
        .map(|k| {
            if *k == Newline {
                "⏎".to_string()
            } else {
                k.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Diagnostic messages, prefixed by their rule when they have one.
fn errors(src: &str) -> Vec<String> {
    lex_str(src)
        .1
        .into_iter()
        .map(|d| match d.rule {
            Some(rule) => format!("[{rule}] {}", d.message),
            None => d.message,
        })
        .collect()
}

fn has_error(src: &str, fragment: &str) {
    let errs = errors(src);
    assert!(
        errs.iter().any(|e| e.contains(fragment)),
        "expected an error containing {fragment:?} for {src:?}, got {errs:?}"
    );
}

fn int(digits: &str, radix: u32) -> TokenKind {
    Int {
        digits: digits.into(),
        radix,
    }
}

// ---------------------------------------------------------------- names

#[test]
fn sigils_are_part_of_the_name() {
    assert_eq!(
        kinds("value #value value# #h#"),
        vec![
            Ident("value".into()),
            MutIdent("value".into()),
            MutFnIdent("value".into()),
            MutMutFnIdent("h".into()),
        ]
    );
    assert_eq!(shown("print(value == #value)"), "print ( value == #value )");
    assert_eq!(shown("#sam.bump#()"), "#sam . bump# ( )");
}

#[test]
fn star_fuses_only_where_an_expression_cannot_end() {
    assert_eq!(shown("a * *p"), "a * *p");
    assert_eq!(shown("a *p"), "a * p");
    assert_eq!(shown("f(x) *q"), "f ( x ) * q");
    assert_eq!(shown("arr[0] *q"), "arr [ 0 ] * q");
    assert_eq!(shown("2 *x"), "2 * x");
    assert_eq!(shown("(*p)"), "( *p )");
    assert_eq!(shown("x := *p"), "x := *p");
    assert_eq!(shown("g(a, *q)"), "g ( a , *q )");
    assert_eq!(shown("a\n*p := &x"), "a ⏎ *p := & x");
    assert_eq!(shown("as *p[i32]: &value"), "as *p [ i32 ] : & value");
    assert_eq!(
        shown("as *q[n, i32]: &array"),
        "as *q [ n , i32 ] : & array"
    );
    assert_eq!(shown("as len: *q[]"), "as len : *q [ ]");
}

#[test]
fn star_before_a_bracket_is_a_pointer_type_star() {
    assert_eq!(shown("type t: *[i32]"), "type t : * [ i32 ]");
    assert_eq!(shown("x: *[n, i32]"), "x : * [ n , i32 ]");
}

#[test]
fn wildcard_and_identifiers_with_underscores() {
    assert_eq!(
        kinds("_ _x x_1"),
        vec![Wildcard, Ident("_x".into()), Ident("x_1".into())]
    );
}

#[test]
fn keywords_and_contextual_words() {
    assert_eq!(
        kinds("default f start yield char ascii"),
        vec![
            Keyword(token::Keyword::Default),
            Keyword(token::Keyword::F),
            Ident("start".into()),
            Keyword(token::Keyword::Yield),
            Ident("char".into()),
            Ident("ascii".into()),
        ]
    );
    for &kw in token::Keyword::ALL {
        assert_eq!(kinds(kw.as_str()), vec![Keyword(kw)]);
    }
}

#[test]
fn fixed_point_type_names_stay_whole() {
    assert_eq!(
        kinds("q16.16 q8.8 q15"),
        vec![
            Ident("q16.16".into()),
            Ident("q8.8".into()),
            Ident("q15".into())
        ]
    );
    // An ordinary field access on a name starting with q is unaffected.
    assert_eq!(shown("q1.x"), "q1 . x");
}

#[test]
fn sigil_errors_name_the_rule() {
    has_error("#", "bare `#`");
    has_error("as # p: 1", "no space is allowed after `#`");
    has_error("#if", "keyword");
    has_error("if# ()", "keyword");
    has_error("#_", "`_` cannot be a mutable name");
}

// ---------------------------------------------------------------- operators

#[test]
fn arithmetic_modifiers_lex_by_longest_match() {
    assert_eq!(shown("a +% b +| c +^ d"), "a +% b +| c +^ d");
    assert_eq!(shown("a -% b *| c /^ d %% e"), "a -% b *| c /^ d %% e");
    assert_eq!(shown("#x +%= 1"), "#x +%= 1");
    assert_eq!(shown("#x *|= 2"), "#x *|= 2");
    assert_eq!(kinds("+%"), vec![Arith(ArithOp::Add, Modifier::Wrap)]);
}

#[test]
fn cast_modifiers_follow_the_closing_paren() {
    assert_eq!(shown("(i8)% big"), "( i8 ) % big");
    assert_eq!(shown("(i8)| big"), "( i8 ) | big");
    assert_eq!(shown("(i8) -128"), "( i8 ) - 128");
}

#[test]
fn comparison_logic_and_shifts() {
    assert_eq!(shown("a || b | c && d & e"), "a || b | c && d & e");
    assert_eq!(
        shown("x <<= 1 >> 2 <= 3 >= 4 != 5"),
        "x <<= 1 >> 2 <= 3 >= 4 != 5"
    );
    assert_eq!(shown("a |= b &= c ^= d"), "a |= b &= c ^= d");
    assert_eq!(shown("~a !b"), "~ a ! b");
}

#[test]
fn arrows_and_punctuation() {
    assert_eq!(shown("f() -> i32 { => 0 }"), "f ( ) -> i32 { => 0 }");
    assert_eq!(shown("\\(x)[n] { }"), "\\ ( x ) [ n ] { }");
    assert_eq!(shown("a[1..5]; b"), "a [ 1 .. 5 ] ; b");
    assert_eq!(
        shown("type p: f(i64)[!] -> i64"),
        "type p : f ( i64 ) [ ! ] -> i64"
    );
}

#[test]
fn single_equals_is_rejected_with_a_hint() {
    has_error("a = b", "`=` on its own");
}

// ---------------------------------------------------------------- numbers

#[test]
fn integer_literals_keep_their_digits_as_text() {
    assert_eq!(
        kinds("42 1_000_000 0xFF_FF 0b1010"),
        vec![
            int("42", 10),
            int("1000000", 10),
            int("FFFF", 16),
            int("1010", 2)
        ]
    );
    // Larger than u64 and still fine: range checks happen in sema.
    assert_eq!(
        kinds("340282366920938463463374607431768211455"),
        vec![int("340282366920938463463374607431768211455", 10)]
    );
}

#[test]
fn float_literals() {
    assert_eq!(
        kinds("2.5 1.5e-3 2E+10 3e8 1_0.2_5"),
        vec![
            Float("2.5".into()),
            Float("1.5e-3".into()),
            Float("2e+10".into()),
            Float("3e8".into()),
            Float("10.25".into()),
        ]
    );
}

#[test]
fn a_dot_needs_a_digit_after_it_to_make_a_float() {
    assert_eq!(shown("1..5"), "1 .. 5");
    assert_eq!(shown("t[0].x"), "t [ 0 ] . x");
}

#[test]
fn malformed_numbers() {
    has_error("1_", "cannot end with `_`");
    has_error("1_.5", "cannot end with `_`");
    has_error("0x_FF", "cannot follow the radix prefix");
    has_error("0xFF_", "cannot end with `_`");
    has_error("0x", "must be followed by digits");
    has_error("0b102", "not a valid digit in a base-2 literal");
    has_error("0xFG", "not a valid digit in a base-16 literal");
    has_error("12abc", "invalid suffix `abc`");
    has_error("1.5f", "invalid suffix `f`");
}

// ---------------------------------------------------------------- text

#[test]
fn ascii_literals_and_escapes() {
    assert_eq!(
        kinds(r"'x' '\n' '\'' '\\' '\0' '\v'"),
        vec![
            Ascii(b'x'),
            Ascii(b'\n'),
            Ascii(b'\''),
            Ascii(b'\\'),
            Ascii(0),
            Ascii(11)
        ]
    );
}

#[test]
fn string_literals_and_escapes() {
    assert_eq!(kinds(r#""hello""#), vec![Str(b"hello".to_vec())]);
    assert_eq!(
        kinds(r#""a\t\"b\"\a""#),
        vec![Str(b"a\t\"b\"\x07".to_vec())]
    );
}

#[test]
fn malformed_text_literals() {
    has_error("''", "empty ascii literal");
    has_error("'ab'", "more than one character");
    has_error("'a", "unterminated ascii literal");
    has_error(r"'\x41'", "`\\xNN` are not yet specified");
    has_error(r"'\q'", "unknown escape `\\q`");
    has_error("'é'", "non-ASCII");
    has_error("\"abc", "unterminated string");
    has_error("\"a\nb\"", "unterminated string");
}

// ---------------------------------------------------------------- comments

#[test]
fn comments_are_trivia_and_hide_their_newlines() {
    assert_eq!(shown("a /* one\ntwo */ b"), "a b");
    assert_eq!(shown("a /* é is fine in comments */"), "a");
    // Comments do not nest: the first `*/` ends the comment.
    assert_eq!(shown("/* a /* b */ c"), "c");
    assert_eq!(shown("a / *p"), "a / *p");
}

#[test]
fn unterminated_comment() {
    has_error("a /* never closed", "unterminated comment");
}

#[test]
fn unexpected_characters() {
    has_error("a $ b", "unexpected character `$`");
    has_error("a @ b", "unexpected character `@`");
    has_error("λ", "unexpected character `λ`");
}

// ---------------------------------------------------------------- layout (§2.6)

#[test]
fn n1_consecutive_newlines_collapse() {
    assert_eq!(shown("a\n\n  \n/* c */\nb"), "a ⏎ b");
    assert_eq!(shown("\n\n a"), "a");
}

#[test]
fn n2_newline_after_an_operator_is_suppressed() {
    assert_eq!(shown("as x: a +\n    b"), "as x : a + b");
    assert_eq!(shown("#x :=\n 5"), "#x := 5");
    assert_eq!(shown("a &&\n b ||\n c"), "a && b || c");
    assert_eq!(shown("#x +%=\n 1"), "#x +%= 1");
}

#[test]
fn n3_newline_after_an_opener_is_suppressed() {
    assert_eq!(shown("f(\n  a)"), "f ( a )");
    assert_eq!(shown("[\n1]"), "[ 1 ]");
    assert_eq!(shown("{\n x\n}"), "{ x ⏎ }");
}

#[test]
fn n4_one_newline_after_a_comma() {
    assert_eq!(shown("f(a,\n  b)"), "f ( a , b )");
    assert_eq!(
        errors("f(a,\n\n b)"),
        vec!["[N4] a blank line after `,` is not allowed"]
    );
    // "Even whitespace-separated", and a comment in between does not help.
    assert_eq!(errors("f(a,\n   \n b)").len(), 1);
    assert_eq!(errors("f(a,\n/* gap */\n b)").len(), 1);
}

#[test]
fn n5_newline_before_a_closer_is_suppressed_except_brace() {
    assert_eq!(shown("f(a\n)"), "f ( a )");
    assert_eq!(shown("[1,\n 2\n]"), "[ 1 , 2 ]");
    assert_eq!(shown("{ x\n}"), "{ x ⏎ }");
}

#[test]
fn n6_so_expression_spans_lines() {
    assert_eq!(
        shown("as m: so(a > b)\n  a else\n  b"),
        "as m : so ( a > b ) a else b"
    );
    assert_eq!(shown("so((a))\n b else c"), "so ( ( a ) ) b else c");
    // An ordinary `)` at the end of a line still ends the statement.
    assert_eq!(shown("f(a)\nb"), "f ( a ) ⏎ b");
    // The newline before `else` is kept, which makes it an error.
    has_error("so(c) a\nelse b", "`else` must start on the same line");
}

#[test]
fn n7_terminator_is_optional_before_a_brace() {
    assert_eq!(shown("{ => 7 }"), "{ => 7 }");
}

#[test]
fn n8_newline_after_return_is_kept() {
    assert_eq!(shown("{ =>\n}"), "{ => ⏎ }");
}

#[test]
fn same_line_rule_for_else_and_until() {
    assert_eq!(shown("if (a) { } else { }"), "if ( a ) { } else { }");
    assert_eq!(shown("do { } until (x)"), "do { } until ( x )");
    has_error(
        "if (a) { }\nelse { }",
        "`else` must start on the same line as the preceding `}`",
    );
    has_error(
        "do { }\nuntil (x)",
        "`until` must start on the same line as the preceding `}`",
    );
}

#[test]
fn break_on_its_own_line_after_a_brace_is_left_to_the_parser() {
    assert_eq!(shown("{ }\nbreak"), "{ } ⏎ break");
}

#[test]
fn semicolons_are_explicit_terminators() {
    assert_eq!(shown("as a: 1; as b: 2"), "as a : 1 ; as b : 2");
}

#[test]
fn spans_point_at_the_source() {
    let mut diags = Vec::new();
    let tokens = lex(FileId(0), "as #x: 10", &mut diags);
    let spans: Vec<_> = tokens.iter().map(|t| (t.span.lo, t.span.hi)).collect();
    assert_eq!(spans, vec![(0, 2), (3, 5), (5, 6), (7, 9), (9, 9)]);
}
