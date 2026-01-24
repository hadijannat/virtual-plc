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
/// - T#1h30m500ms (mixed units in any order)
pub fn parse_time_literal(s: &str) -> Result<i64, &'static str> {
    // Remove T# or TIME# prefix (case-insensitive)
    let s = s
        .strip_prefix("T#")
        .or_else(|| s.strip_prefix("TIME#"))
        .or_else(|| s.strip_prefix("t#"))
        .or_else(|| s.strip_prefix("time#"))
        .ok_or("Invalid time literal prefix")?;

    let mut total_ns: i64 = 0;
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        // Skip underscores (number separators)
        while i < chars.len() && chars[i] == '_' {
            i += 1;
        }
        if i >= chars.len() {
            break;
        }

        // Parse number (integer or decimal)
        let num_start = i;
        while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
            i += 1;
        }

        if i == num_start {
            // No number found, skip this character
            i += 1;
            continue;
        }

        let num_str: String = chars[num_start..i].iter().collect();
        let value: f64 = num_str.parse().map_err(|_| "Invalid number in time")?;

        // Parse unit (check for 2-char units first: ms, us, ns)
        if i >= chars.len() {
            return Err("Missing time unit");
        }

        let c1 = chars[i].to_ascii_lowercase();

        // Check for two-character units
        let multiplier = if i + 1 < chars.len() {
            let c2 = chars[i + 1].to_ascii_lowercase();
            match (c1, c2) {
                ('m', 's') => {
                    i += 2;
                    1_000_000i64 // milliseconds
                }
                ('u', 's') => {
                    i += 2;
                    1_000i64 // microseconds
                }
                ('n', 's') => {
                    i += 2;
                    1i64 // nanoseconds
                }
                _ => {
                    // Single character unit
                    i += 1;
                    match c1 {
                        'd' => 24 * 60 * 60 * 1_000_000_000i64, // days
                        'h' => 60 * 60 * 1_000_000_000i64,      // hours
                        'm' => 60 * 1_000_000_000i64,           // minutes
                        's' => 1_000_000_000i64,                // seconds
                        _ => return Err("Unknown time unit"),
                    }
                }
            }
        } else {
            // Last character, must be single-char unit
            i += 1;
            match c1 {
                'd' => 24 * 60 * 60 * 1_000_000_000i64,
                'h' => 60 * 60 * 1_000_000_000i64,
                'm' => 60 * 1_000_000_000i64,
                's' => 1_000_000_000i64,
                _ => return Err("Unknown time unit"),
            }
        };

        total_ns = total_ns.saturating_add((value * multiplier as f64) as i64);
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

    #[test]
    fn test_time_literal_milliseconds() {
        assert_eq!(parse_time_literal("T#1ms"), Ok(1_000_000));
        assert_eq!(parse_time_literal("T#100ms"), Ok(100_000_000));
        assert_eq!(parse_time_literal("T#500ms"), Ok(500_000_000));
    }

    #[test]
    fn test_time_literal_microseconds() {
        assert_eq!(parse_time_literal("T#1us"), Ok(1_000));
        assert_eq!(parse_time_literal("T#100us"), Ok(100_000));
    }

    #[test]
    fn test_time_literal_nanoseconds() {
        assert_eq!(parse_time_literal("T#1ns"), Ok(1));
        assert_eq!(parse_time_literal("T#1000ns"), Ok(1000));
    }

    #[test]
    fn test_time_literal_mixed_units() {
        // 1 second + 500 milliseconds
        assert_eq!(parse_time_literal("T#1s500ms"), Ok(1_500_000_000));

        // 1 hour + 30 minutes
        assert_eq!(
            parse_time_literal("T#1h30m"),
            Ok(3_600_000_000_000 + 30 * 60 * 1_000_000_000)
        );

        // 1 hour + 30 minutes + 500 milliseconds
        assert_eq!(
            parse_time_literal("T#1h30m500ms"),
            Ok(3_600_000_000_000 + 30 * 60 * 1_000_000_000 + 500_000_000)
        );

        // Full format: days, hours, minutes, seconds, ms
        assert_eq!(
            parse_time_literal("TIME#1d2h3m4s5ms"),
            Ok(86_400_000_000_000  // 1 day
               + 2 * 3_600_000_000_000  // 2 hours
               + 3 * 60_000_000_000     // 3 minutes
               + 4 * 1_000_000_000      // 4 seconds
               + 5_000_000)             // 5 ms
        );
    }

    #[test]
    fn test_time_literal_minutes_with_ms() {
        // This was the bug: minutes were skipped when "ms" appeared anywhere
        // 2 minutes + 500 milliseconds = 120s + 0.5s = 120.5s
        assert_eq!(
            parse_time_literal("T#2m500ms"),
            Ok(2 * 60 * 1_000_000_000 + 500_000_000)
        );
    }

    #[test]
    fn test_time_literal_decimal() {
        assert_eq!(parse_time_literal("T#1.5s"), Ok(1_500_000_000));
        assert_eq!(parse_time_literal("T#0.5h"), Ok(30 * 60 * 1_000_000_000));
    }
}
