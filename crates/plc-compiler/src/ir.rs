//! Intermediate Representation for compiled PLC programs.
//!
//! Uses a stack-based IR similar to WebAssembly for easy code generation.

use crate::frontend::{BinaryOp, DataType, UnaryOp};
use crate::typechecker::{
    TypedExpr, TypedExprKind, TypedFunction, TypedFunctionBlock, TypedLiteral, TypedPou,
    TypedProgram, TypedStatement, TypedUnit,
};
use anyhow::Result;

/// An IR module containing functions and data.
#[derive(Debug, Clone)]
pub struct Module {
    /// Functions in this module.
    pub functions: Vec<IrFunction>,
    /// Global data segment.
    pub data: Vec<u8>,
    /// Total memory size needed.
    pub memory_size: usize,
}

/// An IR function.
#[derive(Debug, Clone)]
pub struct IrFunction {
    /// Function name.
    pub name: String,
    /// Whether this is the main step function.
    pub is_step: bool,
    /// Local variable count (for Wasm).
    pub locals: Vec<LocalVar>,
    /// IR instructions.
    pub body: Vec<Instruction>,
}

/// A local variable.
#[derive(Debug, Clone)]
pub struct LocalVar {
    /// Variable name.
    pub name: String,
    /// Wasm value type.
    pub wasm_type: WasmType,
}

/// Wasm value types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WasmType {
    /// 32-bit integer.
    I32,
    /// 64-bit integer.
    I64,
    /// 32-bit float.
    F32,
    /// 64-bit float.
    F64,
}

impl WasmType {
    /// Convert from IEC data type.
    pub fn from_data_type(ty: &DataType) -> Self {
        match ty {
            DataType::Bool
            | DataType::Sint
            | DataType::Int
            | DataType::Dint
            | DataType::Usint
            | DataType::Uint
            | DataType::Udint
            | DataType::Byte
            | DataType::Word
            | DataType::Dword => WasmType::I32,
            DataType::Lint | DataType::Ulint | DataType::Lword | DataType::Time => WasmType::I64,
            DataType::Real => WasmType::F32,
            DataType::Lreal => WasmType::F64,
            _ => WasmType::I32,
        }
    }
}

/// IR instructions (stack-based).
#[derive(Debug, Clone)]
pub enum Instruction {
    // Constants
    /// Push i32 constant.
    I32Const(i32),
    /// Push i64 constant.
    I64Const(i64),
    /// Push f32 constant.
    F32Const(f32),
    /// Push f64 constant.
    F64Const(f64),

    // Memory operations
    /// Load i32 from memory address.
    I32Load { offset: u32 },
    /// Load i64 from memory.
    I64Load { offset: u32 },
    /// Load f32 from memory.
    F32Load { offset: u32 },
    /// Load f64 from memory.
    F64Load { offset: u32 },
    /// Load i8 and sign-extend to i32.
    I32Load8S { offset: u32 },
    /// Load i16 and sign-extend to i32.
    I32Load16S { offset: u32 },
    /// Store i32 to memory.
    I32Store { offset: u32 },
    /// Store i64 to memory.
    I64Store { offset: u32 },
    /// Store f32 to memory.
    F32Store { offset: u32 },
    /// Store f64 to memory.
    F64Store { offset: u32 },
    /// Store i8 to memory.
    I32Store8 { offset: u32 },
    /// Store i16 to memory.
    I32Store16 { offset: u32 },

    // Local variables
    /// Get local variable.
    LocalGet(u32),
    /// Set local variable.
    LocalSet(u32),
    /// Tee local (set and keep on stack).
    LocalTee(u32),

    // Arithmetic - i32
    /// i32 addition.
    I32Add,
    /// i32 subtraction.
    I32Sub,
    /// i32 multiplication.
    I32Mul,
    /// i32 signed division.
    I32DivS,
    /// i32 signed remainder.
    I32RemS,

    // Arithmetic - i64
    /// i64 addition.
    I64Add,
    /// i64 subtraction.
    I64Sub,
    /// i64 multiplication.
    I64Mul,
    /// i64 signed division.
    I64DivS,

    // Arithmetic - f32
    /// f32 addition.
    F32Add,
    /// f32 subtraction.
    F32Sub,
    /// f32 multiplication.
    F32Mul,
    /// f32 division.
    F32Div,

    // Arithmetic - f64
    /// f64 addition.
    F64Add,
    /// f64 subtraction.
    F64Sub,
    /// f64 multiplication.
    F64Mul,
    /// f64 division.
    F64Div,

    // Comparison - i32
    /// i32 equal.
    I32Eq,
    /// i32 not equal.
    I32Ne,
    /// i32 signed less than.
    I32LtS,
    /// i32 signed less or equal.
    I32LeS,
    /// i32 signed greater than.
    I32GtS,
    /// i32 signed greater or equal.
    I32GeS,
    /// i32 equal to zero.
    I32Eqz,

    // Comparison - f32
    /// f32 equal.
    F32Eq,
    /// f32 less than.
    F32Lt,
    /// f32 greater than.
    F32Gt,

    // Comparison - f64
    /// f64 equal.
    F64Eq,
    /// f64 less than.
    F64Lt,
    /// f64 greater than.
    F64Gt,

    // Logical/Bitwise
    /// i32 bitwise and.
    I32And,
    /// i32 bitwise or.
    I32Or,
    /// i32 bitwise xor.
    I32Xor,
    /// i32 shift left.
    I32Shl,
    /// i32 shift right (signed).
    I32ShrS,

    // Conversions
    /// Wrap i64 to i32.
    I32WrapI64,
    /// Extend i32 to i64 (signed).
    I64ExtendI32S,
    /// Convert i32 to f32.
    F32ConvertI32S,
    /// Convert i32 to f64.
    F64ConvertI32S,
    /// Truncate f32 to i32.
    I32TruncF32S,
    /// Truncate f64 to i32.
    I32TruncF64S,
    /// Promote f32 to f64.
    F64PromoteF32,
    /// Demote f64 to f32.
    F32DemoteF64,

    // Control flow
    /// Unconditional branch.
    Br(u32),
    /// Conditional branch (if top of stack is non-zero).
    BrIf(u32),
    /// Block start.
    Block,
    /// Loop start.
    Loop,
    /// End block/loop/if.
    End,
    /// If block.
    If,
    /// Else branch.
    Else,
    /// Return from function.
    Return,

    // Function calls
    /// Call function by index.
    Call(u32),
    /// Call host function (imported).
    CallHost(String),
    /// Call user-defined function (by name, resolved later).
    CallUser(String),

    // Stack manipulation
    /// Drop top of stack.
    Drop,
    /// Select between two values.
    Select,

    // No operation (for labels).
    Nop,
}

/// Lower typed AST to IR.
pub fn lower(typed: &TypedUnit) -> Result<Module> {
    let mut lowerer = IrLowerer::new();
    lowerer.lower_unit(typed)
}

struct IrLowerer {
    /// Generated functions.
    functions: Vec<IrFunction>,
    /// Current function's instructions.
    current_body: Vec<Instruction>,
    /// Current function's local variables.
    current_locals: Vec<LocalVar>,
    /// Current nesting depth for break/continue.
    loop_depth: u32,
    /// Memory size.
    memory_size: usize,
}

impl IrLowerer {
    fn new() -> Self {
        Self {
            functions: Vec::new(),
            current_body: Vec::new(),
            current_locals: Vec::new(),
            loop_depth: 0,
            memory_size: 0x1000, // 4KB default
        }
    }

    /// Allocate a temporary local variable and return its index.
    fn alloc_temp_local(&mut self, wasm_type: WasmType) -> u32 {
        let idx = self.current_locals.len() as u32;
        self.current_locals.push(LocalVar {
            name: format!("__temp_{}", idx),
            wasm_type,
        });
        idx
    }

    fn lower_unit(&mut self, typed: &TypedUnit) -> Result<Module> {
        for pou in &typed.units {
            match pou {
                TypedPou::Program(p) => self.lower_program(p)?,
                TypedPou::FunctionBlock(fb) => self.lower_function_block(fb)?,
                TypedPou::Function(f) => self.lower_function(f)?,
            }
        }

        Ok(Module {
            functions: self.functions.clone(),
            data: Vec::new(),
            memory_size: self.memory_size,
        })
    }

    fn lower_program(&mut self, program: &TypedProgram) -> Result<()> {
        self.current_body.clear();
        self.current_locals.clear();

        // Lower all statements
        for stmt in &program.body {
            self.lower_statement(stmt)?;
        }

        // Create step function
        let step_fn = IrFunction {
            name: "step".to_string(),
            is_step: true,
            locals: std::mem::take(&mut self.current_locals),
            body: std::mem::take(&mut self.current_body),
        };

        self.functions.push(step_fn);
        Ok(())
    }

    fn lower_function_block(&mut self, fb: &TypedFunctionBlock) -> Result<()> {
        self.current_body.clear();
        self.current_locals.clear();

        for stmt in &fb.body {
            self.lower_statement(stmt)?;
        }

        let func = IrFunction {
            name: fb.name.clone(),
            is_step: false,
            locals: std::mem::take(&mut self.current_locals),
            body: std::mem::take(&mut self.current_body),
        };

        self.functions.push(func);
        Ok(())
    }

    fn lower_function(&mut self, func: &TypedFunction) -> Result<()> {
        self.current_body.clear();
        self.current_locals.clear();

        for stmt in &func.body {
            self.lower_statement(stmt)?;
        }

        let ir_func = IrFunction {
            name: func.name.clone(),
            is_step: false,
            locals: std::mem::take(&mut self.current_locals),
            body: std::mem::take(&mut self.current_body),
        };

        self.functions.push(ir_func);
        Ok(())
    }

    fn lower_statement(&mut self, stmt: &TypedStatement) -> Result<()> {
        match stmt {
            TypedStatement::Assignment { target, value } => {
                // Push address for store
                self.push_address(target)?;
                // Push value
                self.lower_expr(value)?;
                // Store based on type
                self.emit_store(&target.ty)?;
            }
            TypedStatement::If {
                condition,
                then_branch,
                elsif_branches,
                else_branch,
            } => {
                self.lower_if(condition, then_branch, elsif_branches, else_branch)?;
            }
            TypedStatement::For {
                variable: _,
                var_offset,
                from,
                to,
                by,
                body,
            } => {
                self.lower_for(*var_offset, from, to, by.as_ref(), body)?;
            }
            TypedStatement::While { condition, body } => {
                self.lower_while(condition, body)?;
            }
            TypedStatement::Repeat { body, until } => {
                self.lower_repeat(body, until)?;
            }
            TypedStatement::Case {
                selector,
                branches,
                else_branch,
            } => {
                // Simplified: use if-else chain
                self.lower_case(selector, branches, else_branch)?;
            }
            TypedStatement::Exit => {
                // Break from loop - branch to end of innermost loop
                self.current_body.push(Instruction::Br(self.loop_depth));
            }
            TypedStatement::Continue => {
                // Continue loop - branch to start of loop
                self.current_body.push(Instruction::Br(0));
            }
            TypedStatement::Return(expr) => {
                if let Some(e) = expr {
                    self.lower_expr(e)?;
                }
                self.current_body.push(Instruction::Return);
            }
            TypedStatement::Call {
                name,
                arguments,
                is_user_defined,
            } => {
                for arg in arguments {
                    self.lower_expr(arg)?;
                }
                if *is_user_defined {
                    self.current_body.push(Instruction::CallUser(name.clone()));
                } else {
                    self.current_body.push(Instruction::CallHost(name.clone()));
                }
                // Drop result if any
                self.current_body.push(Instruction::Drop);
            }
            TypedStatement::Empty => {}
        }
        Ok(())
    }

    fn lower_if(
        &mut self,
        condition: &TypedExpr,
        then_branch: &[TypedStatement],
        elsif_branches: &[(TypedExpr, Vec<TypedStatement>)],
        else_branch: &Option<Vec<TypedStatement>>,
    ) -> Result<()> {
        // Evaluate condition
        self.lower_expr(condition)?;

        // If block
        self.current_body.push(Instruction::If);

        // Then branch
        for stmt in then_branch {
            self.lower_statement(stmt)?;
        }

        // Handle elsif branches
        for (cond, stmts) in elsif_branches {
            self.current_body.push(Instruction::Else);
            self.lower_expr(cond)?;
            self.current_body.push(Instruction::If);
            for stmt in stmts {
                self.lower_statement(stmt)?;
            }
        }

        // Else branch
        if let Some(stmts) = else_branch {
            self.current_body.push(Instruction::Else);
            for stmt in stmts {
                self.lower_statement(stmt)?;
            }
        }

        // Close all nested ifs
        for _ in 0..=elsif_branches.len() {
            self.current_body.push(Instruction::End);
        }

        Ok(())
    }

    fn lower_for(
        &mut self,
        var_offset: usize,
        from: &TypedExpr,
        to: &TypedExpr,
        by: Option<&TypedExpr>,
        body: &[TypedStatement],
    ) -> Result<()> {
        // Initialize loop variable: var := from
        self.current_body.push(Instruction::I32Const(0)); // Base address
        self.lower_expr(from)?;
        self.current_body.push(Instruction::I32Store {
            offset: var_offset as u32,
        });

        // Block for break
        self.current_body.push(Instruction::Block);
        // Loop for iteration
        self.current_body.push(Instruction::Loop);
        self.loop_depth += 1;

        // Check condition: var <= to
        self.current_body.push(Instruction::I32Const(0));
        self.current_body.push(Instruction::I32Load {
            offset: var_offset as u32,
        });
        self.lower_expr(to)?;
        self.current_body.push(Instruction::I32GtS);
        self.current_body.push(Instruction::BrIf(1)); // Break if var > to

        // Body
        for stmt in body {
            self.lower_statement(stmt)?;
        }

        // Increment: var := var + by
        self.current_body.push(Instruction::I32Const(0));
        self.current_body.push(Instruction::I32Const(0));
        self.current_body.push(Instruction::I32Load {
            offset: var_offset as u32,
        });
        if let Some(step) = by {
            self.lower_expr(step)?;
        } else {
            self.current_body.push(Instruction::I32Const(1));
        }
        self.current_body.push(Instruction::I32Add);
        self.current_body.push(Instruction::I32Store {
            offset: var_offset as u32,
        });

        // Loop back
        self.current_body.push(Instruction::Br(0));

        self.loop_depth -= 1;
        self.current_body.push(Instruction::End); // End loop
        self.current_body.push(Instruction::End); // End block

        Ok(())
    }

    fn lower_while(&mut self, condition: &TypedExpr, body: &[TypedStatement]) -> Result<()> {
        self.current_body.push(Instruction::Block);
        self.current_body.push(Instruction::Loop);
        self.loop_depth += 1;

        // Check condition
        self.lower_expr(condition)?;
        self.current_body.push(Instruction::I32Eqz);
        self.current_body.push(Instruction::BrIf(1)); // Break if false

        // Body
        for stmt in body {
            self.lower_statement(stmt)?;
        }

        // Loop back
        self.current_body.push(Instruction::Br(0));

        self.loop_depth -= 1;
        self.current_body.push(Instruction::End);
        self.current_body.push(Instruction::End);

        Ok(())
    }

    fn lower_repeat(&mut self, body: &[TypedStatement], until: &TypedExpr) -> Result<()> {
        self.current_body.push(Instruction::Block);
        self.current_body.push(Instruction::Loop);
        self.loop_depth += 1;

        // Body first
        for stmt in body {
            self.lower_statement(stmt)?;
        }

        // Check until condition
        self.lower_expr(until)?;
        self.current_body.push(Instruction::BrIf(1)); // Break if true

        // Loop back
        self.current_body.push(Instruction::Br(0));

        self.loop_depth -= 1;
        self.current_body.push(Instruction::End);
        self.current_body.push(Instruction::End);

        Ok(())
    }

    fn lower_case(
        &mut self,
        selector: &TypedExpr,
        branches: &[(Vec<i64>, Vec<TypedStatement>)],
        else_branch: &Option<Vec<TypedStatement>>,
    ) -> Result<()> {
        // Allocate a temp local for the selector value
        let selector_local = self.alloc_temp_local(WasmType::I32);

        // Evaluate selector once and store in local
        self.lower_expr(selector)?;
        self.current_body
            .push(Instruction::LocalSet(selector_local));

        // Filter branches with values
        let valid_branches: Vec<_> = branches
            .iter()
            .filter(|(values, _)| !values.is_empty())
            .collect();

        // Generate proper if-else-if chain
        // Structure: if (cond1) { branch1 } else { if (cond2) { branch2 } else { ... else_branch } }
        for (i, (values, stmts)) in valid_branches.iter().enumerate() {
            // Build condition: selector == val1 || selector == val2 || ...
            for (j, val) in values.iter().enumerate() {
                self.current_body
                    .push(Instruction::LocalGet(selector_local));
                self.current_body.push(Instruction::I32Const(*val as i32));
                self.current_body.push(Instruction::I32Eq);

                // OR with previous comparisons if not the first value
                if j > 0 {
                    self.current_body.push(Instruction::I32Or);
                }
            }

            self.current_body.push(Instruction::If);

            for stmt in stmts.iter() {
                self.lower_statement(stmt)?;
            }

            // If not the last branch, emit Else for the next branch
            if i < valid_branches.len() - 1 {
                self.current_body.push(Instruction::Else);
            }
        }

        // Handle else branch
        if let Some(stmts) = else_branch {
            if !valid_branches.is_empty() {
                self.current_body.push(Instruction::Else);
            }
            for stmt in stmts {
                self.lower_statement(stmt)?;
            }
        }

        // Close all nested if blocks
        for _ in 0..valid_branches.len() {
            self.current_body.push(Instruction::End);
        }

        Ok(())
    }

    fn lower_expr(&mut self, expr: &TypedExpr) -> Result<()> {
        match &expr.kind {
            TypedExprKind::Literal(lit) => self.lower_literal(lit),
            TypedExprKind::Variable { offset, .. } => {
                self.current_body.push(Instruction::I32Const(0)); // Base
                self.emit_load(&expr.ty, *offset as u32)?;
            }
            TypedExprKind::ArrayAccess {
                array,
                index,
                element_size,
            } => {
                // base + index * element_size
                self.push_address(array)?;
                self.lower_expr(index)?;
                self.current_body
                    .push(Instruction::I32Const(*element_size as i32));
                self.current_body.push(Instruction::I32Mul);
                self.current_body.push(Instruction::I32Add);
                self.emit_load(&expr.ty, 0)?;
            }
            TypedExprKind::FieldAccess {
                object,
                field_offset,
                ..
            } => {
                self.push_address(object)?;
                self.emit_load(&expr.ty, *field_offset as u32)?;
            }
            TypedExprKind::Binary { left, op, right } => {
                self.lower_expr(left)?;
                self.lower_expr(right)?;
                self.emit_binary_op(*op, &left.ty)?;
            }
            TypedExprKind::Unary { op, operand } => {
                self.lower_expr(operand)?;
                self.emit_unary_op(*op, &operand.ty)?;
            }
            TypedExprKind::Call {
                name,
                arguments,
                is_user_defined,
            } => {
                for arg in arguments {
                    self.lower_expr(arg)?;
                }
                if *is_user_defined {
                    self.current_body.push(Instruction::CallUser(name.clone()));
                } else {
                    self.current_body.push(Instruction::CallHost(name.clone()));
                }
            }
        }
        Ok(())
    }

    fn lower_literal(&mut self, lit: &TypedLiteral) {
        match lit {
            TypedLiteral::Bool(v) => {
                self.current_body
                    .push(Instruction::I32Const(if *v { 1 } else { 0 }));
            }
            TypedLiteral::Integer(v, bits) => {
                if *bits <= 32 {
                    self.current_body.push(Instruction::I32Const(*v as i32));
                } else {
                    self.current_body.push(Instruction::I64Const(*v));
                }
            }
            TypedLiteral::Real32(v) => {
                self.current_body.push(Instruction::F32Const(*v));
            }
            TypedLiteral::Real64(v) => {
                self.current_body.push(Instruction::F64Const(*v));
            }
            TypedLiteral::Time(ns) => {
                self.current_body.push(Instruction::I64Const(*ns));
            }
        }
    }

    fn push_address(&mut self, expr: &TypedExpr) -> Result<()> {
        match &expr.kind {
            TypedExprKind::Variable { offset, .. } => {
                self.current_body
                    .push(Instruction::I32Const(*offset as i32));
            }
            TypedExprKind::ArrayAccess {
                array,
                index,
                element_size,
            } => {
                self.push_address(array)?;
                self.lower_expr(index)?;
                self.current_body
                    .push(Instruction::I32Const(*element_size as i32));
                self.current_body.push(Instruction::I32Mul);
                self.current_body.push(Instruction::I32Add);
            }
            _ => {
                self.current_body.push(Instruction::I32Const(0));
            }
        }
        Ok(())
    }

    fn emit_load(&mut self, ty: &DataType, offset: u32) -> Result<()> {
        match ty {
            DataType::Bool | DataType::Sint | DataType::Usint | DataType::Byte => {
                self.current_body.push(Instruction::I32Load8S { offset });
            }
            DataType::Int | DataType::Uint | DataType::Word => {
                self.current_body.push(Instruction::I32Load16S { offset });
            }
            DataType::Dint | DataType::Udint | DataType::Dword => {
                self.current_body.push(Instruction::I32Load { offset });
            }
            DataType::Lint | DataType::Ulint | DataType::Lword | DataType::Time => {
                self.current_body.push(Instruction::I64Load { offset });
            }
            DataType::Real => {
                self.current_body.push(Instruction::F32Load { offset });
            }
            DataType::Lreal => {
                self.current_body.push(Instruction::F64Load { offset });
            }
            _ => {
                self.current_body.push(Instruction::I32Load { offset });
            }
        }
        Ok(())
    }

    fn emit_store(&mut self, ty: &DataType) -> Result<()> {
        match ty {
            DataType::Bool | DataType::Sint | DataType::Usint | DataType::Byte => {
                self.current_body.push(Instruction::I32Store8 { offset: 0 });
            }
            DataType::Int | DataType::Uint | DataType::Word => {
                self.current_body.push(Instruction::I32Store16 { offset: 0 });
            }
            DataType::Dint | DataType::Udint | DataType::Dword => {
                self.current_body.push(Instruction::I32Store { offset: 0 });
            }
            DataType::Lint | DataType::Ulint | DataType::Lword | DataType::Time => {
                self.current_body.push(Instruction::I64Store { offset: 0 });
            }
            DataType::Real => {
                self.current_body.push(Instruction::F32Store { offset: 0 });
            }
            DataType::Lreal => {
                self.current_body.push(Instruction::F64Store { offset: 0 });
            }
            _ => {
                self.current_body.push(Instruction::I32Store { offset: 0 });
            }
        }
        Ok(())
    }

    fn emit_binary_op(&mut self, op: BinaryOp, ty: &DataType) -> Result<()> {
        let is_float = matches!(ty, DataType::Real | DataType::Lreal);
        let is_64 = matches!(
            ty,
            DataType::Lint | DataType::Ulint | DataType::Lreal | DataType::Time
        );

        match op {
            BinaryOp::Add => {
                if is_float {
                    if is_64 {
                        self.current_body.push(Instruction::F64Add);
                    } else {
                        self.current_body.push(Instruction::F32Add);
                    }
                } else if is_64 {
                    self.current_body.push(Instruction::I64Add);
                } else {
                    self.current_body.push(Instruction::I32Add);
                }
            }
            BinaryOp::Sub => {
                if is_float {
                    if is_64 {
                        self.current_body.push(Instruction::F64Sub);
                    } else {
                        self.current_body.push(Instruction::F32Sub);
                    }
                } else if is_64 {
                    self.current_body.push(Instruction::I64Sub);
                } else {
                    self.current_body.push(Instruction::I32Sub);
                }
            }
            BinaryOp::Mul => {
                if is_float {
                    if is_64 {
                        self.current_body.push(Instruction::F64Mul);
                    } else {
                        self.current_body.push(Instruction::F32Mul);
                    }
                } else if is_64 {
                    self.current_body.push(Instruction::I64Mul);
                } else {
                    self.current_body.push(Instruction::I32Mul);
                }
            }
            BinaryOp::Div => {
                if is_float {
                    if is_64 {
                        self.current_body.push(Instruction::F64Div);
                    } else {
                        self.current_body.push(Instruction::F32Div);
                    }
                } else if is_64 {
                    self.current_body.push(Instruction::I64DivS);
                } else {
                    self.current_body.push(Instruction::I32DivS);
                }
            }
            BinaryOp::Mod => {
                self.current_body.push(Instruction::I32RemS);
            }
            BinaryOp::Eq => {
                if is_float {
                    if is_64 {
                        self.current_body.push(Instruction::F64Eq);
                    } else {
                        self.current_body.push(Instruction::F32Eq);
                    }
                } else {
                    self.current_body.push(Instruction::I32Eq);
                }
            }
            BinaryOp::Ne => {
                self.current_body.push(Instruction::I32Ne);
            }
            BinaryOp::Lt => {
                if is_float {
                    self.current_body.push(Instruction::F32Lt);
                } else {
                    self.current_body.push(Instruction::I32LtS);
                }
            }
            BinaryOp::Le => {
                self.current_body.push(Instruction::I32LeS);
            }
            BinaryOp::Gt => {
                if is_float {
                    self.current_body.push(Instruction::F32Gt);
                } else {
                    self.current_body.push(Instruction::I32GtS);
                }
            }
            BinaryOp::Ge => {
                self.current_body.push(Instruction::I32GeS);
            }
            BinaryOp::And | BinaryOp::BitAnd => {
                self.current_body.push(Instruction::I32And);
            }
            BinaryOp::Or | BinaryOp::BitOr => {
                self.current_body.push(Instruction::I32Or);
            }
            BinaryOp::Xor | BinaryOp::BitXor => {
                self.current_body.push(Instruction::I32Xor);
            }
            BinaryOp::Shl => {
                self.current_body.push(Instruction::I32Shl);
            }
            BinaryOp::Shr => {
                self.current_body.push(Instruction::I32ShrS);
            }
            BinaryOp::Pow => {
                // Power not directly supported, would need runtime function
                self.current_body.push(Instruction::I32Const(1));
            }
        }
        Ok(())
    }

    fn emit_unary_op(&mut self, op: UnaryOp, ty: &DataType) -> Result<()> {
        match op {
            UnaryOp::Neg => {
                match ty {
                    DataType::Real => {
                        self.current_body.push(Instruction::F32Const(-1.0));
                        self.current_body.push(Instruction::F32Mul);
                    }
                    DataType::Lreal => {
                        self.current_body.push(Instruction::F64Const(-1.0));
                        self.current_body.push(Instruction::F64Mul);
                    }
                    _ => {
                        // Negate: 0 - value
                        self.current_body.push(Instruction::I32Const(0));
                        // Swap order
                        self.current_body.push(Instruction::I32Sub);
                    }
                }
            }
            UnaryOp::Not => {
                // NOT: xor with 1
                self.current_body.push(Instruction::I32Const(1));
                self.current_body.push(Instruction::I32Xor);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::parse;
    use crate::typechecker::check;

    #[test]
    fn test_lower_simple_program() {
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
        let result = lower(&typed);

        assert!(result.is_ok(), "IR lowering failed: {:?}", result.err());

        let module = result.unwrap();
        assert_eq!(module.functions.len(), 1);
        assert!(module.functions[0].is_step);
    }
}
