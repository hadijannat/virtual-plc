#![allow(non_camel_case_types)]

/// Minimal IEC 61131-3 type aliases (scaffold).
pub type BOOL = bool;
pub type BYTE = u8;
pub type WORD = u16;
pub type DWORD = u32;
pub type LWORD = u64;

pub type SINT = i8;
pub type INT = i16;
pub type DINT = i32;
pub type LINT = i64;

pub type USINT = u8;
pub type UINT = u16;
pub type UDINT = u32;
pub type ULINT = u64;

/// TIME represented as nanoseconds (scaffold; pick a canonical resolution).
pub type TIME = i64;
