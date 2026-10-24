//! Prints the AST as S-expressions, for `muc parse` and for tests.
//!
//! `(+ a (* b c))` makes precedence and structure visible at a glance, and a
//! flat one-line form keeps parser tests short. [`Sexp::pretty`] breaks long
//! lists across lines.

use crate::ast::*;
use crate::lexer::{Modifier, TokenKind};

#[derive(Debug, Clone, PartialEq)]
pub enum Sexp {
    Atom(String),
    List(Vec<Sexp>),
}

fn atom(s: impl Into<String>) -> Sexp {
    Sexp::Atom(s.into())
}

fn list(items: Vec<Sexp>) -> Sexp {
    Sexp::List(items)
}

fn tagged(tag: &str, mut rest: Vec<Sexp>) -> Sexp {
    rest.insert(0, atom(tag));
    Sexp::List(rest)
}

impl Sexp {
    /// The whole expression on one line.
    pub fn flat(&self) -> String {
        match self {
            Sexp::Atom(s) => s.clone(),
            Sexp::List(items) => {
                let inner: Vec<String> = items.iter().map(Sexp::flat).collect();
                format!("({})", inner.join(" "))
            }
        }
    }

    /// Lists that do not fit in `width` columns put each child on its own line.
    pub fn pretty(&self, width: usize) -> String {
        let mut out = String::new();
        self.write_pretty(&mut out, 0, width);
        out
    }

    fn write_pretty(&self, out: &mut String, indent: usize, width: usize) {
        let flat = self.flat();
        match self {
            Sexp::List(items) if indent + flat.len() > width && items.len() > 1 => {
                // Keep the tag and any atoms right after it on the first line:
                // `(fn math`, `(switch l`.
                let head = 1 + items[1..]
                    .iter()
                    .take_while(|i| matches!(i, Sexp::Atom(_)))
                    .count();
                let head = head.min(items.len() - 1).max(1);
                out.push('(');
                let head_text: Vec<String> = items[..head].iter().map(Sexp::flat).collect();
                out.push_str(&head_text.join(" "));
                for child in &items[head..] {
                    out.push('\n');
                    out.push_str(&" ".repeat(indent + 2));
                    child.write_pretty(out, indent + 2, width);
                }
                out.push(')');
            }
            _ => out.push_str(&flat),
        }
    }
}

// ============================================================ items

pub fn file(f: &File) -> Vec<Sexp> {
    let mut out: Vec<Sexp> = f
        .imports
        .iter()
        .map(|i| tagged("import", vec![atom(i.path.to_string())]))
        .collect();
    out.extend(f.items.iter().map(item));
    out
}

pub fn item(item: &Item) -> Sexp {
    let inner = match &item.kind {
        ItemKind::Fn(def) => fn_def(def),
        ItemKind::Bindings(bs) => bindings(bs),
        ItemKind::Struct(s) => struct_def(s),
        ItemKind::Word(w) => word_def(w),
        ItemKind::Union(u) => union_def(u),
        ItemKind::Interface(i) => interface_def(i),
        ItemKind::Alias(a) => tagged("type", vec![atom(a.name.to_string()), ty(&a.ty)]),
    };
    if item.share {
        tagged("share", vec![inner])
    } else {
        inner
    }
}

pub fn fn_def(def: &FnDef) -> Sexp {
    let mut parts = vec![atom(def.name.to_string())];
    if def.prerun {
        parts.push(atom("prerun"));
    }
    if def.cycle {
        parts.push(atom("cycle"));
    }
    parts.push(tagged("params", def.params.iter().map(param).collect()));
    parts.push(captures(&def.captures));
    parts.push(tagged("ret", vec![ty(&def.ret)]));
    parts.push(block(&def.body));
    tagged("fn", parts)
}

fn param(p: &Param) -> Sexp {
    let mut parts = Vec::new();
    match p.mode {
        ParamMode::Value => {}
        ParamMode::Ref => parts.push(atom("ref")),
        ParamMode::Copy => parts.push(atom("copy")),
    }
    parts.push(atom(p.name.to_string()));
    parts.extend(shape(&p.shape));
    tagged("param", parts)
}

fn shape(s: &Shape) -> Option<Sexp> {
    match s {
        Shape::Inferred => None,
        Shape::Typed(t) => Some(ty(t)),
        Shape::Array { len, elem } => Some(tagged("array-type", vec![expr(len), ty(elem)])),
        Shape::ThinPtr(t) => Some(tagged("ptr", vec![ty(t)])),
        Shape::FatPtr { len, elem } => Some(tagged("fat-ptr", vec![expr(len), ty(elem)])),
    }
}

fn capture(c: &Capture) -> Sexp {
    match c {
        Capture::Wildcard(_) => atom("_"),
        Capture::Path(names, _) => atom(
            names
                .iter()
                .map(|n| n.to_string())
                .collect::<Vec<_>>()
                .join("."),
        ),
    }
}

fn captures(c: &Option<Vec<Capture>>) -> Sexp {
    match c {
        None => atom("pure"),
        Some(list) => tagged("captures", list.iter().map(capture).collect()),
    }
}

fn struct_def(s: &StructDef) -> Sexp {
    let mut parts = vec![atom(s.name.to_string())];
    if !s.implements.is_empty() {
        parts.push(tagged(
            "implements",
            s.implements.iter().map(|p| atom(p.to_string())).collect(),
        ));
    }
    for f in &s.fields {
        let mut field = vec![atom(f.name.to_string())];
        field.extend(shape(&f.shape));
        field.push(expr(&f.default));
        parts.push(tagged("field", field));
    }
    parts.extend(s.methods.iter().map(fn_def));
    tagged("struct", parts)
}

fn word_def(w: &WordDef) -> Sexp {
    let mut parts = vec![atom(w.name.to_string())];
    if let Some(repr) = &w.repr {
        let mut r = vec![ty(&repr.ty)];
        if let Some(start) = &repr.start {
            r.push(tagged("start", vec![expr(start)]));
        }
        parts.push(tagged("repr", r));
    }
    for v in &w.variants {
        let mut variant = vec![atom(v.name.to_string())];
        if let Some(value) = &v.value {
            variant.push(expr(value));
        }
        parts.push(tagged("variant", variant));
    }
    tagged("word", parts)
}

fn contract(c: &Contract) -> Sexp {
    tagged("contract", vec![atom(c.name.to_string()), ty(&c.ty)])
}

fn union_def(u: &UnionDef) -> Sexp {
    let mut parts = vec![
        atom(u.name.to_string()),
        tagged(
            "variants",
            u.variants.iter().map(|v| atom(v.to_string())).collect(),
        ),
    ];
    if !u.implements.is_empty() {
        parts.push(tagged(
            "implements",
            u.implements.iter().map(|p| atom(p.to_string())).collect(),
        ));
    }
    parts.extend(u.contracts.iter().map(contract));
    tagged("union", parts)
}

fn interface_def(i: &InterfaceDef) -> Sexp {
    let mut parts = vec![atom(i.name.to_string())];
    if !i.extends.is_empty() {
        parts.push(tagged(
            "extends",
            i.extends.iter().map(|p| atom(p.to_string())).collect(),
        ));
    }
    parts.extend(i.contracts.iter().map(contract));
    tagged("interface", parts)
}

// ============================================================ types

pub fn ty(t: &Type) -> Sexp {
    match &t.kind {
        TypeKind::Named(path) => atom(path.to_string()),
        TypeKind::Effect => atom("effect"),
        TypeKind::Tuple(elems) => tagged("tuple-type", elems.iter().map(ty).collect()),
        TypeKind::Array { len, elem } => tagged("array-type", vec![expr(len), ty(elem)]),
        TypeKind::ThinPtr(t) => tagged("ptr", vec![ty(t)]),
        TypeKind::FatPtr { len, elem } => tagged("fat-ptr", vec![expr(len), ty(elem)]),
        TypeKind::Fn { params, bound, ret } => {
            let mut parts = vec![list(params.iter().map(ty).collect())];
            match bound {
                None => {}
                Some(CaptureBound::Pure) => parts.push(atom("[!]")),
                Some(CaptureBound::List(caps)) => {
                    parts.push(tagged("bound", caps.iter().map(capture).collect()))
                }
            }
            parts.push(ty(ret));
            tagged("fn-type", parts)
        }
    }
}

// ============================================================ statements

pub fn block(b: &Block) -> Sexp {
    tagged("block", b.stmts.iter().map(stmt).collect())
}

fn binding(b: &Binding) -> Sexp {
    let mut parts = vec![atom(b.name.to_string())];
    parts.extend(shape(&b.shape));
    parts.push(expr(&b.value));
    tagged("as", parts)
}

fn bindings(bs: &[Binding]) -> Sexp {
    if bs.len() == 1 {
        binding(&bs[0])
    } else {
        tagged("bindings", bs.iter().map(binding).collect())
    }
}

pub fn stmt(s: &Stmt) -> Sexp {
    match &s.kind {
        StmtKind::Bindings(bs) => bindings(bs),
        StmtKind::Fn(def) => fn_def(def),
        StmtKind::Block(b) => block(b),
        StmtKind::If { arms, otherwise } => {
            let mut parts: Vec<Sexp> = arms
                .iter()
                .map(|(c, b)| list(vec![expr(c), block(b)]))
                .collect();
            if let Some(b) = otherwise {
                parts.push(tagged("else", vec![block(b)]));
            }
            tagged("if", parts)
        }
        StmtKind::While { cond, body } => tagged("while", vec![expr(cond), block(body)]),
        StmtKind::DoUntil { body, cond } => tagged("do-until", vec![block(body), expr(cond)]),
        StmtKind::Switch {
            subject,
            cases,
            default,
        } => {
            let mut parts = vec![expr(subject)];
            for c in cases {
                let exit = match c.exit {
                    CaseExit::Break => "break",
                    CaseExit::Then => "then",
                };
                parts.push(tagged(
                    "case",
                    vec![
                        list(c.values.iter().map(expr).collect()),
                        block(&c.body),
                        atom(exit),
                    ],
                ));
            }
            parts.push(tagged("default", vec![block(default)]));
            tagged("switch", parts)
        }
        StmtKind::When { subject, arms } => {
            let mut parts = vec![expr(subject)];
            for arm in arms {
                let mut a = vec![pattern(&arm.pattern)];
                if let Some(g) = &arm.guard {
                    a.push(tagged("where", vec![expr(g)]));
                }
                a.push(block(&arm.body));
                parts.push(tagged("is", a));
            }
            tagged("when", parts)
        }
        StmtKind::Script { name, body } => {
            tagged("script", vec![atom(name.to_string()), block(body)])
        }
        StmtKind::Do(name) => tagged("do", vec![atom(name.to_string())]),
        StmtKind::Label(name) => tagged("label", vec![atom(name.to_string())]),
        StmtKind::Leap(name) => tagged("leap", vec![atom(name.to_string())]),
        StmtKind::Break => atom("break"),
        StmtKind::Redo => atom("redo"),
        StmtKind::Return(e) => tagged("return", vec![expr(e)]),
        StmtKind::Expr(e) => expr(e),
    }
}

pub fn pattern(p: &Pattern) -> Sexp {
    match &p.kind {
        PatternKind::Wildcard => atom("_"),
        PatternKind::Relational(op, e) => tagged(op.as_str(), vec![expr(e)]),
        PatternKind::Literal(e) => expr(e),
        PatternKind::Path(path) => atom(path.to_string()),
        PatternKind::Tuple(items) => tagged("tuple-pattern", items.iter().map(pattern).collect()),
        PatternKind::Type(t) => tagged("type-pattern", vec![ty(t)]),
    }
}

// ============================================================ expressions

fn modifier_suffix(m: Modifier) -> &'static str {
    m.as_str()
}

pub fn expr(e: &Expr) -> Sexp {
    match &e.kind {
        ExprKind::Int { digits, radix } => atom(
            TokenKind::Int {
                digits: digits.clone(),
                radix: *radix,
            }
            .to_string(),
        ),
        ExprKind::Float(text) => atom(text.clone()),
        ExprKind::Ascii(b) => atom(TokenKind::Ascii(*b).to_string()),
        ExprKind::Str(bytes) => atom(TokenKind::Str(bytes.clone()).to_string()),
        ExprKind::Bool(b) => atom(if *b { "true" } else { "false" }),
        ExprKind::Effect(ok) => atom(if *ok { "done" } else { "fail" }),
        ExprKind::Name(n) => atom(n.to_string()),
        ExprKind::Field { base, field } => tagged(".", vec![expr(base), atom(field.to_string())]),
        ExprKind::Index { base, index } => tagged("index", vec![expr(base), expr(index)]),
        ExprKind::Len(base) => tagged("len", vec![expr(base)]),
        ExprKind::Call { callee, args } => {
            let mut parts = vec![expr(callee)];
            for a in args {
                let value = if a.is_ref {
                    tagged("ref", vec![expr(&a.value)])
                } else {
                    expr(&a.value)
                };
                parts.push(match &a.name {
                    Some(n) => tagged("arg", vec![atom(n.to_string()), value]),
                    None => value,
                });
            }
            tagged("call", parts)
        }
        ExprKind::Unary { op, operand } => {
            let tag = match op {
                UnaryOp::Plus => "pos",
                UnaryOp::Neg => "neg",
                UnaryOp::Not => "not",
                UnaryOp::BitNot => "bitnot",
                UnaryOp::AddrOf => "addr",
            };
            tagged(tag, vec![expr(operand)])
        }
        ExprKind::Binary { op, lhs, rhs } => tagged(&op.to_text(), vec![expr(lhs), expr(rhs)]),
        ExprKind::Assign { op, target, value } => {
            tagged(&op.to_text(), vec![expr(target), expr(value)])
        }
        ExprKind::Cast {
            ty: t,
            modifier,
            operand,
        } => tagged(
            &format!("cast{}", modifier_suffix(*modifier)),
            vec![ty(t), expr(operand)],
        ),
        ExprKind::CastBlock {
            ty: t,
            modifier,
            body,
        } => tagged(
            &format!("cast-block{}", modifier_suffix(*modifier)),
            vec![ty(t), expr(body)],
        ),
        ExprKind::So {
            cond,
            then,
            otherwise,
        } => tagged("so", vec![expr(cond), expr(then), expr(otherwise)]),
        ExprKind::Array(items) => tagged("array", items.iter().map(expr).collect()),
        ExprKind::Tuple(items) => tagged("tuple", items.iter().map(expr).collect()),
        ExprKind::Lambda(l) => tagged(
            "lambda",
            vec![
                list(l.params.iter().map(|p| atom(p.to_string())).collect()),
                captures(&l.captures),
                block(&l.body),
            ],
        ),
        ExprKind::Bindings(bs) => tagged("bindings", bs.iter().map(binding).collect()),
        ExprKind::Error => atom("<error>"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pretty_breaks_only_long_lists() {
        let s = tagged(
            "block",
            vec![tagged("as", vec![atom("x"), atom("1")]), atom("break")],
        );
        assert_eq!(s.pretty(80), "(block (as x 1) break)");
        assert_eq!(s.pretty(10), "(block\n  (as x 1)\n  break)");
    }
}
