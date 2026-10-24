//! Parser tests. Trees are compared as flat S-expressions; most inputs come
//! from the specification.

use super::*;
use crate::lexer::lex;
use crate::sexp;
use crate::source::FileId;

fn parse(src: &str) -> (File, Vec<Diagnostic>) {
    let mut diags = Vec::new();
    let tokens = lex(FileId(0), src, &mut diags);
    let mut types = HashSet::new();
    collect_type_names(&tokens, &mut types);
    let file = parse_file(&tokens, &types, &mut diags);
    (file, diags)
}

fn messages(diags: &[Diagnostic]) -> Vec<String> {
    diags
        .iter()
        .map(|d| {
            let mut text = match d.rule {
                Some(rule) => format!("[{rule}] {}", d.message),
                None => d.message.clone(),
            };
            for note in &d.notes {
                text.push_str(" | note: ");
                text.push_str(note);
            }
            text
        })
        .collect()
}

fn clean(src: &str) -> File {
    let (file, diags) = parse(src);
    assert!(
        diags.is_empty(),
        "unexpected diagnostics for:\n{src}\n{:?}",
        messages(&diags)
    );
    file
}

/// Every item, flat.
fn items(src: &str) -> Vec<String> {
    sexp::file(&clean(src)).iter().map(|s| s.flat()).collect()
}

fn item(src: &str) -> String {
    let all = items(src);
    assert_eq!(all.len(), 1, "expected one item in {src:?}");
    all.into_iter().next().unwrap()
}

/// The statements of a function body.
fn stmts(body: &str) -> Vec<String> {
    stmts_with("", body)
}

fn stmts_with(prelude: &str, body: &str) -> Vec<String> {
    let src = format!("{prelude}\nt f() -> effect {{\n{body}\n}}\n");
    let file = clean(&src);
    let Some(ItemKind::Fn(def)) = file.items.last().map(|i| &i.kind) else {
        panic!("expected the test function last");
    };
    def.body
        .stmts
        .iter()
        .map(|s| sexp::stmt(s).flat())
        .collect()
}

fn stmt(body: &str) -> String {
    let all = stmts(body);
    assert_eq!(all.len(), 1, "expected one statement in {body:?}: {all:?}");
    all.into_iter().next().unwrap()
}

/// An expression, parsed as the value of a return statement.
fn expr(src: &str) -> String {
    expr_with("", src)
}

fn expr_with(prelude: &str, src: &str) -> String {
    let all = stmts_with(prelude, &format!("=> {src}"));
    let s = &all[0];
    s.strip_prefix("(return ")
        .and_then(|s| s.strip_suffix(')'))
        .unwrap()
        .to_string()
}

/// Diagnostics for a whole file.
fn errors(src: &str) -> Vec<String> {
    messages(&parse(src).1)
}

/// Diagnostics for a function body.
fn body_errors(body: &str) -> Vec<String> {
    errors(&format!("t f() -> effect {{\n{body}\n}}\n"))
}

#[track_caller]
fn assert_error(errs: Vec<String>, fragment: &str) {
    assert!(
        errs.iter().any(|e| e.contains(fragment)),
        "expected an error containing {fragment:?}, got {errs:?}"
    );
}

// ============================================================ expressions

#[test]
fn precedence_follows_the_spec_table() {
    assert_eq!(expr("a + b * c"), "(+ a (* b c))");
    assert_eq!(expr("a * b + c"), "(+ (* a b) c)");
    assert_eq!(expr("a - b - c"), "(- (- a b) c)");
    assert_eq!(expr("a / b % c"), "(% (/ a b) c)");
    assert_eq!(expr("a || b && c"), "(|| a (&& b c))");
    assert_eq!(expr("a | b ^ c & d"), "(| a (^ b (& c d)))");
    assert_eq!(expr("a < b + 1"), "(< a (+ b 1))");
}

#[test]
fn bitwise_binds_tighter_than_comparison() {
    // C's `a & b == c` bug is avoided (§6.9).
    assert_eq!(expr("a & b == c"), "(== (& a b) c)");
}

#[test]
fn suffixed_operators() {
    assert_eq!(expr("a +% b *| c"), "(+% a (*| b c))");
    assert_eq!(expr("a -^ b"), "(-^ a b)");
}

#[test]
fn unary_operators() {
    assert_eq!(expr("-a * b"), "(* (neg a) b)");
    assert_eq!(expr("!a && b"), "(&& (not a) b)");
    assert_eq!(expr("~mask & flags"), "(& (bitnot mask) flags)");
    assert_eq!(expr("&x == p"), "(== (addr x) p)");
    assert_eq!(expr("a * *p"), "(* a *p)");
    assert_eq!(expr("- -a"), "(neg (neg a))");
}

#[test]
fn comparison_does_not_chain() {
    assert_error(
        body_errors("=> a < b < c"),
        "comparison operators do not chain",
    );
}

#[test]
fn so_expressions() {
    assert_eq!(expr("so(a > b) a else b"), "(so (> a b) a b)");
    assert_eq!(
        expr("so(x < lo) lo else so(x > hi) hi else x"),
        "(so (< x lo) lo (so (> x hi) hi x))"
    );
    assert_eq!(expr("(so(a > b) a else b) + 1"), "(+ (so (> a b) a b) 1)");
    // The else branch extends as far as it can.
    assert_eq!(expr("so(c) a else b + 1"), "(so c a (+ b 1))");
}

#[test]
fn so_errors() {
    assert_error(
        body_errors("=> 1 + so(c) a else b"),
        "must be parenthesized",
    );
    assert_error(body_errors("=> so(c) a"), "expected `else`");
    assert_error(
        body_errors("=> so(c) #a := 1 else 2"),
        "assignment is not allowed inside a `so` branch",
    );
}

#[test]
fn assignment_is_right_associative() {
    assert_eq!(stmt("#a := #b := 1"), "(:= #a (:= #b 1))");
    assert_eq!(stmt("#count += 1"), "(+= #count 1)");
    assert_eq!(stmt("#x +%= 1"), "(+%= #x 1)");
    assert_eq!(stmt("#x <<= 2"), "(<<= #x 2)");
}

#[test]
fn literals() {
    assert_eq!(expr("0xFF"), "0xFF");
    assert_eq!(expr("1_000"), "1000");
    assert_eq!(expr("2.5"), "2.5");
    assert_eq!(expr("'x'"), "'x'");
    assert_eq!(expr("\"hi\\n\""), "\"hi\\n\"");
    assert_eq!(expr("true"), "true");
    assert_eq!(expr("done"), "done");
    assert_eq!(expr("[1, 2, 3]"), "(array 1 2 3)");
    assert_eq!(expr("(1, 2.5)"), "(tuple 1 2.5)");
}

// ============================================================ casts (§6.5–6.7)

#[test]
fn primitive_casts() {
    assert_eq!(expr("(i64) 22"), "(cast i64 22)");
    assert_eq!(expr("(i8)% big"), "(cast% i8 big)");
    assert_eq!(expr("(i8)| big"), "(cast| i8 big)");
    assert_eq!(expr("(i8)^ big"), "(cast^ i8 big)");
    assert_eq!(expr("(i8) -128"), "(cast i8 (neg 128))");
    assert_eq!(expr("(i32)(-1)"), "(cast i32 (neg 1))");
    assert_eq!(expr("(q16.16) 1"), "(cast q16.16 1)");
    assert_eq!(expr("(u24) x.y"), "(cast u24 (. x y))");
}

#[test]
fn casts_bind_at_unary_level() {
    assert_eq!(expr("(i64) a + b"), "(+ (cast i64 a) b)");
}

#[test]
fn cast_blocks() {
    assert_eq!(
        expr("(i64) {\n    a1 +\n    a2 +\n    a3\n}"),
        "(cast-block i64 (+ (+ a1 a2) a3))"
    );
    assert_eq!(expr("(i64)% { a * b }"), "(cast-block% i64 (* a b))");
}

#[test]
fn tuple_type_is_itself_the_cast() {
    assert_eq!(
        expr("(i32, f32) (1000, 1.25)"),
        "(cast (tuple-type i32 f32) (tuple 1000 1.25))"
    );
}

#[test]
fn parenthesized_expressions_are_not_casts() {
    assert_eq!(expr("(a + b) * c"), "(* (+ a b) c)");
    assert_eq!(expr("(x)"), "x");
    assert_eq!(expr("(width) - 1"), "(- width 1)");
    assert_eq!(expr("(a, b)"), "(tuple a b)");
}

#[test]
fn a_type_name_in_parentheses_is_always_a_cast() {
    let prelude = "word Meters { one }\ntype unary: f(i64) -> i64\nstruct Mask { v: 0 }";
    assert_eq!(expr_with(prelude, "(Meters) x"), "(cast Meters x)");
    assert_eq!(
        expr_with(prelude, "(Meters) { a }"),
        "(cast-block Meters a)"
    );
    // What the name is decides, not the token after `)` (§6.7).
    assert_eq!(expr_with(prelude, "(Meters) -1"), "(cast Meters (neg 1))");
    assert_eq!(expr_with(prelude, "(Meters) - 1"), "(cast Meters (neg 1))");
    assert_eq!(expr_with(prelude, "(Mask) &x"), "(cast Mask (addr x))");
    assert_eq!(
        expr_with(prelude, "(Meters, i32) (x, 1)"),
        "(cast (tuple-type Meters i32) (tuple x 1))"
    );
    assert_eq!(
        expr_with(prelude, "(unary) \\(x) { => 2 * x }"),
        "(cast unary (lambda (x) pure (block (return (* 2 x)))))"
    );
}

#[test]
fn a_cast_needs_an_operand() {
    let prelude = "word Meters { one }";
    let errs = errors(&format!("{prelude}\nt f() -> effect {{\n=> (Meters)\n}}"));
    assert_error(errs, "a cast needs a value after it");
    assert_error(
        body_errors("=> (i32) == 1"),
        "a cast needs a value after it",
    );
}

#[test]
fn values_may_not_be_named_like_types() {
    let prelude = "word Meters { one }\nstruct Shape.Circle { r: 1.0 }\n";
    let src = |body: &str| format!("{prelude}{body}\n");
    assert_error(
        errors(&src("t f() -> effect {\n as Meters: 5\n}")),
        "`Meters` is the name of a type in this project",
    );
    assert_error(
        errors(&src("t f(Meters: i32) -> effect { => done }")),
        "`Meters` is the name of a type",
    );
    assert_error(
        errors(&src("Meters f() -> effect { => done }")),
        "`Meters` is the name of a type",
    );
    assert_error(
        errors(&src("struct P { Meters: 1 }")),
        "`Meters` is the name of a type",
    );
    assert_error(
        errors(&src("word W { Meters }")),
        "`Meters` is the name of a type",
    );
    assert_error(
        errors(&src(
            "type u: f(i64) -> i64\nt f() -> effect {\n as g: (u) \\(Meters) { => 1 }\n}",
        )),
        "`Meters` is the name of a type",
    );
    assert_error(
        errors(&src("as i32: 5")),
        "`i32` is the name of a primitive type",
    );
    assert_error(
        errors(&src("as q16.16: 5")),
        "`q16.16` is the name of a primitive type",
    );
    // A sigil makes a different name, and union variants are qualified.
    clean(&src("as #Meters: 5\nas Circle: 1\nas width: 2"));
    // A subtraction on an ordinary variable is still a subtraction.
    assert_eq!(expr("(width) - 1"), "(- width 1)");
}

#[test]
fn type_names_declared_later_still_count() {
    let src = "t f() -> effect {\n => (Later) x\n}\nstruct Later { v: 0 }\n";
    let file = clean(src);
    assert_eq!(
        sexp::item(&file.items[0]).flat(),
        "(fn t (params) pure (ret effect) (block (return (cast Later x))))"
    );
}

#[test]
fn star_after_a_cast_is_explained() {
    assert_error(
        body_errors("=> (i64) *p"),
        "`*` after a cast is read as multiplication",
    );
}

// ============================================================ places and calls

#[test]
fn places() {
    assert_eq!(expr("a[]"), "(len a)");
    assert_eq!(expr("*q[]"), "(len *q)");
    assert_eq!(expr("*q[0]"), "(index *q 0)");
    assert_eq!(expr("*p"), "*p");
    assert_eq!(expr("t[0]"), "(index t 0)");
    assert_eq!(expr("linda.#age"), "(. linda #age)");
    assert_eq!(expr("Light.red"), "(. Light red)");
}

#[test]
fn calls() {
    assert_eq!(expr("add(1, 2)"), "(call add 1 2)");
    assert_eq!(expr("add(1, b: 2)"), "(call add 1 (arg b 2))");
    assert_eq!(
        expr("Person(5.5, #age: 12)"),
        "(call Person 5.5 (arg #age 12))"
    );
    assert_eq!(expr("bump(ref #n)"), "(call bump (ref #n))");
    assert_eq!(expr("tally#(3)"), "(call tally# 3)");
    assert_eq!(expr("#h#(1)"), "(call #h# 1)");
    assert_eq!(stmt("#sam.bump#()"), "(call (. #sam bump#))");
    assert_eq!(
        stmt("#grid[#i].bump#()"),
        "(call (. (index #grid #i) bump#))"
    );
    assert_eq!(
        expr("shapes.Shape.Circle(3.0)"),
        "(call (. (. shapes Shape) Circle) 3.0)"
    );
}

#[test]
fn call_errors() {
    assert_error(
        body_errors("=> make().area()"),
        "call chaining is not supported",
    );
    assert_error(
        body_errors("g(a: 1, 2)"),
        "positional arguments must come before named ones",
    );
    assert_error(body_errors("bump(ref 5)"), "`ref` needs storage");
    assert_error(body_errors("g(1,)"), "trailing comma");
}

#[test]
fn lambdas() {
    assert_eq!(
        expr_with(
            "type unary: f(i64) -> i64",
            "(unary) \\(x)[#total] { #total += x; => x }"
        ),
        "(cast unary (lambda (x) (captures #total) (block (+= #total x) (return x))))"
    );
}

// ============================================================ statements

#[test]
fn bindings() {
    assert_eq!(stmt("as count: 0"), "(as count 0)");
    assert_eq!(stmt("as #count: 0"), "(as #count 0)");
    assert_eq!(stmt("as a: 1, as b: 2"), "(bindings (as a 1) (as b 2))");
    assert_eq!(
        stmt("as #grid[4, i32]: [5, 6, 7, 8]"),
        "(as #grid (array-type 4 i32) (array 5 6 7 8))"
    );
    assert_eq!(stmt("as *p[i32]: &value"), "(as *p (ptr i32) (addr value))");
    assert_eq!(
        stmt("as *q[n, i32]: &array"),
        "(as *q (fat-ptr n i32) (addr array))"
    );
    assert_eq!(
        stmt("as *r[10, i32]: &array"),
        "(as *r (fat-ptr 10 i32) (addr array))"
    );
    assert_eq!(
        stmts_with(
            "type unary: f(i64) -> i64",
            "as #h#: (unary) \\(x) { => x }"
        ),
        vec!["(as #h# (cast unary (lambda (x) pure (block (return x)))))"]
    );
}

#[test]
fn binding_errors() {
    assert_error(body_errors("as x"), "binding `x` needs an initializer");
    assert_error(body_errors("as a: 1, b: 2"), "expected `as` after `,`");
    assert_error(
        body_errors("as #g[4]: [1, 2, 3, 4]"),
        "array declarators have two slots",
    );
}

#[test]
fn conditionals_and_loops() {
    assert_eq!(
        stmt("if (n > 10) { #n := 10 } else if (n < 0) { #n := 0 } else { #n += 1 }"),
        "(if ((> n 10) (block (:= #n 10))) ((< n 0) (block (:= #n 0))) (else (block (+= #n 1))))"
    );
    assert_eq!(
        stmt("while (#i > 0) { #i -= 1 }"),
        "(while (> #i 0) (block (-= #i 1)))"
    );
    assert_eq!(
        stmt("do { #n += 1 } until (#n == 3)"),
        "(do-until (block (+= #n 1)) (== #n 3))"
    );
    assert_error(
        body_errors("if n > 1 { }"),
        "conditions are always parenthesized",
    );
}

#[test]
fn switch_statement() {
    let body = "switch (l) {\n    case Light.red { #s += 1 } then\n    case 3, 4 { #s += 5 } break\n    default {}\n}";
    assert_eq!(
        stmt(body),
        "(switch l (case ((. Light red)) (block (+= #s 1)) then) (case (3 4) (block (+= #s 5)) break) (default (block)))"
    );
}

#[test]
fn switch_errors() {
    assert_error(
        body_errors("switch (x) {\n case 1 { } break\n}"),
        "has no `default` clause",
    );
    assert_error(
        body_errors("switch (x) {\n case 1 { }\n default {}\n}"),
        "must end with `break` or `then`",
    );
    assert_error(
        body_errors("switch (x) {\n case 1 { }\n break\n default {}\n}"),
        "`break` must be on the same line",
    );
    assert_error(
        body_errors("switch (x) {\n default {}\n case 1 { } break\n}"),
        "`default` must be the last clause",
    );
}

#[test]
fn when_statement() {
    let body = "when (state) {\n    is red { }\n    is yellow where (hour > 20) { }\n    is >= 10 { }\n    is (_, fail) { }\n    is Shape.Circle { }\n    is -1 { }\n    is _ { }\n}";
    assert_eq!(
        stmt(body),
        "(when state (is red (block)) (is yellow (where (> hour 20)) (block)) (is (>= 10) (block)) \
         (is (tuple-pattern _ fail) (block)) (is Shape.Circle (block)) (is (neg 1) (block)) (is _ (block)))"
    );
    assert_error(
        body_errors("when (x) {\n is _ where (a) where (b) { }\n}"),
        "at most one guard",
    );
}

#[test]
fn scripts_labels_and_jumps() {
    assert_eq!(
        stmts(
            "as #steps: 0\nscript bump { #steps += 1 }\ndo bump\nleap out\nredo\nbreak\nout:\n=> done"
        ),
        vec![
            "(as #steps 0)",
            "(script bump (block (+= #steps 1)))",
            "(do bump)",
            "(leap out)",
            "redo",
            "break",
            "(label out)",
            "(return done)",
        ]
    );
}

#[test]
fn return_requires_a_value() {
    assert_error(body_errors("=>\n"), "[N8] return requires a value");
    assert_error(body_errors("{ => }"), "[N8] return requires a value");
}

#[test]
fn same_line_statements_need_a_semicolon() {
    assert_eq!(
        stmts("#age += 1; => done"),
        vec!["(+= #age 1)", "(return done)"]
    );
    assert_error(
        body_errors("#age += 1  => done"),
        "expected a newline or `;` after an expression",
    );
}

#[test]
fn nested_definitions() {
    assert_eq!(
        stmt("helper f(x: i64)[n] -> i64 { => x + n }"),
        "(fn helper (params (param x i64)) (captures n) (ret i64) (block (return (+ x n))))"
    );
    assert_error(
        body_errors("share helper f() -> i64 { => 1 }"),
        "nested definitions cannot be `share`",
    );
}

// ============================================================ items

#[test]
fn functions() {
    assert_eq!(
        item("add f(a: i32, b: i32) -> i32 { => a + b }"),
        "(fn add (params (param a i32) (param b i32)) pure (ret i32) (block (return (+ a b))))"
    );
    assert_eq!(
        item("tally# f(x: i32)[#total] -> effect { #total += x; => done }"),
        "(fn tally# (params (param x i32)) (captures #total) (ret effect) (block (+= #total x) (return done)))"
    );
    assert_eq!(
        item("share prerun square f(x: i32) -> i32 { => x * x }"),
        "(share (fn square prerun (params (param x i32)) pure (ret i32) (block (return (* x x)))))"
    );
    assert_eq!(
        item("g f()[_] -> effect { => done }"),
        "(fn g (params) (captures _) (ret effect) (block (return done)))"
    );
    assert_eq!(
        item("h f()[linda.#age] -> effect { => done }"),
        "(fn h (params) (captures linda.#age) (ret effect) (block (return done)))"
    );
}

#[test]
fn parameter_forms() {
    assert_eq!(
        item(
            "p f(x: i32, #y: i32, ref z: P, ref #w: P, copy c: P, a[10, i32], *t[i32], *q[n, i32], g#: sink) -> effect { => done }"
        ),
        "(fn p (params (param x i32) (param #y i32) (param ref z P) (param ref #w P) (param copy c P) \
         (param a (array-type 10 i32)) (param *t (ptr i32)) (param *q (fat-ptr n i32)) (param g# sink)) \
         pure (ret effect) (block (return done)))"
    );
}

#[test]
fn function_errors() {
    assert_error(
        errors("main f() { => 0 }"),
        "missing return type for `main`",
    );
    assert_error(
        errors("cycle share f g() -> u8 { => 0 }"),
        "must appear in the order",
    );
    assert_error(
        errors("print(1)"),
        "statements are only allowed inside functions",
    );
}

#[test]
fn structs() {
    let src = "share struct Person {\n    height: -1.0,\n    #age: 1,\n    #data[2, i32]: [0, 0],\n    greet f()[height] -> f64 { => height }\n    bump# f()[#age] -> effect { #age += 1; => done }\n}";
    assert_eq!(
        item(src),
        "(share (struct Person (field height (neg 1.0)) (field #age 1) (field #data (array-type 2 i32) (array 0 0)) \
         (fn greet (params) (captures height) (ret f64) (block (return height))) \
         (fn bump# (params) (captures #age) (ret effect) (block (+= #age 1) (return done)))))"
    );
    assert_eq!(
        item("struct Shape.Circle: Drawable { radius: 1.0 }"),
        "(struct Shape.Circle (implements Drawable) (field radius 1.0))"
    );
}

#[test]
fn struct_errors() {
    assert_error(
        errors("struct P {\n a: 1\n b: 2\n}"),
        "expected `,` after this field",
    );
    assert_error(
        errors("struct P { a }"),
        "every field needs a constant default",
    );
}

#[test]
fn words() {
    assert_eq!(
        item("word Light { red, yellow, green }"),
        "(word Light (variant red) (variant yellow) (variant green))"
    );
    assert_eq!(
        item("word Err(u16, start: 256) { First, Second }"),
        "(word Err (repr u16 (start 256)) (variant First) (variant Second))"
    );
    assert_eq!(
        item("share word Code(u8) {\n    ok: 0,\n    bad: 1,\n}"),
        "(share (word Code (repr u8) (variant ok 0) (variant bad 1)))"
    );
}

#[test]
fn unions_and_interfaces() {
    assert_eq!(
        item("union Shape(Circle, Square) : Drawable {\n    type area: f() -> f64\n}"),
        "(union Shape (variants Circle Square) (implements Drawable) (contract area (fn-type () f64)))"
    );
    assert_eq!(
        item("interface Solid: Drawable {\n    type volume: f()[] -> f64\n}"),
        "(interface Solid (extends Drawable) (contract volume (fn-type () (bound) f64)))"
    );
}

#[test]
fn type_aliases_and_function_types() {
    assert_eq!(
        items(
            "type unary: f(i64) -> i64\ntype pure_op: f(i64)[!] -> i64\ntype bar: i16\ntype ptrs: (*[i32], *[n, u8])"
        ),
        vec![
            "(type unary (fn-type (i64) i64))",
            "(type pure_op (fn-type (i64) [!] i64))",
            "(type bar i16)",
            "(type ptrs (tuple-type (ptr i32) (fat-ptr n u8)))",
        ]
    );
}

#[test]
fn imports_and_globals() {
    assert_eq!(
        items(
            "import log\nimport shapes\n\nshare as limit: 100\nas #total: 0\n\nmain f() -> u8 { => 0 }"
        ),
        vec![
            "(import log)",
            "(import shapes)",
            "(share (as limit 100))",
            "(as #total 0)",
            "(fn main (params) pure (ret u8) (block (return 0)))",
        ]
    );
    assert_error(
        errors("main f() -> u8 { => 0 }\nimport log"),
        "imports must come before all definitions",
    );
}

// ============================================================ recovery

#[test]
fn one_run_reports_independent_errors() {
    let errs = body_errors("as : 1\nas y 2\n#z := (1 +)\n=> done");
    assert_eq!(errs.len(), 3, "{errs:?}");
}

#[test]
fn recovery_keeps_later_items() {
    let (file, diags) = parse("broken f( -> u8 { }\nok f() -> u8 { => 0 }\n");
    assert!(!diags.is_empty());
    assert!(
        file.items
            .iter()
            .any(|i| matches!(&i.kind, ItemKind::Fn(d) if d.name.text == "ok"))
    );
}

#[test]
fn primitive_type_names() {
    for name in [
        "i8", "u16", "i24", "u128", "f32", "f64", "bool", "ascii", "q16.16", "q15",
    ] {
        assert!(is_primitive_type_name(name), "{name}");
    }
    for name in ["i7", "u0", "i136", "i08", "f16", "q", "q1.", "char", "int"] {
        assert!(!is_primitive_type_name(name), "{name}");
    }
}
