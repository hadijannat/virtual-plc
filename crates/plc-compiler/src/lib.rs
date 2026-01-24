//! IEC 61131-3 compiler pipeline targeting WebAssembly.
//!
//! This crate provides:
//! - [`frontend`] - ST lexer, parser, and AST
//! - [`typechecker`] - Type checking and semantic analysis
//! - [`ir`] - Intermediate representation
//! - [`codegen`] - WebAssembly code generation
//!
//! # Example
//!
//! ```
//! use plc_compiler::compile;
//!
//! let source = r#"
//!     PROGRAM Main
//!     VAR
//!         x : INT := 0;
//!     END_VAR
//!         x := x + 1;
//!     END_PROGRAM
//! "#;
//!
//! let wasm = compile(source).expect("Compilation failed");
//! assert!(!wasm.is_empty());
//! ```

pub mod codegen;
pub mod frontend;
pub mod ir;
pub mod typechecker;

use frontend::CompilationUnit;

/// Compile Structured Text source to WebAssembly.
///
/// This is a convenience function that creates a `Compiler` and calls
/// `compile_st_to_wasm`. For more control, use `Compiler` directly.
///
/// # Arguments
///
/// * `source` - The ST source code to compile.
///
/// # Returns
///
/// The compiled WebAssembly binary, or an error.
pub fn compile(source: &str) -> anyhow::Result<Vec<u8>> {
    Compiler::new().compile_st_to_wasm(source)
}

/// The main compiler driver.
#[derive(Debug, Default)]
pub struct Compiler {
    /// Enable debug output.
    pub debug: bool,
}

impl Compiler {
    /// Create a new compiler instance.
    pub fn new() -> Self {
        Self::default()
    }

    /// Compile Structured Text source to WebAssembly.
    ///
    /// # Arguments
    ///
    /// * `source` - The ST source code to compile.
    ///
    /// # Returns
    ///
    /// The compiled WebAssembly binary, or an error.
    pub fn compile_st_to_wasm(&self, source: &str) -> anyhow::Result<Vec<u8>> {
        // 1. Parse source into AST
        let ast = self.parse(source)?;

        // 2. Type check the AST
        let typed_ast = self.type_check(&ast)?;

        // 3. Generate IR
        let ir_module = self.generate_ir(&typed_ast)?;

        // 4. Generate Wasm
        let wasm = self.generate_wasm(&ir_module)?;

        Ok(wasm)
    }

    /// Parse ST source into AST.
    fn parse(&self, source: &str) -> anyhow::Result<CompilationUnit> {
        frontend::parse(source)
    }

    /// Type check the AST.
    fn type_check(&self, ast: &CompilationUnit) -> anyhow::Result<typechecker::TypedUnit> {
        typechecker::check(ast)
    }

    /// Generate IR from typed AST.
    fn generate_ir(&self, typed: &typechecker::TypedUnit) -> anyhow::Result<ir::Module> {
        ir::lower(typed)
    }

    /// Generate Wasm from IR.
    fn generate_wasm(&self, ir_module: &ir::Module) -> anyhow::Result<Vec<u8>> {
        codegen::emit(ir_module)
    }
}
