//! Lexical tokens for IEC 61131-3 Structured Text.
//!
//! While pest handles lexing during parsing, these token definitions
//! are useful for error messages and documentation.

use super::ast::Span;

/// Token types for Structured Text.
#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // Keywords - Program structure
    /// PROGRAM keyword.
    Program,
    /// END_PROGRAM keyword.
    EndProgram,
    /// FUNCTION keyword.
    Function,
    /// END_FUNCTION keyword.
    EndFunction,
    /// FUNCTION_BLOCK keyword.
    FunctionBlock,
    /// END_FUNCTION_BLOCK keyword.
    EndFunctionBlock,

    // Keywords - Variable declarations
    /// VAR keyword.
    Var,
    /// VAR_INPUT keyword.
    VarInput,
    /// VAR_OUTPUT keyword.
    VarOutput,
    /// VAR_IN_OUT keyword.
    VarInOut,
    /// VAR_EXTERNAL keyword.
    VarExternal,
    /// VAR_GLOBAL keyword.
    VarGlobal,
    /// VAR_TEMP keyword.
    VarTemp,
    /// END_VAR keyword.
    EndVar,
    /// RETAIN keyword.
    Retain,
    /// CONSTANT keyword.
    Constant,

    // Keywords - Control flow
    /// IF keyword.
    If,
    /// THEN keyword.
    Then,
    /// ELSIF keyword.
    Elsif,
    /// ELSE keyword.
    Else,
    /// END_IF keyword.
    EndIf,
    /// CASE keyword.
    Case,
    /// OF keyword.
    Of,
    /// END_CASE keyword.
    EndCase,
    /// FOR keyword.
    For,
    /// TO keyword.
    To,
    /// BY keyword.
    By,
    /// DO keyword.
    Do,
    /// END_FOR keyword.
    EndFor,
    /// WHILE keyword.
    While,
    /// END_WHILE keyword.
    EndWhile,
    /// REPEAT keyword.
    Repeat,
    /// UNTIL keyword.
    Until,
    /// END_REPEAT keyword.
    EndRepeat,
    /// EXIT keyword.
    Exit,
    /// CONTINUE keyword.
    Continue,
    /// RETURN keyword.
    Return,

    // Keywords - Data types
    /// BOOL type.
    Bool,
    /// SINT type.
    Sint,
    /// INT type.
    Int,
    /// DINT type.
    Dint,
    /// LINT type.
    Lint,
    /// USINT type.
    Usint,
    /// UINT type.
    Uint,
    /// UDINT type.
    Udint,
    /// ULINT type.
    Ulint,
    /// REAL type.
    Real,
    /// LREAL type.
    Lreal,
    /// TIME type.
    Time,
    /// DATE type.
    Date,
    /// TIME_OF_DAY type.
    TimeOfDay,
    /// TOD type (alias for TIME_OF_DAY).
    Tod,
    /// DATE_AND_TIME type.
    DateAndTime,
    /// DT type (alias for DATE_AND_TIME).
    Dt,
    /// BYTE type.
    Byte,
    /// WORD type.
    Word,
    /// DWORD type.
    Dword,
    /// LWORD type.
    Lword,
    /// STRING type.
    String,
    /// WSTRING type.
    WString,
    /// ARRAY keyword.
    Array,

    // Keywords - Logical operators
    /// AND operator.
    And,
    /// OR operator.
    Or,
    /// XOR operator.
    Xor,
    /// NOT operator.
    Not,
    /// MOD operator.
    Mod,

    // Keywords - Literals
    /// TRUE literal.
    True,
    /// FALSE literal.
    False,

    // Operators
    /// Assignment (:=).
    Assign,
    /// Plus (+).
    Plus,
    /// Minus (-).
    Minus,
    /// Star (*).
    Star,
    /// Slash (/).
    Slash,
    /// Power (**).
    Power,
    /// Equal (=).
    Equal,
    /// Not equal (<>).
    NotEqual,
    /// Less than (<).
    Less,
    /// Less or equal (<=).
    LessEqual,
    /// Greater than (>).
    Greater,
    /// Greater or equal (>=).
    GreaterEqual,
    /// Ampersand (&).
    Ampersand,

    // Delimiters
    /// Left parenthesis (().
    LParen,
    /// Right parenthesis ()).
    RParen,
    /// Left bracket ([).
    LBracket,
    /// Right bracket (]).
    RBracket,
    /// Semicolon (;).
    Semicolon,
    /// Colon (:).
    Colon,
    /// Comma (,).
    Comma,
    /// Dot (.).
    Dot,
    /// Range (..).
    Range,
    /// Percent (%) for direct addresses.
    Percent,

    // Literals and identifiers
    /// Integer literal.
    IntegerLiteral(i64),
    /// Real literal.
    RealLiteral(f64),
    /// String literal.
    StringLiteral(String),
    /// Time literal.
    TimeLiteral(i64),
    /// Identifier.
    Identifier(String),

    // Special
    /// End of input.
    Eof,
    /// Unknown/invalid token.
    Unknown(char),
}

impl TokenKind {
    /// Get the keyword for a string, if it matches.
    pub fn from_keyword(s: &str) -> Option<Self> {
        // IEC 61131-3 keywords are case-insensitive
        match s.to_uppercase().as_str() {
            "PROGRAM" => Some(TokenKind::Program),
            "END_PROGRAM" => Some(TokenKind::EndProgram),
            "FUNCTION" => Some(TokenKind::Function),
            "END_FUNCTION" => Some(TokenKind::EndFunction),
            "FUNCTION_BLOCK" => Some(TokenKind::FunctionBlock),
            "END_FUNCTION_BLOCK" => Some(TokenKind::EndFunctionBlock),
            "VAR" => Some(TokenKind::Var),
            "VAR_INPUT" => Some(TokenKind::VarInput),
            "VAR_OUTPUT" => Some(TokenKind::VarOutput),
            "VAR_IN_OUT" => Some(TokenKind::VarInOut),
            "VAR_EXTERNAL" => Some(TokenKind::VarExternal),
            "VAR_GLOBAL" => Some(TokenKind::VarGlobal),
            "VAR_TEMP" => Some(TokenKind::VarTemp),
            "END_VAR" => Some(TokenKind::EndVar),
            "RETAIN" => Some(TokenKind::Retain),
            "CONSTANT" => Some(TokenKind::Constant),
            "IF" => Some(TokenKind::If),
            "THEN" => Some(TokenKind::Then),
            "ELSIF" => Some(TokenKind::Elsif),
            "ELSE" => Some(TokenKind::Else),
            "END_IF" => Some(TokenKind::EndIf),
            "CASE" => Some(TokenKind::Case),
            "OF" => Some(TokenKind::Of),
            "END_CASE" => Some(TokenKind::EndCase),
            "FOR" => Some(TokenKind::For),
            "TO" => Some(TokenKind::To),
            "BY" => Some(TokenKind::By),
            "DO" => Some(TokenKind::Do),
            "END_FOR" => Some(TokenKind::EndFor),
            "WHILE" => Some(TokenKind::While),
            "END_WHILE" => Some(TokenKind::EndWhile),
            "REPEAT" => Some(TokenKind::Repeat),
            "UNTIL" => Some(TokenKind::Until),
            "END_REPEAT" => Some(TokenKind::EndRepeat),
            "EXIT" => Some(TokenKind::Exit),
            "CONTINUE" => Some(TokenKind::Continue),
            "RETURN" => Some(TokenKind::Return),
            "BOOL" => Some(TokenKind::Bool),
            "SINT" => Some(TokenKind::Sint),
            "INT" => Some(TokenKind::Int),
            "DINT" => Some(TokenKind::Dint),
            "LINT" => Some(TokenKind::Lint),
            "USINT" => Some(TokenKind::Usint),
            "UINT" => Some(TokenKind::Uint),
            "UDINT" => Some(TokenKind::Udint),
            "ULINT" => Some(TokenKind::Ulint),
            "REAL" => Some(TokenKind::Real),
            "LREAL" => Some(TokenKind::Lreal),
            "TIME" => Some(TokenKind::Time),
            "DATE" => Some(TokenKind::Date),
            "TIME_OF_DAY" => Some(TokenKind::TimeOfDay),
            "TOD" => Some(TokenKind::Tod),
            "DATE_AND_TIME" => Some(TokenKind::DateAndTime),
            "DT" => Some(TokenKind::Dt),
            "BYTE" => Some(TokenKind::Byte),
            "WORD" => Some(TokenKind::Word),
            "DWORD" => Some(TokenKind::Dword),
            "LWORD" => Some(TokenKind::Lword),
            "STRING" => Some(TokenKind::String),
            "WSTRING" => Some(TokenKind::WString),
            "ARRAY" => Some(TokenKind::Array),
            "AND" => Some(TokenKind::And),
            "OR" => Some(TokenKind::Or),
            "XOR" => Some(TokenKind::Xor),
            "NOT" => Some(TokenKind::Not),
            "MOD" => Some(TokenKind::Mod),
            "TRUE" => Some(TokenKind::True),
            "FALSE" => Some(TokenKind::False),
            _ => None,
        }
    }
}

/// A token with its span.
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    /// Token type.
    pub kind: TokenKind,
    /// Source location.
    pub span: Span,
}

impl Token {
    /// Create a new token.
    pub fn new(kind: TokenKind, span: Span) -> Self {
        Self { kind, span }
    }
}

/// Parse a time literal string into nanoseconds.
///
/// Supports formats like:
/// - T#1s, T#100ms, T#1h30m
/// - TIME#1d2h3m4s5ms
pub fn parse_time_literal(s: &str) -> Result<i64, &'static str> {
    // Remove T# or TIME# prefix
    let s = s
        .strip_prefix("T#")
        .or_else(|| s.strip_prefix("TIME#"))
        .or_else(|| s.strip_prefix("t#"))
        .or_else(|| s.strip_prefix("time#"))
        .ok_or("Invalid time literal prefix")?;

    let mut total_ns: i64 = 0;
    let mut current_num = String::new();

    for c in s.chars() {
        if c.is_ascii_digit() || c == '.' || c == '_' {
            if c != '_' {
                current_num.push(c);
            }
        } else {
            if current_num.is_empty() {
                continue;
            }
            let value: f64 = current_num.parse().map_err(|_| "Invalid number in time")?;
            current_num.clear();

            let multiplier = match c.to_ascii_lowercase() {
                'd' => 24 * 60 * 60 * 1_000_000_000i64,
                'h' => 60 * 60 * 1_000_000_000i64,
                'm' if s.contains("ms") => continue, // Handle ms below
                'm' => 60 * 1_000_000_000i64,
                's' => 1_000_000_000i64,
                _ => return Err("Unknown time unit"),
            };

            total_ns += (value * multiplier as f64) as i64;
        }
    }

    // Handle trailing 'ms', 'us', 'ns'
    if s.ends_with("ms") {
        if let Some(idx) = s.rfind(|c: char| !c.is_ascii_digit() && c != '.') {
            let num_str = &s[idx + 1..s.len() - 2];
            if !num_str.is_empty() {
                let value: f64 = num_str.parse().map_err(|_| "Invalid ms value")?;
                total_ns += (value * 1_000_000.0) as i64;
            }
        }
    } else if s.ends_with("us") {
        if let Some(idx) = s.rfind(|c: char| !c.is_ascii_digit() && c != '.') {
            let num_str = &s[idx + 1..s.len() - 2];
            if !num_str.is_empty() {
                let value: f64 = num_str.parse().map_err(|_| "Invalid us value")?;
                total_ns += (value * 1_000.0) as i64;
            }
        }
    } else if s.ends_with("ns") {
        if let Some(idx) = s.rfind(|c: char| !c.is_ascii_digit() && c != '.') {
            let num_str = &s[idx + 1..s.len() - 2];
            if !num_str.is_empty() {
                let value: f64 = num_str.parse().map_err(|_| "Invalid ns value")?;
                total_ns += value as i64;
            }
        }
    }

    Ok(total_ns)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_keyword_lookup() {
        assert_eq!(TokenKind::from_keyword("PROGRAM"), Some(TokenKind::Program));
        assert_eq!(TokenKind::from_keyword("program"), Some(TokenKind::Program));
        assert_eq!(TokenKind::from_keyword("Program"), Some(TokenKind::Program));
        assert_eq!(TokenKind::from_keyword("IF"), Some(TokenKind::If));
        assert_eq!(TokenKind::from_keyword("unknown"), None);
    }

    #[test]
    fn test_time_literal_parsing() {
        assert_eq!(parse_time_literal("T#1s"), Ok(1_000_000_000));
        assert_eq!(parse_time_literal("T#1h"), Ok(3_600_000_000_000));
        assert_eq!(parse_time_literal("TIME#1d"), Ok(86_400_000_000_000));
    }
}
