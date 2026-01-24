//! WebAssembly code generation from IR.
//!
//! Generates Wasm binary using wasm-encoder from the IR module.

use crate::ir::{Instruction, Module as IrModule};
use anyhow::{anyhow, Result};
use wasm_encoder::{
    CodeSection, ExportKind, ExportSection, Function, FunctionSection, ImportSection,
    Instruction as WasmInstr, MemorySection, MemoryType, Module, TypeSection, ValType,
};

/// Emit WebAssembly binary from an IR module.
pub fn emit(ir_module: &IrModule) -> Result<Vec<u8>> {
    let mut emitter = WasmEmitter::new();
    emitter.emit(ir_module)
}

struct WasmEmitter {
    /// Type section.
    types: TypeSection,
    /// Import section.
    imports: ImportSection,
    /// Function section.
    functions: FunctionSection,
    /// Export section.
    exports: ExportSection,
    /// Code section.
    code: CodeSection,
    /// Memory section.
    memory: MemorySection,
    /// Next function index.
    next_func_idx: u32,
    /// Host function indices.
    host_funcs: std::collections::HashMap<String, u32>,
    /// User function indices (by name).
    user_funcs: std::collections::HashMap<String, u32>,
}

impl WasmEmitter {
    fn new() -> Self {
        Self {
            types: TypeSection::new(),
            imports: ImportSection::new(),
            functions: FunctionSection::new(),
            exports: ExportSection::new(),
            code: CodeSection::new(),
            memory: MemorySection::new(),
            next_func_idx: 0,
            host_funcs: std::collections::HashMap::new(),
            user_funcs: std::collections::HashMap::new(),
        }
    }

    fn emit(&mut self, ir_module: &IrModule) -> Result<Vec<u8>> {
        // Define types
        self.define_types();

        // Import host functions
        self.import_host_functions();

        // Define memory (1 page = 64KB, matches IR memory_size)
        let pages = ir_module.memory_size.div_ceil(0x10000) as u64;
        self.memory.memory(MemoryType {
            minimum: pages.max(1),
            maximum: Some(pages.max(1)),
            memory64: false,
            shared: false,
            page_size_log2: None,
        });

        // First pass: assign function indices to user-defined functions
        let first_user_func_idx = self.next_func_idx;
        for (i, func) in ir_module.functions.iter().enumerate() {
            self.user_funcs
                .insert(func.name.clone(), first_user_func_idx + i as u32);
        }

        // Generate code for each function
        for func in &ir_module.functions {
            let type_idx = 0; // () -> () for step function
            self.functions.function(type_idx);

            // Convert IR locals to Wasm locals
            let wasm_locals: Vec<(u32, ValType)> = func
                .locals
                .iter()
                .map(|l| {
                    let val_type = match l.wasm_type {
                        crate::ir::WasmType::I32 => ValType::I32,
                        crate::ir::WasmType::I64 => ValType::I64,
                        crate::ir::WasmType::F32 => ValType::F32,
                        crate::ir::WasmType::F64 => ValType::F64,
                    };
                    (1, val_type) // One local of this type
                })
                .collect();

            let mut f = Function::new(wasm_locals);

            // Emit instructions
            for instr in &func.body {
                self.emit_instruction(&mut f, instr)?;
            }

            f.instruction(&WasmInstr::End);
            self.code.function(&f);

            // Export step function
            if func.is_step {
                self.exports
                    .export("step", ExportKind::Func, self.next_func_idx);
            }

            self.next_func_idx += 1;
        }

        // Export memory
        self.exports.export("memory", ExportKind::Memory, 0);

        // Build the module
        let mut module = Module::new();
        module.section(&self.types);
        module.section(&self.imports);
        module.section(&self.functions);
        module.section(&self.memory);
        module.section(&self.exports);
        module.section(&self.code);

        Ok(module.finish())
    }

    fn define_types(&mut self) {
        // Type 0: () -> () for step function
        self.types.ty().function(vec![], vec![]);

        // Type 1: (i32) -> i32 for read_di
        self.types
            .ty()
            .function(vec![ValType::I32], vec![ValType::I32]);

        // Type 2: (i32, i32) -> () for write_do
        self.types
            .ty()
            .function(vec![ValType::I32, ValType::I32], vec![]);

        // Type 3: () -> i32 for get_cycle_time
        self.types.ty().function(vec![], vec![ValType::I32]);
    }

    fn import_host_functions(&mut self) {
        // Import PLC host functions
        // read_di: (bit: i32) -> i32
        self.imports
            .import("plc", "read_di", wasm_encoder::EntityType::Function(1));
        self.host_funcs
            .insert("read_di".to_string(), self.next_func_idx);
        self.next_func_idx += 1;

        // write_do: (bit: i32, value: i32) -> ()
        self.imports
            .import("plc", "write_do", wasm_encoder::EntityType::Function(2));
        self.host_funcs
            .insert("write_do".to_string(), self.next_func_idx);
        self.next_func_idx += 1;

        // read_ai: (channel: i32) -> i32
        self.imports
            .import("plc", "read_ai", wasm_encoder::EntityType::Function(1));
        self.host_funcs
            .insert("read_ai".to_string(), self.next_func_idx);
        self.next_func_idx += 1;

        // write_ao: (channel: i32, value: i32) -> ()
        self.imports
            .import("plc", "write_ao", wasm_encoder::EntityType::Function(2));
        self.host_funcs
            .insert("write_ao".to_string(), self.next_func_idx);
        self.next_func_idx += 1;

        // get_cycle_time: () -> i32
        self.imports.import(
            "plc",
            "get_cycle_time",
            wasm_encoder::EntityType::Function(3),
        );
        self.host_funcs
            .insert("get_cycle_time".to_string(), self.next_func_idx);
        self.next_func_idx += 1;
    }

    fn emit_instruction(&self, f: &mut Function, instr: &Instruction) -> Result<()> {
        match instr {
            // Constants
            Instruction::I32Const(v) => {
                f.instruction(&WasmInstr::I32Const(*v));
            }
            Instruction::I64Const(v) => {
                f.instruction(&WasmInstr::I64Const(*v));
            }
            Instruction::F32Const(v) => {
                f.instruction(&WasmInstr::F32Const(*v));
            }
            Instruction::F64Const(v) => {
                f.instruction(&WasmInstr::F64Const(*v));
            }

            // Memory loads
            Instruction::I32Load { offset } => {
                f.instruction(&WasmInstr::I32Load(wasm_encoder::MemArg {
                    offset: *offset as u64,
                    align: 2,
                    memory_index: 0,
                }));
            }
            Instruction::I64Load { offset } => {
                f.instruction(&WasmInstr::I64Load(wasm_encoder::MemArg {
                    offset: *offset as u64,
                    align: 3,
                    memory_index: 0,
                }));
            }
            Instruction::F32Load { offset } => {
                f.instruction(&WasmInstr::F32Load(wasm_encoder::MemArg {
                    offset: *offset as u64,
                    align: 2,
                    memory_index: 0,
                }));
            }
            Instruction::F64Load { offset } => {
                f.instruction(&WasmInstr::F64Load(wasm_encoder::MemArg {
                    offset: *offset as u64,
                    align: 3,
                    memory_index: 0,
                }));
            }
            Instruction::I32Load8S { offset } => {
                f.instruction(&WasmInstr::I32Load8S(wasm_encoder::MemArg {
                    offset: *offset as u64,
                    align: 0,
                    memory_index: 0,
                }));
            }
            Instruction::I32Load16S { offset } => {
                f.instruction(&WasmInstr::I32Load16S(wasm_encoder::MemArg {
                    offset: *offset as u64,
                    align: 1,
                    memory_index: 0,
                }));
            }

            // Memory stores
            Instruction::I32Store { offset } => {
                f.instruction(&WasmInstr::I32Store(wasm_encoder::MemArg {
                    offset: *offset as u64,
                    align: 2,
                    memory_index: 0,
                }));
            }
            Instruction::I64Store { offset } => {
                f.instruction(&WasmInstr::I64Store(wasm_encoder::MemArg {
                    offset: *offset as u64,
                    align: 3,
                    memory_index: 0,
                }));
            }
            Instruction::F32Store { offset } => {
                f.instruction(&WasmInstr::F32Store(wasm_encoder::MemArg {
                    offset: *offset as u64,
                    align: 2,
                    memory_index: 0,
                }));
            }
            Instruction::F64Store { offset } => {
                f.instruction(&WasmInstr::F64Store(wasm_encoder::MemArg {
                    offset: *offset as u64,
                    align: 3,
                    memory_index: 0,
                }));
            }
            Instruction::I32Store8 { offset } => {
                f.instruction(&WasmInstr::I32Store8(wasm_encoder::MemArg {
                    offset: *offset as u64,
                    align: 0,
                    memory_index: 0,
                }));
            }
            Instruction::I32Store16 { offset } => {
                f.instruction(&WasmInstr::I32Store16(wasm_encoder::MemArg {
                    offset: *offset as u64,
                    align: 1,
                    memory_index: 0,
                }));
            }

            // Local variables
            Instruction::LocalGet(idx) => {
                f.instruction(&WasmInstr::LocalGet(*idx));
            }
            Instruction::LocalSet(idx) => {
                f.instruction(&WasmInstr::LocalSet(*idx));
            }
            Instruction::LocalTee(idx) => {
                f.instruction(&WasmInstr::LocalTee(*idx));
            }

            // i32 arithmetic
            Instruction::I32Add => {
                f.instruction(&WasmInstr::I32Add);
            }
            Instruction::I32Sub => {
                f.instruction(&WasmInstr::I32Sub);
            }
            Instruction::I32Mul => {
                f.instruction(&WasmInstr::I32Mul);
            }
            Instruction::I32DivS => {
                f.instruction(&WasmInstr::I32DivS);
            }
            Instruction::I32RemS => {
                f.instruction(&WasmInstr::I32RemS);
            }

            // i64 arithmetic
            Instruction::I64Add => {
                f.instruction(&WasmInstr::I64Add);
            }
            Instruction::I64Sub => {
                f.instruction(&WasmInstr::I64Sub);
            }
            Instruction::I64Mul => {
                f.instruction(&WasmInstr::I64Mul);
            }
            Instruction::I64DivS => {
                f.instruction(&WasmInstr::I64DivS);
            }

            // f32 arithmetic
            Instruction::F32Add => {
                f.instruction(&WasmInstr::F32Add);
            }
            Instruction::F32Sub => {
                f.instruction(&WasmInstr::F32Sub);
            }
            Instruction::F32Mul => {
                f.instruction(&WasmInstr::F32Mul);
            }
            Instruction::F32Div => {
                f.instruction(&WasmInstr::F32Div);
            }

            // f64 arithmetic
            Instruction::F64Add => {
                f.instruction(&WasmInstr::F64Add);
            }
            Instruction::F64Sub => {
                f.instruction(&WasmInstr::F64Sub);
            }
            Instruction::F64Mul => {
                f.instruction(&WasmInstr::F64Mul);
            }
            Instruction::F64Div => {
                f.instruction(&WasmInstr::F64Div);
            }

            // i32 comparison
            Instruction::I32Eq => {
                f.instruction(&WasmInstr::I32Eq);
            }
            Instruction::I32Ne => {
                f.instruction(&WasmInstr::I32Ne);
            }
            Instruction::I32LtS => {
                f.instruction(&WasmInstr::I32LtS);
            }
            Instruction::I32LeS => {
                f.instruction(&WasmInstr::I32LeS);
            }
            Instruction::I32GtS => {
                f.instruction(&WasmInstr::I32GtS);
            }
            Instruction::I32GeS => {
                f.instruction(&WasmInstr::I32GeS);
            }
            Instruction::I32Eqz => {
                f.instruction(&WasmInstr::I32Eqz);
            }

            // f32 comparison
            Instruction::F32Eq => {
                f.instruction(&WasmInstr::F32Eq);
            }
            Instruction::F32Lt => {
                f.instruction(&WasmInstr::F32Lt);
            }
            Instruction::F32Gt => {
                f.instruction(&WasmInstr::F32Gt);
            }

            // f64 comparison
            Instruction::F64Eq => {
                f.instruction(&WasmInstr::F64Eq);
            }
            Instruction::F64Lt => {
                f.instruction(&WasmInstr::F64Lt);
            }
            Instruction::F64Gt => {
                f.instruction(&WasmInstr::F64Gt);
            }

            // Bitwise
            Instruction::I32And => {
                f.instruction(&WasmInstr::I32And);
            }
            Instruction::I32Or => {
                f.instruction(&WasmInstr::I32Or);
            }
            Instruction::I32Xor => {
                f.instruction(&WasmInstr::I32Xor);
            }
            Instruction::I32Shl => {
                f.instruction(&WasmInstr::I32Shl);
            }
            Instruction::I32ShrS => {
                f.instruction(&WasmInstr::I32ShrS);
            }

            // Conversions
            Instruction::I32WrapI64 => {
                f.instruction(&WasmInstr::I32WrapI64);
            }
            Instruction::I64ExtendI32S => {
                f.instruction(&WasmInstr::I64ExtendI32S);
            }
            Instruction::F32ConvertI32S => {
                f.instruction(&WasmInstr::F32ConvertI32S);
            }
            Instruction::F64ConvertI32S => {
                f.instruction(&WasmInstr::F64ConvertI32S);
            }
            Instruction::I32TruncF32S => {
                f.instruction(&WasmInstr::I32TruncF32S);
            }
            Instruction::I32TruncF64S => {
                f.instruction(&WasmInstr::I32TruncF64S);
            }
            Instruction::F64PromoteF32 => {
                f.instruction(&WasmInstr::F64PromoteF32);
            }
            Instruction::F32DemoteF64 => {
                f.instruction(&WasmInstr::F32DemoteF64);
            }

            // Control flow
            Instruction::Br(depth) => {
                f.instruction(&WasmInstr::Br(*depth));
            }
            Instruction::BrIf(depth) => {
                f.instruction(&WasmInstr::BrIf(*depth));
            }
            Instruction::Block => {
                f.instruction(&WasmInstr::Block(wasm_encoder::BlockType::Empty));
            }
            Instruction::Loop => {
                f.instruction(&WasmInstr::Loop(wasm_encoder::BlockType::Empty));
            }
            Instruction::End => {
                f.instruction(&WasmInstr::End);
            }
            Instruction::If => {
                f.instruction(&WasmInstr::If(wasm_encoder::BlockType::Empty));
            }
            Instruction::Else => {
                f.instruction(&WasmInstr::Else);
            }
            Instruction::Return => {
                f.instruction(&WasmInstr::Return);
            }

            // Function calls
            Instruction::Call(idx) => {
                f.instruction(&WasmInstr::Call(*idx));
            }
            Instruction::CallHost(name) => {
                let idx = self
                    .host_funcs
                    .get(name)
                    .ok_or_else(|| anyhow!("Unknown host function: {}", name))?;
                f.instruction(&WasmInstr::Call(*idx));
            }
            Instruction::CallUser(name) => {
                let idx = self
                    .user_funcs
                    .get(name)
                    .ok_or_else(|| anyhow!("Unknown user function: {}", name))?;
                f.instruction(&WasmInstr::Call(*idx));
            }

            // Stack
            Instruction::Drop => {
                f.instruction(&WasmInstr::Drop);
            }
            Instruction::Select => {
                f.instruction(&WasmInstr::Select);
            }

            Instruction::Nop => {
                f.instruction(&WasmInstr::Nop);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::parse;
    use crate::ir::lower;
    use crate::typechecker::check;

    #[test]
    fn test_emit_simple_program() {
        let source = r#"
            PROGRAM Main
            VAR
                x : INT := 0;
            END_VAR
                x := x + 1;
            END_PROGRAM
        "#;

        let ast = parse(source).unwrap();
        let typed = check(&ast).unwrap();
        let ir = lower(&typed).unwrap();
        let result = emit(&ir);

        assert!(result.is_ok(), "Wasm emit failed: {:?}", result.err());

        let wasm = result.unwrap();
        assert!(!wasm.is_empty(), "Generated Wasm should not be empty");

        // Check Wasm magic number
        assert_eq!(&wasm[0..4], b"\x00asm", "Invalid Wasm magic number");
    }

    #[test]
    fn test_emit_with_if() {
        let source = r#"
            PROGRAM Test
            VAR
                flag : BOOL := TRUE;
                count : INT := 0;
            END_VAR
                IF flag THEN
                    count := count + 1;
                ELSE
                    count := 0;
                END_IF;
            END_PROGRAM
        "#;

        let ast = parse(source).unwrap();
        let typed = check(&ast).unwrap();
        let ir = lower(&typed).unwrap();
        let result = emit(&ir);

        assert!(result.is_ok());
    }
}
