//! Abstract Syntax Tree definitions for IEC 61131-3 Structured Text.
//!
//! This module defines the AST nodes produced by the parser and consumed
//! by the type checker and code generator.

use std::fmt;

/// Source location information for error reporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Span {
    /// Starting byte offset.
    pub start: usize,
    /// Ending byte offset.
    pub end: usize,
    /// Line number (1-based).
    pub line: usize,
    /// Column number (1-based).
    pub column: usize,
}

impl Span {
    /// Create a new span.
    pub fn new(start: usize, end: usize, line: usize, column: usize) -> Self {
        Self {
            start,
            end,
            line,
            column,
        }
    }

    /// Merge two spans into one covering both.
    pub fn merge(self, other: Self) -> Self {
        Self {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
            line: self.line.min(other.line),
            column: if self.line <= other.line {
                self.column
            } else {
                other.column
            },
        }
    }
}

/// A node with associated span information.
#[derive(Debug, Clone, PartialEq)]
pub struct Spanned<T> {
    /// The node value.
    pub node: T,
    /// Source location.
    pub span: Span,
}

impl<T> Spanned<T> {
    /// Create a new spanned node.
    pub fn new(node: T, span: Span) -> Self {
        Self { node, span }
    }
}

/// Top-level compilation unit.
#[derive(Debug, Clone, PartialEq)]
pub struct CompilationUnit {
    /// Program Organization Units (POUs).
    pub units: Vec<Spanned<ProgramUnit>>,
}

/// A Program Organization Unit (POU).
#[derive(Debug, Clone, PartialEq)]
pub enum ProgramUnit {
    /// PROGRAM ... END_PROGRAM
    Program(Program),
    /// FUNCTION_BLOCK ... END_FUNCTION_BLOCK
    FunctionBlock(FunctionBlock),
    /// FUNCTION ... END_FUNCTION
    Function(Function),
}

/// A PROGRAM declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    /// Program name.
    pub name: String,
    /// Variable declarations.
    pub variables: Vec<Spanned<VarBlock>>,
    /// Program body statements.
    pub body: Vec<Spanned<Statement>>,
}

/// A FUNCTION_BLOCK declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct FunctionBlock {
    /// Function block name.
    pub name: String,
    /// Variable declarations.
    pub variables: Vec<Spanned<VarBlock>>,
    /// Function block body statements.
    pub body: Vec<Spanned<Statement>>,
}

/// A FUNCTION declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct Function {
    /// Function name.
    pub name: String,
    /// Return type.
    pub return_type: DataType,
    /// Variable declarations (inputs and locals).
    pub variables: Vec<Spanned<VarBlock>>,
    /// Function body statements.
    pub body: Vec<Spanned<Statement>>,
}

/// Variable declaration block.
#[derive(Debug, Clone, PartialEq)]
pub struct VarBlock {
    /// Block type (VAR, VAR_INPUT, etc.).
    pub kind: VarBlockKind,
    /// Whether variables are RETAIN (battery-backed).
    pub retain: bool,
    /// Whether variables are CONSTANT.
    pub constant: bool,
    /// Variable declarations in this block.
    pub declarations: Vec<Spanned<VarDecl>>,
}

/// Types of variable blocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VarBlockKind {
    /// Local variables (VAR).
    Var,
    /// Input parameters (VAR_INPUT).
    Input,
    /// Output parameters (VAR_OUTPUT).
    Output,
    /// In/Out parameters (VAR_IN_OUT).
    InOut,
    /// External variables (VAR_EXTERNAL).
    External,
    /// Global variables (VAR_GLOBAL).
    Global,
    /// Temporary variables (VAR_TEMP).
    Temp,
}

impl fmt::Display for VarBlockKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VarBlockKind::Var => write!(f, "VAR"),
            VarBlockKind::Input => write!(f, "VAR_INPUT"),
            VarBlockKind::Output => write!(f, "VAR_OUTPUT"),
            VarBlockKind::InOut => write!(f, "VAR_IN_OUT"),
            VarBlockKind::External => write!(f, "VAR_EXTERNAL"),
            VarBlockKind::Global => write!(f, "VAR_GLOBAL"),
            VarBlockKind::Temp => write!(f, "VAR_TEMP"),
        }
    }
}

/// A single variable declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct VarDecl {
    /// Variable name.
    pub name: String,
    /// Variable data type.
    pub data_type: DataType,
    /// Optional initial value.
    pub initial_value: Option<Spanned<Expression>>,
    /// Optional direct address (%IX0.0, %QW1, etc.).
    pub address: Option<DirectAddress>,
}

/// Direct address for I/O mapping.
#[derive(Debug, Clone, PartialEq)]
pub struct DirectAddress {
    /// Address type: I (input), Q (output), M (memory).
    pub location: AddressLocation,
    /// Size: X (bit), B (byte), W (word), D (double word), L (long).
    pub size: AddressSize,
    /// Address indices (e.g., [0, 0] for %IX0.0).
    pub indices: Vec<u32>,
}

/// Address location type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressLocation {
    /// Input (%I).
    Input,
    /// Output (%Q).
    Output,
    /// Memory (%M).
    Memory,
}

/// Address size specifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressSize {
    /// Bit (X or none).
    Bit,
    /// Byte (B).
    Byte,
    /// Word (W) - 16 bits.
    Word,
    /// Double word (D) - 32 bits.
    DWord,
    /// Long word (L) - 64 bits.
    LWord,
}

/// IEC 61131-3 data types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataType {
    // Boolean
    /// BOOL - Boolean (1 bit).
    Bool,

    // Integer types
    /// SINT - Signed 8-bit integer.
    Sint,
    /// INT - Signed 16-bit integer.
    Int,
    /// DINT - Signed 32-bit integer.
    Dint,
    /// LINT - Signed 64-bit integer.
    Lint,
    /// USINT - Unsigned 8-bit integer.
    Usint,
    /// UINT - Unsigned 16-bit integer.
    Uint,
    /// UDINT - Unsigned 32-bit integer.
    Udint,
    /// ULINT - Unsigned 64-bit integer.
    Ulint,

    // Floating point
    /// REAL - 32-bit floating point.
    Real,
    /// LREAL - 64-bit floating point.
    Lreal,

    // Time types
    /// TIME - Duration.
    Time,
    /// DATE - Calendar date.
    Date,
    /// TIME_OF_DAY / TOD - Time of day.
    TimeOfDay,
    /// DATE_AND_TIME / DT - Date and time.
    DateTime,

    // Bit string types
    /// BYTE - 8 bits.
    Byte,
    /// WORD - 16 bits.
    Word,
    /// DWORD - 32 bits.
    Dword,
    /// LWORD - 64 bits.
    Lword,

    // String types
    /// STRING - Variable-length string.
    String(Option<usize>),
    /// WSTRING - Wide string.
    WString(Option<usize>),

    // Array type
    /// ARRAY [lo..hi] OF type.
    Array {
        /// Lower bound.
        lower: i64,
        /// Upper bound.
        upper: i64,
        /// Element type.
        element_type: Box<DataType>,
    },

    // User-defined types
    /// Reference to a named type (struct, enum, FB instance).
    Named(String),
}

impl fmt::Display for DataType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DataType::Bool => write!(f, "BOOL"),
            DataType::Sint => write!(f, "SINT"),
            DataType::Int => write!(f, "INT"),
            DataType::Dint => write!(f, "DINT"),
            DataType::Lint => write!(f, "LINT"),
            DataType::Usint => write!(f, "USINT"),
            DataType::Uint => write!(f, "UINT"),
            DataType::Udint => write!(f, "UDINT"),
            DataType::Ulint => write!(f, "ULINT"),
            DataType::Real => write!(f, "REAL"),
            DataType::Lreal => write!(f, "LREAL"),
            DataType::Time => write!(f, "TIME"),
            DataType::Date => write!(f, "DATE"),
            DataType::TimeOfDay => write!(f, "TIME_OF_DAY"),
            DataType::DateTime => write!(f, "DATE_AND_TIME"),
            DataType::Byte => write!(f, "BYTE"),
            DataType::Word => write!(f, "WORD"),
            DataType::Dword => write!(f, "DWORD"),
            DataType::Lword => write!(f, "LWORD"),
            DataType::String(None) => write!(f, "STRING"),
            DataType::String(Some(len)) => write!(f, "STRING[{len}]"),
            DataType::WString(None) => write!(f, "WSTRING"),
            DataType::WString(Some(len)) => write!(f, "WSTRING[{len}]"),
            DataType::Array {
                lower,
                upper,
                element_type,
            } => {
                write!(f, "ARRAY[{lower}..{upper}] OF {element_type}")
            }
            DataType::Named(name) => write!(f, "{name}"),
        }
    }
}

impl DataType {
    /// Get the size in bytes for this type (if known at compile time).
    pub fn size_bytes(&self) -> Option<usize> {
        match self {
            DataType::Bool => Some(1),
            DataType::Sint | DataType::Usint | DataType::Byte => Some(1),
            DataType::Int | DataType::Uint | DataType::Word => Some(2),
            DataType::Dint | DataType::Udint | DataType::Dword | DataType::Real => Some(4),
            DataType::Lint | DataType::Ulint | DataType::Lword | DataType::Lreal => Some(8),
            DataType::Time => Some(8), // i64 nanoseconds
            DataType::Date | DataType::TimeOfDay | DataType::DateTime => Some(8),
            DataType::String(Some(len)) => Some(len + 1),
            DataType::WString(Some(len)) => Some((len + 1) * 2),
            DataType::Array {
                lower,
                upper,
                element_type,
            } => {
                let count = (upper - lower + 1) as usize;
                element_type.size_bytes().map(|s| s * count)
            }
            _ => None,
        }
    }

    /// Check if this is a numeric type.
    pub fn is_numeric(&self) -> bool {
        matches!(
            self,
            DataType::Sint
                | DataType::Int
                | DataType::Dint
                | DataType::Lint
                | DataType::Usint
                | DataType::Uint
                | DataType::Udint
                | DataType::Ulint
                | DataType::Real
                | DataType::Lreal
        )
    }

    /// Check if this is an integer type.
    pub fn is_integer(&self) -> bool {
        matches!(
            self,
            DataType::Sint
                | DataType::Int
                | DataType::Dint
                | DataType::Lint
                | DataType::Usint
                | DataType::Uint
                | DataType::Udint
                | DataType::Ulint
        )
    }
}

/// A statement in the program body.
#[derive(Debug, Clone, PartialEq)]
pub enum Statement {
    /// Assignment: variable := expression.
    Assignment(Assignment),
    /// IF ... THEN ... ELSIF ... ELSE ... END_IF.
    If(IfStatement),
    /// CASE ... OF ... END_CASE.
    Case(CaseStatement),
    /// FOR ... TO ... BY ... DO ... END_FOR.
    For(ForStatement),
    /// WHILE ... DO ... END_WHILE.
    While(WhileStatement),
    /// REPEAT ... UNTIL ... END_REPEAT.
    Repeat(RepeatStatement),
    /// EXIT - break out of loop.
    Exit,
    /// CONTINUE - skip to next iteration.
    Continue,
    /// RETURN - return from function/program.
    Return(Option<Spanned<Expression>>),
    /// Function/FB call as statement.
    Call(CallStatement),
    /// Empty statement (;).
    Empty,
}

/// Assignment statement.
#[derive(Debug, Clone, PartialEq)]
pub struct Assignment {
    /// Target variable or array element.
    pub target: Spanned<Expression>,
    /// Value to assign.
    pub value: Spanned<Expression>,
}

/// IF statement with optional ELSIF and ELSE branches.
#[derive(Debug, Clone, PartialEq)]
pub struct IfStatement {
    /// IF condition.
    pub condition: Spanned<Expression>,
    /// THEN branch statements.
    pub then_branch: Vec<Spanned<Statement>>,
    /// ELSIF branches.
    pub elsif_branches: Vec<ElsifBranch>,
    /// Optional ELSE branch.
    pub else_branch: Option<Vec<Spanned<Statement>>>,
}

/// ELSIF branch.
#[derive(Debug, Clone, PartialEq)]
pub struct ElsifBranch {
    /// ELSIF condition.
    pub condition: Spanned<Expression>,
    /// ELSIF statements.
    pub statements: Vec<Spanned<Statement>>,
}

/// CASE statement.
#[derive(Debug, Clone, PartialEq)]
pub struct CaseStatement {
    /// Expression to match.
    pub selector: Spanned<Expression>,
    /// Case branches.
    pub branches: Vec<CaseBranch>,
    /// Optional ELSE branch.
    pub else_branch: Option<Vec<Spanned<Statement>>>,
}

/// A single CASE branch.
#[derive(Debug, Clone, PartialEq)]
pub struct CaseBranch {
    /// Values to match (can be ranges).
    pub values: Vec<CaseValue>,
    /// Statements to execute.
    pub statements: Vec<Spanned<Statement>>,
}

/// A value or range in a CASE branch.
#[derive(Debug, Clone, PartialEq)]
pub enum CaseValue {
    /// Single value.
    Single(Spanned<Expression>),
    /// Range: lo..hi.
    Range(Spanned<Expression>, Spanned<Expression>),
}

/// FOR loop statement.
#[derive(Debug, Clone, PartialEq)]
pub struct ForStatement {
    /// Loop variable name.
    pub variable: String,
    /// Initial value.
    pub from: Spanned<Expression>,
    /// Final value.
    pub to: Spanned<Expression>,
    /// Optional step value (default 1).
    pub by: Option<Spanned<Expression>>,
    /// Loop body.
    pub body: Vec<Spanned<Statement>>,
}

/// WHILE loop statement.
#[derive(Debug, Clone, PartialEq)]
pub struct WhileStatement {
    /// Loop condition.
    pub condition: Spanned<Expression>,
    /// Loop body.
    pub body: Vec<Spanned<Statement>>,
}

/// REPEAT loop statement.
#[derive(Debug, Clone, PartialEq)]
pub struct RepeatStatement {
    /// Loop body.
    pub body: Vec<Spanned<Statement>>,
    /// Exit condition.
    pub until: Spanned<Expression>,
}

/// Function/FB call as a statement.
#[derive(Debug, Clone, PartialEq)]
pub struct CallStatement {
    /// Function or FB instance name.
    pub name: String,
    /// Call arguments.
    pub arguments: Vec<CallArgument>,
}

/// An argument in a function/FB call.
#[derive(Debug, Clone, PartialEq)]
pub struct CallArgument {
    /// Optional parameter name for named arguments.
    pub name: Option<String>,
    /// Argument value.
    pub value: Spanned<Expression>,
}

/// An expression.
#[derive(Debug, Clone, PartialEq)]
pub enum Expression {
    /// Literal value.
    Literal(Literal),
    /// Variable reference.
    Variable(String),
    /// Array indexing: arr[index].
    ArrayAccess {
        /// Array expression.
        array: Box<Spanned<Expression>>,
        /// Index expression.
        index: Box<Spanned<Expression>>,
    },
    /// Struct field access: struct.field.
    FieldAccess {
        /// Struct expression.
        object: Box<Spanned<Expression>>,
        /// Field name.
        field: String,
    },
    /// Binary operation.
    Binary {
        /// Left operand.
        left: Box<Spanned<Expression>>,
        /// Operator.
        op: BinaryOp,
        /// Right operand.
        right: Box<Spanned<Expression>>,
    },
    /// Unary operation.
    Unary {
        /// Operator.
        op: UnaryOp,
        /// Operand.
        operand: Box<Spanned<Expression>>,
    },
    /// Function call.
    Call {
        /// Function name.
        name: String,
        /// Arguments.
        arguments: Vec<CallArgument>,
    },
    /// Parenthesized expression.
    Paren(Box<Spanned<Expression>>),
}

/// Literal values.
#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    /// Boolean: TRUE or FALSE.
    Bool(bool),
    /// Integer literal.
    Integer(i64),
    /// Real (floating-point) literal.
    Real(f64),
    /// String literal.
    String(String),
    /// Time literal (nanoseconds).
    Time(i64),
    /// Date literal.
    Date { year: u16, month: u8, day: u8 },
    /// Time of day literal (nanoseconds since midnight).
    TimeOfDay(i64),
    /// Date and time literal.
    DateTime {
        year: u16,
        month: u8,
        day: u8,
        hour: u8,
        minute: u8,
        second: u8,
        nanosecond: u32,
    },
}

/// Binary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    // Arithmetic
    /// Addition (+).
    Add,
    /// Subtraction (-).
    Sub,
    /// Multiplication (*).
    Mul,
    /// Division (/).
    Div,
    /// Modulo (MOD).
    Mod,
    /// Exponentiation (**).
    Pow,

    // Comparison
    /// Equal (=).
    Eq,
    /// Not equal (<>).
    Ne,
    /// Less than (<).
    Lt,
    /// Less than or equal (<=).
    Le,
    /// Greater than (>).
    Gt,
    /// Greater than or equal (>=).
    Ge,

    // Logical
    /// Logical AND.
    And,
    /// Logical OR.
    Or,
    /// Logical XOR.
    Xor,

    // Bitwise
    /// Bitwise AND (&).
    BitAnd,
    /// Bitwise OR (|).
    BitOr,
    /// Bitwise XOR (^).
    BitXor,
    /// Shift left (SHL).
    Shl,
    /// Shift right (SHR).
    Shr,
}

impl fmt::Display for BinaryOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BinaryOp::Add => write!(f, "+"),
            BinaryOp::Sub => write!(f, "-"),
            BinaryOp::Mul => write!(f, "*"),
            BinaryOp::Div => write!(f, "/"),
            BinaryOp::Mod => write!(f, "MOD"),
            BinaryOp::Pow => write!(f, "**"),
            BinaryOp::Eq => write!(f, "="),
            BinaryOp::Ne => write!(f, "<>"),
            BinaryOp::Lt => write!(f, "<"),
            BinaryOp::Le => write!(f, "<="),
            BinaryOp::Gt => write!(f, ">"),
            BinaryOp::Ge => write!(f, ">="),
            BinaryOp::And => write!(f, "AND"),
            BinaryOp::Or => write!(f, "OR"),
            BinaryOp::Xor => write!(f, "XOR"),
            BinaryOp::BitAnd => write!(f, "&"),
            BinaryOp::BitOr => write!(f, "|"),
            BinaryOp::BitXor => write!(f, "^"),
            BinaryOp::Shl => write!(f, "SHL"),
            BinaryOp::Shr => write!(f, "SHR"),
        }
    }
}

/// Unary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    /// Negation (-).
    Neg,
    /// Logical NOT.
    Not,
}

impl fmt::Display for UnaryOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UnaryOp::Neg => write!(f, "-"),
            UnaryOp::Not => write!(f, "NOT"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_span_merge() {
        let s1 = Span::new(0, 10, 1, 1);
        let s2 = Span::new(15, 25, 1, 16);
        let merged = s1.merge(s2);
        assert_eq!(merged.start, 0);
        assert_eq!(merged.end, 25);
    }

    #[test]
    fn test_data_type_size() {
        assert_eq!(DataType::Bool.size_bytes(), Some(1));
        assert_eq!(DataType::Int.size_bytes(), Some(2));
        assert_eq!(DataType::Dint.size_bytes(), Some(4));
        assert_eq!(DataType::Lint.size_bytes(), Some(8));
        assert_eq!(DataType::Real.size_bytes(), Some(4));
        assert_eq!(DataType::Lreal.size_bytes(), Some(8));
    }

    #[test]
    fn test_data_type_display() {
        assert_eq!(format!("{}", DataType::Bool), "BOOL");
        assert_eq!(format!("{}", DataType::Dint), "DINT");
        assert_eq!(
            format!(
                "{}",
                DataType::Array {
                    lower: 0,
                    upper: 9,
                    element_type: Box::new(DataType::Int)
                }
            ),
            "ARRAY[0..9] OF INT"
        );
    }

    #[test]
    fn test_is_numeric() {
        assert!(DataType::Int.is_numeric());
        assert!(DataType::Real.is_numeric());
        assert!(!DataType::Bool.is_numeric());
        assert!(!DataType::String(None).is_numeric());
    }
}
