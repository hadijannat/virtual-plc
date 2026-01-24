//! Integration tests for the IEC 61131-3 compiler.
//!
//! These tests verify the complete compilation pipeline from ST source to Wasm.

use plc_compiler::compile;

/// Test compiling a simple blink program.
#[test]
fn test_compile_blink_program() {
    let source = r#"
        PROGRAM Blink
        VAR
            output : BOOL := FALSE;
            counter : INT := 0;
            max_count : INT := 100;
        END_VAR
            counter := counter + 1;
            IF counter >= max_count THEN
                output := NOT output;
                counter := 0;
            END_IF;
        END_PROGRAM
    "#;

    let result = compile(source);
    assert!(result.is_ok(), "Compile failed: {:?}", result.err());

    let wasm = result.unwrap();
    assert!(!wasm.is_empty(), "Wasm output is empty");

    // Verify Wasm magic number
    assert_eq!(&wasm[0..4], &[0x00, 0x61, 0x73, 0x6d], "Invalid Wasm magic");
    // Verify Wasm version (1)
    assert_eq!(&wasm[4..8], &[0x01, 0x00, 0x00, 0x00], "Invalid Wasm version");
}

/// Test compiling a program with arithmetic expressions.
#[test]
fn test_compile_arithmetic() {
    let source = r#"
        PROGRAM Arithmetic
        VAR
            a : INT := 10;
            b : INT := 5;
            sum : INT;
            diff : INT;
            prod : INT;
            quot : INT;
            rem : INT;
        END_VAR
            sum := a + b;
            diff := a - b;
            prod := a * b;
            quot := a / b;
            rem := a MOD b;
        END_PROGRAM
    "#;

    let result = compile(source);
    assert!(result.is_ok(), "Compile failed: {:?}", result.err());
}

/// Test compiling a program with boolean logic.
#[test]
fn test_compile_boolean_logic() {
    let source = r#"
        PROGRAM BooleanLogic
        VAR
            input1 : BOOL := TRUE;
            input2 : BOOL := FALSE;
            and_result : BOOL;
            or_result : BOOL;
            xor_result : BOOL;
            not_result : BOOL;
        END_VAR
            and_result := input1 AND input2;
            or_result := input1 OR input2;
            xor_result := input1 XOR input2;
            not_result := NOT input1;
        END_PROGRAM
    "#;

    let result = compile(source);
    assert!(result.is_ok(), "Compile failed: {:?}", result.err());
}

/// Test compiling a program with FOR loop.
#[test]
fn test_compile_for_loop() {
    let source = r#"
        PROGRAM ForLoop
        VAR
            i : INT;
            sum : INT := 0;
        END_VAR
            FOR i := 1 TO 10 DO
                sum := sum + i;
            END_FOR;
        END_PROGRAM
    "#;

    let result = compile(source);
    assert!(result.is_ok(), "Compile failed: {:?}", result.err());
}

/// Test compiling a program with WHILE loop.
#[test]
fn test_compile_while_loop() {
    let source = r#"
        PROGRAM WhileLoop
        VAR
            counter : INT := 0;
            limit : INT := 100;
        END_VAR
            WHILE counter < limit DO
                counter := counter + 1;
            END_WHILE;
        END_PROGRAM
    "#;

    let result = compile(source);
    assert!(result.is_ok(), "Compile failed: {:?}", result.err());
}

/// Test compiling a program with nested IF statements.
#[test]
fn test_compile_nested_if() {
    let source = r#"
        PROGRAM NestedIf
        VAR
            value : INT := 50;
            category : INT := 0;
        END_VAR
            IF value < 0 THEN
                category := -1;
            ELSIF value < 25 THEN
                category := 1;
            ELSIF value < 50 THEN
                category := 2;
            ELSIF value < 75 THEN
                category := 3;
            ELSE
                category := 4;
            END_IF;
        END_PROGRAM
    "#;

    let result = compile(source);
    assert!(result.is_ok(), "Compile failed: {:?}", result.err());
}

/// Test compiling a FUNCTION.
#[test]
fn test_compile_function() {
    let source = r#"
        FUNCTION Add : INT
        VAR_INPUT
            a : INT;
            b : INT;
        END_VAR
            Add := a + b;
        END_FUNCTION
    "#;

    let result = compile(source);
    assert!(result.is_ok(), "Compile failed: {:?}", result.err());
}

/// Test compiling a program with comparison operators.
#[test]
fn test_compile_comparisons() {
    let source = r#"
        PROGRAM Comparisons
        VAR
            a : INT := 10;
            b : INT := 20;
            lt : BOOL;
            le : BOOL;
            gt : BOOL;
            ge : BOOL;
            eq : BOOL;
            ne : BOOL;
        END_VAR
            lt := a < b;
            le := a <= b;
            gt := a > b;
            ge := a >= b;
            eq := a = b;
            ne := a <> b;
        END_PROGRAM
    "#;

    let result = compile(source);
    assert!(result.is_ok(), "Compile failed: {:?}", result.err());
}

/// Test compiling with different integer types.
#[test]
fn test_compile_integer_types() {
    let source = r#"
        PROGRAM IntTypes
        VAR
            sint_val : SINT := 127;
            int_val : INT := 32767;
            dint_val : DINT := 100000;
            usint_val : USINT := 255;
            uint_val : UINT := 65535;
            udint_val : UDINT := 1000000;
        END_VAR
            int_val := int_val + 1;
            dint_val := dint_val - 1;
        END_PROGRAM
    "#;

    let result = compile(source);
    assert!(result.is_ok(), "Compile failed: {:?}", result.err());
}

/// Test compiling with REAL types.
#[test]
fn test_compile_real_types() {
    let source = r#"
        PROGRAM RealTypes
        VAR
            real_val : REAL := 3.14;
            lreal_val : LREAL := 2.71828;
            result : REAL;
        END_VAR
            result := real_val + 1.0;
        END_PROGRAM
    "#;

    let result = compile(source);
    assert!(result.is_ok(), "Compile failed: {:?}", result.err());
}

/// Test that generated Wasm can be validated by wasmtime.
#[test]
fn test_wasm_validates() {
    let source = r#"
        PROGRAM ValidateMe
        VAR
            x : INT := 0;
        END_VAR
            x := x + 1;
        END_PROGRAM
    "#;

    let wasm = compile(source).expect("Compile failed");

    // Use wasmparser to validate the Wasm module
    let parser = wasmparser::Parser::new(0);
    let mut validator = wasmparser::Validator::new();

    for payload in parser.parse_all(&wasm) {
        match payload {
            Ok(payload) => {
                if let Err(e) = validator.payload(&payload) {
                    panic!("Wasm validation failed: {}", e);
                }
            }
            Err(e) => {
                panic!("Wasm parsing failed: {}", e);
            }
        }
    }
}
