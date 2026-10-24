//! The abstract syntax tree produced by the parser.
//!
//! The AST mirrors the source closely: nothing is resolved or desugared yet.
//! Every node carries a [`Span`]. Names keep their sigil, because `x`, `#x`
//! and `*x` are distinct symbols (spec §2.2).

use crate::lexer::{ArithOp, Modifier};
use crate::source::Span;
use std::fmt;

// ============================================================ names

/// The sigil attached to a name. It is part of the name's identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Sigil {
    /// `x`
    None,
    /// `#x`: mutable storage.
    Mut,
    /// `x#`: a mutating function.
    MutFn,
    /// `#x#`: a rebindable binding holding a mutating function.
    MutMutFn,
    /// `*x`: a pointer.
    Ptr,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Name {
    pub sigil: Sigil,
    pub text: String,
    pub span: Span,
}

impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.sigil {
            Sigil::None => write!(f, "{}", self.text),
            Sigil::Mut => write!(f, "#{}", self.text),
            Sigil::MutFn => write!(f, "{}#", self.text),
            Sigil::MutMutFn => write!(f, "#{}#", self.text),
            Sigil::Ptr => write!(f, "*{}", self.text),
        }
    }
}

/// A dotted name made of plain identifiers: `Shape.Circle`, `log.write`.
#[derive(Debug, Clone, PartialEq)]
pub struct Path {
    pub segments: Vec<Name>,
    pub span: Span,
}

impl fmt::Display for Path {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, seg) in self.segments.iter().enumerate() {
            if i > 0 {
                f.write_str(".")?;
            }
            write!(f, "{seg}")?;
        }
        Ok(())
    }
}

// ============================================================ files and items

#[derive(Debug, Clone, PartialEq)]
pub struct File {
    pub imports: Vec<Import>,
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Import {
    pub path: Path,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub share: bool,
    pub kind: ItemKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ItemKind {
    Fn(FnDef),
    /// Top-level `as` bindings (globals).
    Bindings(Vec<Binding>),
    Struct(StructDef),
    Word(WordDef),
    Union(UnionDef),
    Interface(InterfaceDef),
    Alias(TypeAlias),
}

/// `name f(params)[captures] -> type { body }`. Also used for methods and
/// nested definitions.
#[derive(Debug, Clone, PartialEq)]
pub struct FnDef {
    pub prerun: bool,
    pub cycle: bool,
    /// `area` or `bump#`.
    pub name: Name,
    pub params: Vec<Param>,
    /// `None` means no capture group: the function is pure.
    pub captures: Option<Vec<Capture>>,
    pub ret: Type,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamMode {
    /// By copy (or an unobservable view, §9.4).
    Value,
    /// `ref`: a no-copy view; `ref #x` is a mutable reference.
    Ref,
    /// `copy`: a forced copy.
    Copy,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub mode: ParamMode,
    pub name: Name,
    pub shape: Shape,
    pub span: Span,
}

/// How a declared name is shaped. Shared by parameters, bindings and fields,
/// which all use the same declarator forms.
#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    /// `x: T` in a parameter. Bindings use `Inferred` instead.
    Typed(Type),
    /// `as x: value`: the type comes from the initializer.
    Inferred,
    /// `a[10, i32]`: an array; the length is exact in a binding, a minimum in a
    /// parameter.
    Array { len: Expr, elem: Type },
    /// `*p[T]`: a thin pointer.
    ThinPtr(Type),
    /// `*q[n, T]`: a fat pointer. `len` is a binder name or a minimum literal.
    FatPtr { len: Expr, elem: Type },
}

/// One entry in a capture group.
#[derive(Debug, Clone, PartialEq)]
pub enum Capture {
    /// `_`: inherit callees' captures.
    Wildcard(Span),
    /// `a`, `#b`, `linda.#age`.
    Path(Vec<Name>, Span),
}

/// The capture bound written in a function type (§4.5).
#[derive(Debug, Clone, PartialEq)]
pub enum CaptureBound {
    /// `[!]`: implementations must be pure.
    Pure,
    /// `[]` or `[a, #b]`.
    List(Vec<Capture>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct StructDef {
    /// `Person` or a union variant like `Shape.Circle`.
    pub name: Path,
    /// Interfaces named after `:` (an optional, checked hint).
    pub implements: Vec<Path>,
    pub fields: Vec<Field>,
    pub methods: Vec<FnDef>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub name: Name,
    /// `Inferred` or `Array`.
    pub shape: Shape,
    pub default: Expr,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WordDef {
    pub name: Name,
    /// `(u16, start: 256)`.
    pub repr: Option<WordRepr>,
    pub variants: Vec<WordVariant>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WordRepr {
    pub ty: Type,
    pub start: Option<Expr>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WordVariant {
    pub name: Name,
    pub value: Option<Expr>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UnionDef {
    pub name: Name,
    pub variants: Vec<Name>,
    pub implements: Vec<Path>,
    pub contracts: Vec<Contract>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InterfaceDef {
    pub name: Name,
    pub extends: Vec<Path>,
    pub contracts: Vec<Contract>,
}

/// `type area: f() -> f64` inside a union or interface.
#[derive(Debug, Clone, PartialEq)]
pub struct Contract {
    pub name: Name,
    pub ty: Type,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypeAlias {
    pub name: Name,
    pub ty: Type,
}

// ============================================================ types

#[derive(Debug, Clone, PartialEq)]
pub struct Type {
    pub kind: TypeKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypeKind {
    /// A primitive (`i32`, `q16.16`) or a user type (`Person`, `Shape.Circle`).
    /// Which one is decided by name resolution.
    Named(Path),
    /// `effect` (a keyword, so it gets its own variant).
    Effect,
    Tuple(Vec<Type>),
    /// `[n, T]`
    Array {
        len: Box<Expr>,
        elem: Box<Type>,
    },
    /// `*[T]`
    ThinPtr(Box<Type>),
    /// `*[n, T]`
    FatPtr {
        len: Box<Expr>,
        elem: Box<Type>,
    },
    /// `f(i64)[bound] -> i64`. `bound: None` is unconstrained.
    Fn {
        params: Vec<Type>,
        bound: Option<CaptureBound>,
        ret: Box<Type>,
    },
}

// ============================================================ statements

#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
}

/// `as target: value`
#[derive(Debug, Clone, PartialEq)]
pub struct Binding {
    pub name: Name,
    /// `Inferred`, `Array`, `ThinPtr` or `FatPtr`.
    pub shape: Shape,
    pub value: Expr,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StmtKind {
    /// `as a: 1, as b: 2`
    Bindings(Vec<Binding>),
    /// A nested function definition.
    Fn(Box<FnDef>),
    Block(Block),
    /// `if (c) {..} else if (d) {..} else {..}`
    If {
        arms: Vec<(Expr, Block)>,
        otherwise: Option<Block>,
    },
    While {
        cond: Expr,
        body: Block,
    },
    DoUntil {
        body: Block,
        cond: Expr,
    },
    Switch {
        subject: Expr,
        cases: Vec<Case>,
        default: Block,
    },
    When {
        subject: Expr,
        arms: Vec<WhenArm>,
    },
    Script {
        name: Name,
        body: Block,
    },
    /// `do bump`: run a script in place.
    Do(Name),
    Label(Name),
    Leap(Name),
    Break,
    Redo,
    /// `=> value`
    Return(Expr),
    Expr(Expr),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaseExit {
    Break,
    Then,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Case {
    pub values: Vec<Expr>,
    pub body: Block,
    pub exit: CaseExit,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WhenArm {
    pub pattern: Pattern,
    pub guard: Option<Expr>,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Pattern {
    pub kind: PatternKind,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelOp {
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
}

impl RelOp {
    pub fn as_str(self) -> &'static str {
        match self {
            RelOp::Eq => "==",
            RelOp::Ne => "!=",
            RelOp::Lt => "<",
            RelOp::Gt => ">",
            RelOp::Le => "<=",
            RelOp::Ge => ">=",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum PatternKind {
    /// `_`
    Wildcard,
    /// `>= 1`: the right side must be a compile-time constant (checked in sema).
    Relational(RelOp, Expr),
    /// A literal, `done` or `fail`.
    Literal(Expr),
    /// `red`, `Light.red`, `Circle`, `Shape.Circle`. Whether this names a
    /// value, a word variant, a union variant or a type is decided in sema.
    Path(Path),
    Tuple(Vec<Pattern>),
    /// Type syntax that cannot be a path: `[4, i32]`, `*[i32]`, `f() -> i32`.
    Type(Type),
}

// ============================================================ expressions

#[derive(Debug, Clone, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    /// `+x`
    Plus,
    /// `-x`
    Neg,
    /// `!x`
    Not,
    /// `~x`
    BitNot,
    /// `&x`
    AddrOf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Arith(ArithOp, Modifier),
    Rel(RelOp),
    And,
    Or,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
}

impl BinOp {
    pub fn to_text(self) -> String {
        match self {
            BinOp::Arith(op, m) => format!("{}{}", op.as_str(), m.as_str()),
            BinOp::Rel(r) => r.as_str().to_string(),
            BinOp::And => "&&".into(),
            BinOp::Or => "||".into(),
            BinOp::BitAnd => "&".into(),
            BinOp::BitOr => "|".into(),
            BinOp::BitXor => "^".into(),
            BinOp::Shl => "<<".into(),
            BinOp::Shr => ">>".into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignOp {
    /// `:=`
    Set,
    /// `+=`, `+%=`, ...
    Arith(ArithOp, Modifier),
    Shl,
    Shr,
    BitAnd,
    BitOr,
    BitXor,
}

impl AssignOp {
    pub fn to_text(self) -> String {
        match self {
            AssignOp::Set => ":=".into(),
            AssignOp::Arith(op, m) => format!("{}{}=", op.as_str(), m.as_str()),
            AssignOp::Shl => "<<=".into(),
            AssignOp::Shr => ">>=".into(),
            AssignOp::BitAnd => "&=".into(),
            AssignOp::BitOr => "|=".into(),
            AssignOp::BitXor => "^=".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Arg {
    /// `b: 2` or `#age: 12`.
    pub name: Option<Name>,
    /// `ref #n`
    pub is_ref: bool,
    pub value: Expr,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Lambda {
    pub params: Vec<Name>,
    pub captures: Option<Vec<Capture>>,
    pub body: Block,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExprKind {
    /// Digits as text; range-checked against the target type in sema.
    Int {
        digits: String,
        radix: u32,
    },
    Float(String),
    Ascii(u8),
    Str(Vec<u8>),
    Bool(bool),
    /// `done` (true) or `fail` (false).
    Effect(bool),
    /// Any name: `x`, `#x`, `x#`, `#x#`, `*p`.
    Name(Name),
    /// `base.field`, `base.#field`, `base.method#`.
    Field {
        base: Box<Expr>,
        field: Name,
    },
    /// `base[index]`
    Index {
        base: Box<Expr>,
        index: Box<Expr>,
    },
    /// `base[]`: the length.
    Len(Box<Expr>),
    Call {
        callee: Box<Expr>,
        args: Vec<Arg>,
    },
    Unary {
        op: UnaryOp,
        operand: Box<Expr>,
    },
    Binary {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    Assign {
        op: AssignOp,
        target: Box<Expr>,
        value: Box<Expr>,
    },
    /// `(T) x`, `(T)% x`, and the tuple form `(i32, f32) (1, 2.5)`.
    Cast {
        ty: Type,
        modifier: Modifier,
        operand: Box<Expr>,
    },
    /// `(T) { expr }`: casts the operands of every operator inside (§6.6).
    CastBlock {
        ty: Type,
        modifier: Modifier,
        body: Box<Expr>,
    },
    /// `so(cond) a else b`
    So {
        cond: Box<Expr>,
        then: Box<Expr>,
        otherwise: Box<Expr>,
    },
    Array(Vec<Expr>),
    Tuple(Vec<Expr>),
    Lambda(Box<Lambda>),
    /// `(as a: 1, as b: 2)`: the grammar's `synthesized` form. Its meaning is
    /// not described in the spec yet.
    Bindings(Vec<Binding>),
    /// Placeholder after a parse error, so later passes can keep going.
    Error,
}

impl Expr {
    /// A place: a name followed by field, index and length accesses (the
    /// grammar's `place` and `ptr_place`). Only places can be called (§7.3).
    pub fn is_place(&self) -> bool {
        match &self.kind {
            ExprKind::Name(_) => true,
            ExprKind::Field { base, .. } | ExprKind::Index { base, .. } => base.is_place(),
            ExprKind::Len(base) => base.is_place(),
            _ => false,
        }
    }

    pub fn is_literal(&self) -> bool {
        matches!(
            self.kind,
            ExprKind::Int { .. }
                | ExprKind::Float(_)
                | ExprKind::Ascii(_)
                | ExprKind::Str(_)
                | ExprKind::Bool(_)
                | ExprKind::Effect(_)
        )
    }
}
