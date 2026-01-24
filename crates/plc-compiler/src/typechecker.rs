//! Type checker for IEC 61131-3 Structured Text.
//!
//! Performs semantic analysis on the AST:
//! - Type checking and inference
//! - Scope analysis and variable resolution
//! - Type coercion for numeric operations

use crate::frontend::{
    BinaryOp, CaseStatement, CompilationUnit, DataType, Expression, ForStatement, Function,
    FunctionBlock, IfStatement, Literal, Program, ProgramUnit, RepeatStatement, Spanned,
    Statement, UnaryOp, VarBlock, VarBlockKind, VarDecl, WhileStatement,
};
use anyhow::{anyhow, Result};
use std::collections::HashMap;

/// Information about a function signature.
#[derive(Debug, Clone)]
pub struct FunctionSignature {
    /// Function name.
    pub name: String,
    /// Return type.
    pub return_type: DataType,
    /// Parameter types.
    pub params: Vec<DataType>,
    /// Whether this is a user-defined function (vs host import).
    pub is_user_defined: bool,
}

/// A typed compilation unit.
#[derive(Debug, Clone)]
pub struct TypedUnit {
    /// Typed program units.
    pub units: Vec<TypedPou>,
    /// Function signatures for all defined functions.
    pub functions: HashMap<String, FunctionSignature>,
}

/// A typed Program Organization Unit.
#[derive(Debug, Clone)]
pub enum TypedPou {
    /// Typed program.
    Program(TypedProgram),
    /// Typed function block.
    FunctionBlock(TypedFunctionBlock),
    /// Typed function.
    Function(TypedFunction),
}

/// A typed program.
#[derive(Debug, Clone)]
pub struct TypedProgram {
    /// Program name.
    pub name: String,
    /// Symbol table for this program.
    pub symbols: SymbolTable,
    /// Typed statements.
    pub body: Vec<TypedStatement>,
}

/// A typed function block.
#[derive(Debug, Clone)]
pub struct TypedFunctionBlock {
    /// Function block name.
    pub name: String,
    /// Symbol table.
    pub symbols: SymbolTable,
    /// Typed statements.
    pub body: Vec<TypedStatement>,
}

/// A typed function.
#[derive(Debug, Clone)]
pub struct TypedFunction {
    /// Function name.
    pub name: String,
    /// Return type.
    pub return_type: DataType,
    /// Symbol table.
    pub symbols: SymbolTable,
    /// Typed statements.
    pub body: Vec<TypedStatement>,
}

/// Symbol table for a scope.
#[derive(Debug, Clone, Default)]
pub struct SymbolTable {
    /// Variables indexed by name.
    pub variables: HashMap<String, SymbolInfo>,
    /// Variable layout for code generation.
    pub layout: Vec<VarLayout>,
}

/// Information about a symbol.
#[derive(Debug, Clone)]
pub struct SymbolInfo {
    /// Variable name.
    pub name: String,
    /// Data type.
    pub data_type: DataType,
    /// Variable kind (input, output, local, etc.).
    pub kind: VarBlockKind,
    /// Byte offset in memory.
    pub offset: usize,
    /// Size in bytes.
    pub size: usize,
    /// Whether it's a constant.
    pub constant: bool,
}

/// Layout information for a variable.
#[derive(Debug, Clone)]
pub struct VarLayout {
    /// Variable name.
    pub name: String,
    /// Byte offset.
    pub offset: usize,
    /// Size in bytes.
    pub size: usize,
}

/// A typed statement.
#[derive(Debug, Clone)]
pub enum TypedStatement {
    /// Assignment with resolved types.
    Assignment {
        /// Target variable info.
        target: TypedExpr,
        /// Source value with type.
        value: TypedExpr,
    },
    /// If statement.
    If {
        /// Condition (must be BOOL).
        condition: TypedExpr,
        /// Then branch.
        then_branch: Vec<TypedStatement>,
        /// Elsif branches.
        elsif_branches: Vec<(TypedExpr, Vec<TypedStatement>)>,
        /// Else branch.
        else_branch: Option<Vec<TypedStatement>>,
    },
    /// For loop.
    For {
        /// Loop variable name.
        variable: String,
        /// Variable offset.
        var_offset: usize,
        /// Start value.
        from: TypedExpr,
        /// End value.
        to: TypedExpr,
        /// Step value.
        by: Option<TypedExpr>,
        /// Loop body.
        body: Vec<TypedStatement>,
    },
    /// While loop.
    While {
        /// Condition.
        condition: TypedExpr,
        /// Body.
        body: Vec<TypedStatement>,
    },
    /// Repeat loop.
    Repeat {
        /// Body.
        body: Vec<TypedStatement>,
        /// Until condition.
        until: TypedExpr,
    },
    /// Case statement.
    Case {
        /// Selector expression.
        selector: TypedExpr,
        /// Branches: (values, statements).
        branches: Vec<(Vec<i64>, Vec<TypedStatement>)>,
        /// Else branch.
        else_branch: Option<Vec<TypedStatement>>,
    },
    /// Exit loop.
    Exit,
    /// Continue loop.
    Continue,
    /// Return from function.
    Return(Option<TypedExpr>),
    /// Function/FB call.
    Call {
        /// Callee name.
        name: String,
        /// Arguments.
        arguments: Vec<TypedExpr>,
        /// Whether this is a user-defined function (vs host import).
        is_user_defined: bool,
    },
    /// Empty statement.
    Empty,
}

/// A typed expression with its resolved type.
#[derive(Debug, Clone)]
pub struct TypedExpr {
    /// The expression kind.
    pub kind: TypedExprKind,
    /// Resolved type.
    pub ty: DataType,
}

/// Typed expression kinds.
#[derive(Debug, Clone)]
pub enum TypedExprKind {
    /// Literal value.
    Literal(TypedLiteral),
    /// Variable reference.
    Variable {
        /// Variable name.
        name: String,
        /// Offset in memory.
        offset: usize,
    },
    /// Array access.
    ArrayAccess {
        /// Base array.
        array: Box<TypedExpr>,
        /// Index.
        index: Box<TypedExpr>,
        /// Element size.
        element_size: usize,
    },
    /// Field access.
    FieldAccess {
        /// Object.
        object: Box<TypedExpr>,
        /// Field name.
        field: String,
        /// Field offset.
        field_offset: usize,
    },
    /// Binary operation.
    Binary {
        /// Left operand.
        left: Box<TypedExpr>,
        /// Operator.
        op: BinaryOp,
        /// Right operand.
        right: Box<TypedExpr>,
    },
    /// Unary operation.
    Unary {
        /// Operator.
        op: UnaryOp,
        /// Operand.
        operand: Box<TypedExpr>,
    },
    /// Function call.
    Call {
        /// Function name.
        name: String,
        /// Arguments.
        arguments: Vec<TypedExpr>,
        /// Whether this is a user-defined function (vs host import).
        is_user_defined: bool,
    },
}

/// Typed literal values.
#[derive(Debug, Clone)]
pub enum TypedLiteral {
    /// Boolean.
    Bool(bool),
    /// Integer (with bit width).
    Integer(i64, u8),
    /// Real (32-bit).
    Real32(f32),
    /// Real (64-bit).
    Real64(f64),
    /// Time in nanoseconds.
    Time(i64),
}

/// Type check a compilation unit.
pub fn check(ast: &CompilationUnit) -> Result<TypedUnit> {
    let mut checker = TypeChecker::new();
    checker.check_unit(ast)
}

/// Type checker implementation.
struct TypeChecker {
    /// Current scope's symbol table.
    symbols: SymbolTable,
    /// Next available byte offset for variables.
    next_offset: usize,
    /// Known function signatures (both user-defined and host imports).
    functions: HashMap<String, FunctionSignature>,
}

impl TypeChecker {
    fn new() -> Self {
        let mut functions = HashMap::new();

        // Register built-in host functions
        functions.insert(
            "read_di".to_string(),
            FunctionSignature {
                name: "read_di".to_string(),
                return_type: DataType::Int,
                params: vec![DataType::Int],
                is_user_defined: false,
            },
        );
        functions.insert(
            "write_do".to_string(),
            FunctionSignature {
                name: "write_do".to_string(),
                return_type: DataType::Bool, // void, but we use Bool
                params: vec![DataType::Int, DataType::Int],
                is_user_defined: false,
            },
        );
        functions.insert(
            "read_ai".to_string(),
            FunctionSignature {
                name: "read_ai".to_string(),
                return_type: DataType::Int,
                params: vec![DataType::Int],
                is_user_defined: false,
            },
        );
        functions.insert(
            "write_ao".to_string(),
            FunctionSignature {
                name: "write_ao".to_string(),
                return_type: DataType::Bool,
                params: vec![DataType::Int, DataType::Int],
                is_user_defined: false,
            },
        );
        functions.insert(
            "get_cycle_time".to_string(),
            FunctionSignature {
                name: "get_cycle_time".to_string(),
                return_type: DataType::Int,
                params: vec![],
                is_user_defined: false,
            },
        );

        Self {
            symbols: SymbolTable::default(),
            // Start after process image area (0x100 bytes reserved)
            next_offset: 0x100,
            functions,
        }
    }

    fn check_unit(&mut self, ast: &CompilationUnit) -> Result<TypedUnit> {
        // First pass: collect all function signatures
        for spanned_unit in &ast.units {
            match &spanned_unit.node {
                ProgramUnit::Function(f) => {
                    let params: Vec<DataType> = f
                        .variables
                        .iter()
                        .filter(|vb| vb.node.kind == VarBlockKind::Input)
                        .flat_map(|vb| vb.node.declarations.iter())
                        .map(|d| d.node.data_type.clone())
                        .collect();

                    self.functions.insert(
                        f.name.clone(),
                        FunctionSignature {
                            name: f.name.clone(),
                            return_type: f.return_type.clone(),
                            params,
                            is_user_defined: true,
                        },
                    );
                }
                ProgramUnit::FunctionBlock(fb) => {
                    // Function blocks are also callable
                    self.functions.insert(
                        fb.name.clone(),
                        FunctionSignature {
                            name: fb.name.clone(),
                            return_type: DataType::Bool, // FBs don't return values directly
                            params: vec![],
                            is_user_defined: true,
                        },
                    );
                }
                ProgramUnit::Program(_) => {}
            }
        }

        // Second pass: type check all units
        let mut units = Vec::new();
        for spanned_unit in &ast.units {
            let typed = match &spanned_unit.node {
                ProgramUnit::Program(p) => TypedPou::Program(self.check_program(p)?),
                ProgramUnit::FunctionBlock(fb) => {
                    TypedPou::FunctionBlock(self.check_function_block(fb)?)
                }
                ProgramUnit::Function(f) => TypedPou::Function(self.check_function(f)?),
            };
            units.push(typed);
        }

        Ok(TypedUnit {
            units,
            functions: self.functions.clone(),
        })
    }

    fn check_program(&mut self, program: &Program) -> Result<TypedProgram> {
        self.symbols = SymbolTable::default();
        self.next_offset = 0x100;

        // Register variables
        for var_block in &program.variables {
            self.register_var_block(&var_block.node)?;
        }

        // Type check body
        let body = self.check_statements(&program.body)?;

        Ok(TypedProgram {
            name: program.name.clone(),
            symbols: self.symbols.clone(),
            body,
        })
    }

    fn check_function_block(&mut self, fb: &FunctionBlock) -> Result<TypedFunctionBlock> {
        self.symbols = SymbolTable::default();
        self.next_offset = 0x100;

        for var_block in &fb.variables {
            self.register_var_block(&var_block.node)?;
        }

        let body = self.check_statements(&fb.body)?;

        Ok(TypedFunctionBlock {
            name: fb.name.clone(),
            symbols: self.symbols.clone(),
            body,
        })
    }

    fn check_function(&mut self, func: &Function) -> Result<TypedFunction> {
        self.symbols = SymbolTable::default();
        self.next_offset = 0x100;

        // Register return value as a variable
        let ret_size = func.return_type.size_bytes().unwrap_or(4);
        self.symbols.variables.insert(
            func.name.clone(),
            SymbolInfo {
                name: func.name.clone(),
                data_type: func.return_type.clone(),
                kind: VarBlockKind::Var,
                offset: self.next_offset,
                size: ret_size,
                constant: false,
            },
        );
        self.next_offset += ret_size;

        for var_block in &func.variables {
            self.register_var_block(&var_block.node)?;
        }

        let body = self.check_statements(&func.body)?;

        Ok(TypedFunction {
            name: func.name.clone(),
            return_type: func.return_type.clone(),
            symbols: self.symbols.clone(),
            body,
        })
    }

    fn register_var_block(&mut self, block: &VarBlock) -> Result<()> {
        for decl in &block.declarations {
            self.register_variable(&decl.node, block.kind, block.constant)?;
        }
        Ok(())
    }

    fn register_variable(
        &mut self,
        decl: &VarDecl,
        kind: VarBlockKind,
        constant: bool,
    ) -> Result<()> {
        let size = decl.data_type.size_bytes().unwrap_or(4);

        // Align offset
        let alignment = size.min(8);
        self.next_offset = (self.next_offset + alignment - 1) & !(alignment - 1);

        let info = SymbolInfo {
            name: decl.name.clone(),
            data_type: decl.data_type.clone(),
            kind,
            offset: self.next_offset,
            size,
            constant,
        };

        self.symbols.layout.push(VarLayout {
            name: decl.name.clone(),
            offset: self.next_offset,
            size,
        });

        self.symbols.variables.insert(decl.name.clone(), info);
        self.next_offset += size;

        Ok(())
    }

    fn check_statements(
        &mut self,
        statements: &[Spanned<Statement>],
    ) -> Result<Vec<TypedStatement>> {
        let mut typed = Vec::new();
        for stmt in statements {
            if let Some(t) = self.check_statement(&stmt.node)? {
                typed.push(t);
            }
        }
        Ok(typed)
    }

    fn check_statement(&mut self, stmt: &Statement) -> Result<Option<TypedStatement>> {
        match stmt {
            Statement::Assignment(assign) => {
                let target = self.check_expr(&assign.target.node)?;
                let value = self.check_expr(&assign.value.node)?;

                // Type compatibility check
                self.check_assignment_types(&target.ty, &value.ty)?;

                Ok(Some(TypedStatement::Assignment { target, value }))
            }
            Statement::If(if_stmt) => self.check_if(if_stmt),
            Statement::For(for_stmt) => self.check_for(for_stmt),
            Statement::While(while_stmt) => self.check_while(while_stmt),
            Statement::Repeat(repeat_stmt) => self.check_repeat(repeat_stmt),
            Statement::Case(case_stmt) => self.check_case(case_stmt),
            Statement::Exit => Ok(Some(TypedStatement::Exit)),
            Statement::Continue => Ok(Some(TypedStatement::Continue)),
            Statement::Return(expr) => {
                let typed_expr = expr
                    .as_ref()
                    .map(|e| self.check_expr(&e.node))
                    .transpose()?;
                Ok(Some(TypedStatement::Return(typed_expr)))
            }
            Statement::Call(call) => {
                let args: Result<Vec<_>> = call
                    .arguments
                    .iter()
                    .map(|a| self.check_expr(&a.value.node))
                    .collect();

                // Look up function to determine if user-defined or host
                let is_user_defined = self
                    .functions
                    .get(&call.name)
                    .map(|f| f.is_user_defined)
                    .unwrap_or(false);

                Ok(Some(TypedStatement::Call {
                    name: call.name.clone(),
                    arguments: args?,
                    is_user_defined,
                }))
            }
            Statement::Empty => Ok(Some(TypedStatement::Empty)),
        }
    }

    fn check_if(&mut self, if_stmt: &IfStatement) -> Result<Option<TypedStatement>> {
        let condition = self.check_expr(&if_stmt.condition.node)?;
        self.expect_bool(&condition.ty)?;

        let then_branch = self.check_statements(&if_stmt.then_branch)?;

        let elsif_branches: Result<Vec<_>> = if_stmt
            .elsif_branches
            .iter()
            .map(|branch| {
                let cond = self.check_expr(&branch.condition.node)?;
                self.expect_bool(&cond.ty)?;
                let stmts = self.check_statements(&branch.statements)?;
                Ok((cond, stmts))
            })
            .collect();

        let else_branch = if_stmt
            .else_branch
            .as_ref()
            .map(|stmts| self.check_statements(stmts))
            .transpose()?;

        Ok(Some(TypedStatement::If {
            condition,
            then_branch,
            elsif_branches: elsif_branches?,
            else_branch,
        }))
    }

    fn check_for(&mut self, for_stmt: &ForStatement) -> Result<Option<TypedStatement>> {
        let var_info = self
            .symbols
            .variables
            .get(&for_stmt.variable)
            .ok_or_else(|| anyhow!("Undefined variable: {}", for_stmt.variable))?
            .clone();

        let from = self.check_expr(&for_stmt.from.node)?;
        let to = self.check_expr(&for_stmt.to.node)?;
        let by = for_stmt
            .by
            .as_ref()
            .map(|e| self.check_expr(&e.node))
            .transpose()?;

        let body = self.check_statements(&for_stmt.body)?;

        Ok(Some(TypedStatement::For {
            variable: for_stmt.variable.clone(),
            var_offset: var_info.offset,
            from,
            to,
            by,
            body,
        }))
    }

    fn check_while(&mut self, while_stmt: &WhileStatement) -> Result<Option<TypedStatement>> {
        let condition = self.check_expr(&while_stmt.condition.node)?;
        self.expect_bool(&condition.ty)?;

        let body = self.check_statements(&while_stmt.body)?;

        Ok(Some(TypedStatement::While { condition, body }))
    }

    fn check_repeat(&mut self, repeat_stmt: &RepeatStatement) -> Result<Option<TypedStatement>> {
        let body = self.check_statements(&repeat_stmt.body)?;

        let until = self.check_expr(&repeat_stmt.until.node)?;
        self.expect_bool(&until.ty)?;

        Ok(Some(TypedStatement::Repeat { body, until }))
    }

    fn check_case(&mut self, case_stmt: &CaseStatement) -> Result<Option<TypedStatement>> {
        let selector = self.check_expr(&case_stmt.selector.node)?;

        let branches: Result<Vec<_>> = case_stmt
            .branches
            .iter()
            .map(|branch| {
                let mut values: Vec<i64> = Vec::new();
                for v in &branch.values {
                    match v {
                        crate::frontend::CaseValue::Single(e) => {
                            values.push(self.expr_to_const(&e.node)?);
                        }
                        crate::frontend::CaseValue::Range(start, end) => {
                            // Return error for ranges until properly implemented
                            let start_val = self.expr_to_const(&start.node)?;
                            let end_val = self.expr_to_const(&end.node)?;
                            return Err(anyhow!(
                                "CASE ranges not yet supported: {}..{}",
                                start_val,
                                end_val
                            ));
                        }
                    }
                }
                let stmts = self.check_statements(&branch.statements)?;
                Ok((values, stmts))
            })
            .collect();

        let else_branch = case_stmt
            .else_branch
            .as_ref()
            .map(|stmts| self.check_statements(stmts))
            .transpose()?;

        Ok(Some(TypedStatement::Case {
            selector,
            branches: branches?,
            else_branch,
        }))
    }

    fn check_expr(&mut self, expr: &Expression) -> Result<TypedExpr> {
        match expr {
            Expression::Literal(lit) => self.check_literal(lit),
            Expression::Variable(name) => self.check_variable(name),
            Expression::ArrayAccess { array, index } => {
                let arr = self.check_expr(&array.node)?;
                let idx = self.check_expr(&index.node)?;

                let (elem_type, elem_size) = match &arr.ty {
                    DataType::Array { element_type, .. } => {
                        let size = element_type.size_bytes().unwrap_or(4);
                        (element_type.as_ref().clone(), size)
                    }
                    _ => return Err(anyhow!("Cannot index non-array type")),
                };

                Ok(TypedExpr {
                    kind: TypedExprKind::ArrayAccess {
                        array: Box::new(arr),
                        index: Box::new(idx),
                        element_size: elem_size,
                    },
                    ty: elem_type,
                })
            }
            Expression::FieldAccess { object, field } => {
                let obj = self.check_expr(&object.node)?;
                // Simplified: assume INT field type
                Ok(TypedExpr {
                    kind: TypedExprKind::FieldAccess {
                        object: Box::new(obj),
                        field: field.clone(),
                        field_offset: 0,
                    },
                    ty: DataType::Int,
                })
            }
            Expression::Binary { left, op, right } => {
                let l = self.check_expr(&left.node)?;
                let r = self.check_expr(&right.node)?;

                let result_type = self.binary_result_type(&l.ty, *op, &r.ty)?;

                Ok(TypedExpr {
                    kind: TypedExprKind::Binary {
                        left: Box::new(l),
                        op: *op,
                        right: Box::new(r),
                    },
                    ty: result_type,
                })
            }
            Expression::Unary { op, operand } => {
                let operand_typed = self.check_expr(&operand.node)?;
                let result_type = match op {
                    UnaryOp::Not => {
                        // NOT operator requires BOOL operand (logical negation)
                        // Note: IEC 61131-3 also allows NOT on bit types (BYTE, WORD, etc.)
                        // for bitwise negation, but we enforce BOOL-only for now
                        self.expect_bool(&operand_typed.ty)?;
                        DataType::Bool
                    }
                    UnaryOp::Neg => operand_typed.ty.clone(),
                };

                Ok(TypedExpr {
                    kind: TypedExprKind::Unary {
                        op: *op,
                        operand: Box::new(operand_typed),
                    },
                    ty: result_type,
                })
            }
            Expression::Call { name, arguments } => {
                let args: Result<Vec<_>> =
                    arguments.iter().map(|a| self.check_expr(&a.value.node)).collect();

                // Look up function signature - error if not found
                let func_sig = self
                    .functions
                    .get(name)
                    .ok_or_else(|| anyhow!("Unknown function: {}", name))?;

                Ok(TypedExpr {
                    kind: TypedExprKind::Call {
                        name: name.clone(),
                        arguments: args?,
                        is_user_defined: func_sig.is_user_defined,
                    },
                    ty: func_sig.return_type.clone(),
                })
            }
            Expression::Paren(inner) => self.check_expr(&inner.node),
        }
    }

    fn check_literal(&self, lit: &Literal) -> Result<TypedExpr> {
        match lit {
            Literal::Bool(v) => Ok(TypedExpr {
                kind: TypedExprKind::Literal(TypedLiteral::Bool(*v)),
                ty: DataType::Bool,
            }),
            Literal::Integer(v) => {
                let bits = if *v >= i8::MIN as i64 && *v <= i8::MAX as i64 {
                    8
                } else if *v >= i16::MIN as i64 && *v <= i16::MAX as i64 {
                    16
                } else if *v >= i32::MIN as i64 && *v <= i32::MAX as i64 {
                    32
                } else {
                    64
                };
                let ty = match bits {
                    8 => DataType::Sint,
                    16 => DataType::Int,
                    32 => DataType::Dint,
                    64 => DataType::Lint,
                    _ => DataType::Dint,
                };
                Ok(TypedExpr {
                    kind: TypedExprKind::Literal(TypedLiteral::Integer(*v, bits)),
                    ty,
                })
            }
            Literal::Real(v) => Ok(TypedExpr {
                kind: TypedExprKind::Literal(TypedLiteral::Real64(*v)),
                ty: DataType::Lreal,
            }),
            Literal::Time(ns) => Ok(TypedExpr {
                kind: TypedExprKind::Literal(TypedLiteral::Time(*ns)),
                ty: DataType::Time,
            }),
            Literal::String(s) => Ok(TypedExpr {
                kind: TypedExprKind::Literal(TypedLiteral::Bool(false)), // Placeholder
                ty: DataType::String(Some(s.len())),
            }),
            _ => Err(anyhow!("Unsupported literal type")),
        }
    }

    fn check_variable(&self, name: &str) -> Result<TypedExpr> {
        let info = self
            .symbols
            .variables
            .get(name)
            .ok_or_else(|| anyhow!("Undefined variable: {}", name))?;

        Ok(TypedExpr {
            kind: TypedExprKind::Variable {
                name: name.to_string(),
                offset: info.offset,
            },
            ty: info.data_type.clone(),
        })
    }

    fn binary_result_type(
        &self,
        left: &DataType,
        op: BinaryOp,
        right: &DataType,
    ) -> Result<DataType> {
        match op {
            // Comparison operators always return BOOL
            BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
                Ok(DataType::Bool)
            }
            // Logical operators require BOOL
            BinaryOp::And | BinaryOp::Or | BinaryOp::Xor => {
                if *left == DataType::Bool && *right == DataType::Bool {
                    Ok(DataType::Bool)
                } else {
                    Err(anyhow!("Logical operators require BOOL operands"))
                }
            }
            // Arithmetic operators - use wider type
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod | BinaryOp::Pow => {
                self.numeric_promotion(left, right)
            }
            // Bitwise operators
            BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::BitXor | BinaryOp::Shl | BinaryOp::Shr => {
                Ok(left.clone())
            }
        }
    }

    fn numeric_promotion(&self, left: &DataType, right: &DataType) -> Result<DataType> {
        // Simplified promotion rules
        if *left == DataType::Lreal || *right == DataType::Lreal {
            return Ok(DataType::Lreal);
        }
        if *left == DataType::Real || *right == DataType::Real {
            return Ok(DataType::Real);
        }
        if *left == DataType::Lint || *right == DataType::Lint {
            return Ok(DataType::Lint);
        }
        if *left == DataType::Dint || *right == DataType::Dint {
            return Ok(DataType::Dint);
        }
        Ok(DataType::Int)
    }

    fn check_assignment_types(&self, target: &DataType, source: &DataType) -> Result<()> {
        // Simplified: allow same types or numeric promotion
        if target == source {
            return Ok(());
        }
        if target.is_numeric() && source.is_numeric() {
            return Ok(());
        }
        Err(anyhow!(
            "Cannot assign {} to {}",
            source,
            target
        ))
    }

    fn expect_bool(&self, ty: &DataType) -> Result<()> {
        if *ty == DataType::Bool {
            Ok(())
        } else {
            Err(anyhow!("Expected BOOL, got {}", ty))
        }
    }

    fn expr_to_const(&self, expr: &Expression) -> Result<i64> {
        match expr {
            Expression::Literal(Literal::Integer(n)) => Ok(*n),
            _ => Err(anyhow!("Expected constant expression")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::parse;

    #[test]
    fn test_type_check_simple_program() {
        let source = r#"
            PROGRAM Main
            VAR
                x : INT := 0;
            END_VAR
                x := x + 1;
            END_PROGRAM
        "#;

        let ast = parse(source).unwrap();
        let result = check(&ast);
        assert!(result.is_ok(), "Type check failed: {:?}", result.err());
    }

    #[test]
    fn test_type_check_bool_condition() {
        let source = r#"
            PROGRAM Test
            VAR
                flag : BOOL;
                count : INT;
            END_VAR
                IF flag THEN
                    count := 1;
                END_IF;
            END_PROGRAM
        "#;

        let ast = parse(source).unwrap();
        let result = check(&ast);
        assert!(result.is_ok());
    }

    #[test]
    fn test_not_operator_requires_bool() {
        // Valid: NOT with BOOL operand
        let valid_source = r#"
            PROGRAM Test
            VAR
                flag : BOOL;
                result : BOOL;
            END_VAR
                result := NOT flag;
            END_PROGRAM
        "#;

        let ast = parse(valid_source).unwrap();
        let result = check(&ast);
        assert!(result.is_ok(), "NOT with BOOL should succeed");

        // Invalid: NOT with INT operand
        let invalid_source = r#"
            PROGRAM Test
            VAR
                count : INT;
                result : BOOL;
            END_VAR
                result := NOT count;
            END_PROGRAM
        "#;

        let ast = parse(invalid_source).unwrap();
        let result = check(&ast);
        assert!(result.is_err(), "NOT with INT should fail");
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("Expected BOOL"),
            "Error should mention BOOL, got: {}",
            err_msg
        );
    }

    #[test]
    fn test_unknown_function_error() {
        let source = r#"
            PROGRAM Test
            VAR
                x : INT;
            END_VAR
                x := unknown_func();
            END_PROGRAM
        "#;

        let ast = parse(source).unwrap();
        let result = check(&ast);
        assert!(result.is_err(), "Unknown function should fail");
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("Unknown function"),
            "Error should mention unknown function, got: {}",
            err_msg
        );
    }

    #[test]
    fn test_known_host_function_succeeds() {
        let source = r#"
            PROGRAM Test
            VAR
                x : INT;
            END_VAR
                x := read_di(0);
            END_PROGRAM
        "#;

        let ast = parse(source).unwrap();
        let result = check(&ast);
        assert!(result.is_ok(), "Known host function should succeed: {:?}", result.err());
    }
}
