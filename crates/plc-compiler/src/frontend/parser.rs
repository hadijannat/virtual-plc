//! Parser for IEC 61131-3 Structured Text using pest.
//!
//! Converts pest parse tree to our AST nodes.

use super::ast::*;
use anyhow::{anyhow, Result};
use pest::iterators::{Pair, Pairs};
use pest::Parser;
use pest_derive::Parser;

#[derive(Parser)]
#[grammar = "frontend/st.pest"]
struct StParser;

/// Helper trait for extracting the next element from a pest iterator with context.
trait PairsExt<'i> {
    /// Get the next pair, returning an error with context if missing.
    fn expect_next(&mut self, context: &str) -> Result<Pair<'i, Rule>>;
}

impl<'i> PairsExt<'i> for Pairs<'i, Rule> {
    fn expect_next(&mut self, context: &str) -> Result<Pair<'i, Rule>> {
        self.next()
            .ok_or_else(|| anyhow!("Parser error: expected {} but found end of input", context))
    }
}

/// Parse Structured Text source code into an AST.
pub fn parse(source: &str) -> Result<CompilationUnit> {
    let pairs = StParser::parse(Rule::compilation_unit, source)
        .map_err(|e| anyhow!("Parse error: {}", e))?;

    let mut units = Vec::new();
    for pair in pairs {
        if pair.as_rule() == Rule::compilation_unit {
            for inner in pair.into_inner() {
                match inner.as_rule() {
                    Rule::pou => {
                        let span = span_from_pair(&inner);
                        let unit = parse_pou(inner)?;
                        units.push(Spanned::new(unit, span));
                    }
                    Rule::EOI => {}
                    _ => {}
                }
            }
        }
    }

    Ok(CompilationUnit { units })
}

fn span_from_pair(pair: &Pair<Rule>) -> Span {
    let pest_span = pair.as_span();
    let (line, col) = pest_span.start_pos().line_col();
    Span::new(pest_span.start(), pest_span.end(), line, col)
}

fn parse_pou(pair: Pair<Rule>) -> Result<ProgramUnit> {
    let inner = pair
        .into_inner()
        .next()
        .ok_or_else(|| anyhow!("Expected program unit content"))?;
    match inner.as_rule() {
        Rule::program => Ok(ProgramUnit::Program(parse_program(inner)?)),
        Rule::function_block => Ok(ProgramUnit::FunctionBlock(parse_function_block(inner)?)),
        Rule::function => Ok(ProgramUnit::Function(parse_function(inner)?)),
        _ => Err(anyhow!("Unexpected POU type: {:?}", inner.as_rule())),
    }
}

fn parse_program(pair: Pair<Rule>) -> Result<Program> {
    let mut inner = pair.into_inner();
    let name = inner
        .expect_next("program name")?
        .as_str()
        .to_string();

    let mut variables = Vec::new();
    let mut body = Vec::new();

    for item in inner {
        match item.as_rule() {
            Rule::var_block => {
                let span = span_from_pair(&item);
                variables.push(Spanned::new(parse_var_block(item)?, span));
            }
            Rule::statement_list => {
                body = parse_statement_list(item)?;
            }
            _ => {}
        }
    }

    Ok(Program {
        name,
        variables,
        body,
    })
}

fn parse_function_block(pair: Pair<Rule>) -> Result<FunctionBlock> {
    let mut inner = pair.into_inner();
    let name = inner
        .expect_next("function block name")?
        .as_str()
        .to_string();

    let mut variables = Vec::new();
    let mut body = Vec::new();

    for item in inner {
        match item.as_rule() {
            Rule::var_block => {
                let span = span_from_pair(&item);
                variables.push(Spanned::new(parse_var_block(item)?, span));
            }
            Rule::statement_list => {
                body = parse_statement_list(item)?;
            }
            _ => {}
        }
    }

    Ok(FunctionBlock {
        name,
        variables,
        body,
    })
}

fn parse_function(pair: Pair<Rule>) -> Result<Function> {
    let mut inner = pair.into_inner();
    let name = inner
        .expect_next("function name")?
        .as_str()
        .to_string();
    let return_type = parse_data_type(inner.expect_next("function return type")?)?;

    let mut variables = Vec::new();
    let mut body = Vec::new();

    for item in inner {
        match item.as_rule() {
            Rule::var_block => {
                let span = span_from_pair(&item);
                variables.push(Spanned::new(parse_var_block(item)?, span));
            }
            Rule::statement_list => {
                body = parse_statement_list(item)?;
            }
            _ => {}
        }
    }

    Ok(Function {
        name,
        return_type,
        variables,
        body,
    })
}

fn parse_var_block(pair: Pair<Rule>) -> Result<VarBlock> {
    let mut inner = pair.into_inner();

    let kind_str = inner.expect_next("variable block kind")?.as_str().to_uppercase();
    let kind = match kind_str.as_str() {
        "VAR" => VarBlockKind::Var,
        "VAR_INPUT" => VarBlockKind::Input,
        "VAR_OUTPUT" => VarBlockKind::Output,
        "VAR_IN_OUT" => VarBlockKind::InOut,
        "VAR_EXTERNAL" => VarBlockKind::External,
        "VAR_GLOBAL" => VarBlockKind::Global,
        "VAR_TEMP" => VarBlockKind::Temp,
        _ => return Err(anyhow!("Unknown var block kind: {}", kind_str)),
    };

    let mut retain = false;
    let mut constant = false;
    let mut declarations = Vec::new();

    for item in inner {
        match item.as_rule() {
            Rule::var_modifier => {
                let modifier = item.as_str().to_uppercase();
                match modifier.as_str() {
                    "RETAIN" => retain = true,
                    "CONSTANT" => constant = true,
                    _ => {}
                }
            }
            Rule::var_decl => {
                let span = span_from_pair(&item);
                // parse_var_decl returns Vec<VarDecl> to handle comma-separated declarations
                let decls = parse_var_decl(item)?;
                for decl in decls {
                    declarations.push(Spanned::new(decl, span));
                }
            }
            _ => {}
        }
    }

    Ok(VarBlock {
        kind,
        retain,
        constant,
        declarations,
    })
}

/// Parse a variable declaration, which may declare multiple variables
/// with the same type (e.g., "a, b, c: INT := 0;").
fn parse_var_decl(pair: Pair<Rule>) -> Result<Vec<VarDecl>> {
    let mut inner = pair.into_inner();

    // Get identifier list - may contain multiple identifiers
    let id_list = inner.expect_next("identifier list")?;
    let names: Vec<String> = id_list
        .into_inner()
        .map(|p| p.as_str().to_string())
        .collect();

    if names.is_empty() {
        return Err(anyhow!("Variable declaration has no identifiers"));
    }

    let data_type = parse_data_type(inner.expect_next("variable data type")?)?;

    // Parse initial value with proper error handling
    let initial_value = match inner.next() {
        Some(p) => {
            let span = span_from_pair(&p);
            let expr = parse_expression(p)?;
            Some(Spanned::new(expr, span))
        }
        None => None,
    };

    // Create a VarDecl for each identifier
    let decls = names
        .into_iter()
        .map(|name| VarDecl {
            name,
            data_type: data_type.clone(),
            initial_value: initial_value.clone(),
            address: None,
        })
        .collect();

    Ok(decls)
}

fn parse_data_type(pair: Pair<Rule>) -> Result<DataType> {
    let inner = pair
        .into_inner()
        .next()
        .ok_or_else(|| anyhow!("Expected data type content"))?;
    match inner.as_rule() {
        Rule::elementary_type => {
            let type_str = inner.as_str().to_uppercase();
            Ok(match type_str.as_str() {
                "BOOL" => DataType::Bool,
                "SINT" => DataType::Sint,
                "INT" => DataType::Int,
                "DINT" => DataType::Dint,
                "LINT" => DataType::Lint,
                "USINT" => DataType::Usint,
                "UINT" => DataType::Uint,
                "UDINT" => DataType::Udint,
                "ULINT" => DataType::Ulint,
                "REAL" => DataType::Real,
                "LREAL" => DataType::Lreal,
                "TIME" => DataType::Time,
                "DATE" => DataType::Date,
                "TIME_OF_DAY" | "TOD" => DataType::TimeOfDay,
                "DATE_AND_TIME" | "DT" => DataType::DateTime,
                "BYTE" => DataType::Byte,
                "WORD" => DataType::Word,
                "DWORD" => DataType::Dword,
                "LWORD" => DataType::Lword,
                _ => return Err(anyhow!("Unknown elementary type: {}", type_str)),
            })
        }
        Rule::string_type => {
            let mut parts = inner.into_inner();
            let type_name = parts.next().map(|p| p.as_str().to_uppercase());
            let length = parts.next().and_then(|p| p.as_str().parse().ok());

            match type_name.as_deref() {
                Some("STRING") | None => Ok(DataType::String(length)),
                Some("WSTRING") => Ok(DataType::WString(length)),
                _ => Err(anyhow!("Unknown string type")),
            }
        }
        Rule::array_type => {
            let mut parts = inner.into_inner();
            let subrange = parts
                .next()
                .ok_or_else(|| anyhow!("Expected array subrange"))?;
            let mut subrange_inner = subrange.into_inner();
            let lower_pair = subrange_inner
                .next()
                .ok_or_else(|| anyhow!("Expected array lower bound"))?;
            let upper_pair = subrange_inner
                .next()
                .ok_or_else(|| anyhow!("Expected array upper bound"))?;
            let lower_expr = parse_expression(lower_pair)?;
            let upper_expr = parse_expression(upper_pair)?;

            // For now, require constant bounds
            let lower = expr_to_i64(&lower_expr)?;
            let upper = expr_to_i64(&upper_expr)?;

            let element_type_pair = parts
                .next()
                .ok_or_else(|| anyhow!("Expected array element type"))?;
            let element_type = parse_data_type(element_type_pair)?;

            Ok(DataType::Array {
                lower,
                upper,
                element_type: Box::new(element_type),
            })
        }
        Rule::identifier => Ok(DataType::Named(inner.as_str().to_string())),
        _ => Err(anyhow!("Unexpected data type: {:?}", inner.as_rule())),
    }
}

fn expr_to_i64(expr: &Expression) -> Result<i64> {
    match expr {
        Expression::Literal(Literal::Integer(n)) => Ok(*n),
        Expression::Unary {
            op: UnaryOp::Neg,
            operand,
        } => expr_to_i64(&operand.node).map(|n| -n),
        _ => Err(anyhow!("Expected constant integer expression")),
    }
}

fn parse_statement_list(pair: Pair<Rule>) -> Result<Vec<Spanned<Statement>>> {
    let mut statements = Vec::new();
    for item in pair.into_inner() {
        if item.as_rule() == Rule::statement {
            let span = span_from_pair(&item);
            if let Some(stmt) = parse_statement(item)? {
                statements.push(Spanned::new(stmt, span));
            }
        }
    }
    Ok(statements)
}

fn parse_statement(pair: Pair<Rule>) -> Result<Option<Statement>> {
    let inner = pair.into_inner().next();
    let inner = match inner {
        Some(i) => i,
        None => return Ok(None),
    };

    match inner.as_rule() {
        Rule::assignment_stmt => Ok(Some(parse_assignment(inner)?)),
        Rule::if_stmt => Ok(Some(parse_if(inner)?)),
        Rule::case_stmt => Ok(Some(parse_case(inner)?)),
        Rule::for_stmt => Ok(Some(parse_for(inner)?)),
        Rule::while_stmt => Ok(Some(parse_while(inner)?)),
        Rule::repeat_stmt => Ok(Some(parse_repeat(inner)?)),
        Rule::exit_stmt => Ok(Some(Statement::Exit)),
        Rule::continue_stmt => Ok(Some(Statement::Continue)),
        Rule::return_stmt => {
            let expr = match inner.into_inner().next() {
                Some(p) => {
                    let span = span_from_pair(&p);
                    Some(Spanned::new(parse_expression(p)?, span))
                }
                None => None,
            };
            Ok(Some(Statement::Return(expr)))
        }
        Rule::call_stmt => Ok(Some(parse_call_stmt(inner)?)),
        Rule::empty_stmt => Ok(Some(Statement::Empty)),
        _ => Ok(None),
    }
}

fn parse_assignment(pair: Pair<Rule>) -> Result<Statement> {
    let mut inner = pair.into_inner();
    let target_pair = inner.expect_next("assignment target")?;
    let target_span = span_from_pair(&target_pair);
    let target = Spanned::new(parse_variable(target_pair)?, target_span);

    let value_pair = inner.expect_next("assignment value")?;
    let value_span = span_from_pair(&value_pair);
    let value = Spanned::new(parse_expression(value_pair)?, value_span);

    Ok(Statement::Assignment(Assignment { target, value }))
}

fn parse_if(pair: Pair<Rule>) -> Result<Statement> {
    let mut inner = pair.into_inner();

    let cond_pair = inner.expect_next("if condition")?;
    let cond_span = span_from_pair(&cond_pair);
    let condition = Spanned::new(parse_expression(cond_pair)?, cond_span);

    let then_branch = parse_statement_list(inner.expect_next("if then branch")?)?;

    let mut elsif_branches = Vec::new();
    let mut else_branch = None;

    for item in inner {
        match item.as_rule() {
            Rule::elsif_branch => {
                let mut parts = item.into_inner();
                let cond = parts.expect_next("elsif condition")?;
                let cond_span = span_from_pair(&cond);
                elsif_branches.push(ElsifBranch {
                    condition: Spanned::new(parse_expression(cond)?, cond_span),
                    statements: parse_statement_list(parts.expect_next("elsif statements")?)?,
                });
            }
            Rule::else_branch => {
                let stmt_list = item
                    .into_inner()
                    .next()
                    .ok_or_else(|| anyhow!("Expected else statements"))?;
                else_branch = Some(parse_statement_list(stmt_list)?);
            }
            _ => {}
        }
    }

    Ok(Statement::If(IfStatement {
        condition,
        then_branch,
        elsif_branches,
        else_branch,
    }))
}

fn parse_case(pair: Pair<Rule>) -> Result<Statement> {
    let mut inner = pair.into_inner();

    let sel_pair = inner.expect_next("case selector")?;
    let sel_span = span_from_pair(&sel_pair);
    let selector = Spanned::new(parse_expression(sel_pair)?, sel_span);

    let mut branches = Vec::new();
    let mut else_branch = None;

    for item in inner {
        match item.as_rule() {
            Rule::case_branch => {
                let mut parts = item.into_inner();
                let values_pair = parts.expect_next("case values")?;
                let values = parse_case_values(values_pair)?;
                let statements = parse_statement_list(parts.expect_next("case statements")?)?;
                branches.push(CaseBranch { values, statements });
            }
            Rule::else_branch => {
                let stmt_list = item
                    .into_inner()
                    .next()
                    .ok_or_else(|| anyhow!("Expected else statements in case"))?;
                else_branch = Some(parse_statement_list(stmt_list)?);
            }
            _ => {}
        }
    }

    Ok(Statement::Case(CaseStatement {
        selector,
        branches,
        else_branch,
    }))
}

fn parse_case_values(pair: Pair<Rule>) -> Result<Vec<CaseValue>> {
    let mut values = Vec::new();
    for item in pair.into_inner() {
        if item.as_rule() == Rule::case_value {
            let mut parts = item.into_inner();
            let first = parts
                .expect_next("case value expression")?;
            let first_span = span_from_pair(&first);
            let first_expr = Spanned::new(parse_expression(first)?, first_span);

            if let Some(second) = parts.next() {
                let second_span = span_from_pair(&second);
                let second_expr = Spanned::new(parse_expression(second)?, second_span);
                values.push(CaseValue::Range(first_expr, second_expr));
            } else {
                values.push(CaseValue::Single(first_expr));
            }
        }
    }
    Ok(values)
}

fn parse_for(pair: Pair<Rule>) -> Result<Statement> {
    let mut inner = pair.into_inner();

    let variable = inner
        .expect_next("FOR loop variable")?
        .as_str()
        .to_string();

    let from_pair = inner.expect_next("FOR loop start expression")?;
    let from_span = span_from_pair(&from_pair);
    let from = Spanned::new(parse_expression(from_pair)?, from_span);

    let to_pair = inner.expect_next("FOR loop end expression")?;
    let to_span = span_from_pair(&to_pair);
    let to = Spanned::new(parse_expression(to_pair)?, to_span);

    let mut by = None;
    let mut body = Vec::new();

    for item in inner {
        match item.as_rule() {
            Rule::expression => {
                let span = span_from_pair(&item);
                by = Some(Spanned::new(parse_expression(item)?, span));
            }
            Rule::statement_list => {
                body = parse_statement_list(item)?;
            }
            _ => {}
        }
    }

    Ok(Statement::For(ForStatement {
        variable,
        from,
        to,
        by,
        body,
    }))
}

fn parse_while(pair: Pair<Rule>) -> Result<Statement> {
    let mut inner = pair.into_inner();

    let cond_pair = inner.expect_next("WHILE condition")?;
    let cond_span = span_from_pair(&cond_pair);
    let condition = Spanned::new(parse_expression(cond_pair)?, cond_span);

    let body_pair = inner.expect_next("WHILE body")?;
    let body = parse_statement_list(body_pair)?;

    Ok(Statement::While(WhileStatement { condition, body }))
}

fn parse_repeat(pair: Pair<Rule>) -> Result<Statement> {
    let mut inner = pair.into_inner();

    let body_pair = inner.expect_next("REPEAT body")?;
    let body = parse_statement_list(body_pair)?;

    let until_pair = inner.expect_next("UNTIL condition")?;
    let until_span = span_from_pair(&until_pair);
    let until = Spanned::new(parse_expression(until_pair)?, until_span);

    Ok(Statement::Repeat(RepeatStatement { body, until }))
}

fn parse_call_stmt(pair: Pair<Rule>) -> Result<Statement> {
    let mut inner = pair.into_inner();
    let name = inner
        .expect_next("function/block call name")?
        .as_str()
        .to_string();
    let arguments = inner
        .next()
        .map(parse_arguments)
        .transpose()?
        .unwrap_or_default();

    Ok(Statement::Call(CallStatement { name, arguments }))
}

fn parse_arguments(pair: Pair<Rule>) -> Result<Vec<CallArgument>> {
    let mut args = Vec::new();
    for item in pair.into_inner() {
        if item.as_rule() == Rule::argument {
            let mut parts: Vec<_> = item.into_inner().collect();

            if parts.len() == 2 {
                // Named argument
                let name = Some(parts[0].as_str().to_string());
                let value_span = span_from_pair(&parts[1]);
                let value = Spanned::new(parse_expression(parts.pop().unwrap())?, value_span);
                args.push(CallArgument { name, value });
            } else {
                // Positional argument
                let value_span = span_from_pair(&parts[0]);
                let value = Spanned::new(parse_expression(parts.pop().unwrap())?, value_span);
                args.push(CallArgument { name: None, value });
            }
        }
    }
    Ok(args)
}

fn parse_expression(pair: Pair<Rule>) -> Result<Expression> {
    // expression = { or_expr }
    // Extract the or_expr from expression
    let or_expr = pair
        .into_inner()
        .next()
        .ok_or_else(|| anyhow!("Expected expression content"))?;
    parse_or_expr(or_expr)
}

fn parse_or_expr(pair: Pair<Rule>) -> Result<Expression> {
    let mut inner = pair.into_inner();
    let first = inner
        .next()
        .ok_or_else(|| anyhow!("Expected OR expression operand"))?;
    let mut left = parse_xor_expr(first)?;

    while let Some(op_pair) = inner.next() {
        if op_pair.as_rule() == Rule::or_op {
            let right_pair = inner
                .next()
                .ok_or_else(|| anyhow!("Expected right operand after OR operator"))?;
            let right = parse_xor_expr(right_pair)?;
            let left_span = Span::default(); // Simplified
            let right_span = Span::default();
            left = Expression::Binary {
                left: Box::new(Spanned::new(left, left_span)),
                op: BinaryOp::Or,
                right: Box::new(Spanned::new(right, right_span)),
            };
        } else {
            let right = parse_xor_expr(op_pair)?;
            let left_span = Span::default();
            let right_span = Span::default();
            left = Expression::Binary {
                left: Box::new(Spanned::new(left, left_span)),
                op: BinaryOp::Or,
                right: Box::new(Spanned::new(right, right_span)),
            };
        }
    }

    Ok(left)
}

fn parse_xor_expr(pair: Pair<Rule>) -> Result<Expression> {
    let mut inner = pair.into_inner();
    let first = inner
        .next()
        .ok_or_else(|| anyhow!("Expected XOR expression operand"))?;
    let mut left = parse_and_expr(first)?;

    while let Some(op_pair) = inner.next() {
        if op_pair.as_rule() == Rule::xor_op {
            let right_pair = inner
                .next()
                .ok_or_else(|| anyhow!("Expected right operand after XOR operator"))?;
            let right = parse_and_expr(right_pair)?;
            let left_span = Span::default();
            let right_span = Span::default();
            left = Expression::Binary {
                left: Box::new(Spanned::new(left, left_span)),
                op: BinaryOp::Xor,
                right: Box::new(Spanned::new(right, right_span)),
            };
        }
    }

    Ok(left)
}

fn parse_and_expr(pair: Pair<Rule>) -> Result<Expression> {
    let mut inner = pair.into_inner();
    let first = inner
        .next()
        .ok_or_else(|| anyhow!("Expected AND expression operand"))?;
    let mut left = parse_comparison(first)?;

    while let Some(op_pair) = inner.next() {
        if op_pair.as_rule() == Rule::and_op {
            let right_pair = inner
                .next()
                .ok_or_else(|| anyhow!("Expected right operand after AND operator"))?;
            let right = parse_comparison(right_pair)?;
            let left_span = Span::default();
            let right_span = Span::default();
            left = Expression::Binary {
                left: Box::new(Spanned::new(left, left_span)),
                op: BinaryOp::And,
                right: Box::new(Spanned::new(right, right_span)),
            };
        }
    }

    Ok(left)
}

fn parse_comparison(pair: Pair<Rule>) -> Result<Expression> {
    let mut inner = pair.into_inner();
    let first = inner
        .next()
        .ok_or_else(|| anyhow!("Expected comparison operand"))?;
    let mut left = parse_add_expr(first)?;

    while let Some(op_pair) = inner.next() {
        if op_pair.as_rule() == Rule::comparison_op {
            let op = match op_pair.as_str() {
                "=" => BinaryOp::Eq,
                "<>" => BinaryOp::Ne,
                "<" => BinaryOp::Lt,
                "<=" => BinaryOp::Le,
                ">" => BinaryOp::Gt,
                ">=" => BinaryOp::Ge,
                _ => return Err(anyhow!("Unknown comparison operator")),
            };
            let right_pair = inner
                .next()
                .ok_or_else(|| anyhow!("Expected right operand after comparison operator"))?;
            let right = parse_add_expr(right_pair)?;
            let left_span = Span::default();
            let right_span = Span::default();
            left = Expression::Binary {
                left: Box::new(Spanned::new(left, left_span)),
                op,
                right: Box::new(Spanned::new(right, right_span)),
            };
        }
    }

    Ok(left)
}

fn parse_add_expr(pair: Pair<Rule>) -> Result<Expression> {
    let mut inner = pair.into_inner();
    let first = inner
        .next()
        .ok_or_else(|| anyhow!("Expected additive expression operand"))?;
    let mut left = parse_mul_expr(first)?;

    while let Some(op_pair) = inner.next() {
        if op_pair.as_rule() == Rule::add_op {
            let op = match op_pair.as_str() {
                "+" => BinaryOp::Add,
                "-" => BinaryOp::Sub,
                _ => return Err(anyhow!("Unknown add operator")),
            };
            let right_pair = inner
                .next()
                .ok_or_else(|| anyhow!("Expected right operand after +/- operator"))?;
            let right = parse_mul_expr(right_pair)?;
            let left_span = Span::default();
            let right_span = Span::default();
            left = Expression::Binary {
                left: Box::new(Spanned::new(left, left_span)),
                op,
                right: Box::new(Spanned::new(right, right_span)),
            };
        }
    }

    Ok(left)
}

fn parse_mul_expr(pair: Pair<Rule>) -> Result<Expression> {
    let mut inner = pair.into_inner();
    let first = inner
        .next()
        .ok_or_else(|| anyhow!("Expected multiplicative expression operand"))?;
    let mut left = parse_power_expr(first)?;

    while let Some(op_pair) = inner.next() {
        if op_pair.as_rule() == Rule::mul_op {
            let op = match op_pair.as_str().to_uppercase().as_str() {
                "*" => BinaryOp::Mul,
                "/" => BinaryOp::Div,
                "MOD" => BinaryOp::Mod,
                _ => return Err(anyhow!("Unknown mul operator")),
            };
            let right_pair = inner
                .next()
                .ok_or_else(|| anyhow!("Expected right operand after */MOD operator"))?;
            let right = parse_power_expr(right_pair)?;
            let left_span = Span::default();
            let right_span = Span::default();
            left = Expression::Binary {
                left: Box::new(Spanned::new(left, left_span)),
                op,
                right: Box::new(Spanned::new(right, right_span)),
            };
        }
    }

    Ok(left)
}

fn parse_power_expr(pair: Pair<Rule>) -> Result<Expression> {
    let mut inner = pair.into_inner();
    let first = inner
        .next()
        .ok_or_else(|| anyhow!("Expected power expression operand"))?;
    let mut left = parse_unary_expr(first)?;

    for right_pair in inner {
        let right = parse_unary_expr(right_pair)?;
        let left_span = Span::default();
        let right_span = Span::default();
        left = Expression::Binary {
            left: Box::new(Spanned::new(left, left_span)),
            op: BinaryOp::Pow,
            right: Box::new(Spanned::new(right, right_span)),
        };
    }

    Ok(left)
}

fn parse_unary_expr(pair: Pair<Rule>) -> Result<Expression> {
    // unary_expr = { unary_op? ~ primary_expr }
    let mut inner = pair.into_inner();
    let first = inner
        .next()
        .ok_or_else(|| anyhow!("Expected unary expression content"))?;

    if first.as_rule() == Rule::unary_op {
        let op = match first.as_str().to_uppercase().as_str() {
            "-" => UnaryOp::Neg,
            "NOT" => UnaryOp::Not,
            _ => return Err(anyhow!("Unknown unary operator")),
        };
        let primary = inner
            .next()
            .ok_or_else(|| anyhow!("Expected operand after unary operator"))?;
        let operand = parse_primary_expr_inner(primary)?;
        let span = Span::default();
        Ok(Expression::Unary {
            op,
            operand: Box::new(Spanned::new(operand, span)),
        })
    } else {
        // first is primary_expr
        parse_primary_expr_inner(first)
    }
}

fn parse_primary_expr_inner(pair: Pair<Rule>) -> Result<Expression> {
    // primary_expr = { "(" ~ expression ~ ")" | function_call | literal | variable }
    let inner = pair
        .into_inner()
        .next()
        .ok_or_else(|| anyhow!("Expected primary expression content"))?;
    match inner.as_rule() {
        Rule::expression => {
            let expr = parse_expression(inner)?;
            Ok(Expression::Paren(Box::new(Spanned::new(
                expr,
                Span::default(),
            ))))
        }
        Rule::function_call => parse_function_call(inner),
        Rule::literal => parse_literal(inner),
        Rule::variable => parse_variable(inner),
        _ => Err(anyhow!(
            "Unexpected primary expression: {:?}",
            inner.as_rule()
        )),
    }
}

fn parse_function_call(pair: Pair<Rule>) -> Result<Expression> {
    let mut inner = pair.into_inner();
    let name = inner
        .next()
        .ok_or_else(|| anyhow!("Expected function name"))?
        .as_str()
        .to_string();
    let arguments = inner
        .next()
        .map(parse_arguments)
        .transpose()?
        .unwrap_or_default();

    Ok(Expression::Call { name, arguments })
}

fn parse_literal(pair: Pair<Rule>) -> Result<Expression> {
    let inner = pair
        .into_inner()
        .next()
        .ok_or_else(|| anyhow!("Expected literal content"))?;
    match inner.as_rule() {
        Rule::bool_literal => {
            let val = inner.as_str().to_uppercase() == "TRUE";
            Ok(Expression::Literal(Literal::Bool(val)))
        }
        Rule::integer_literal => {
            let s = inner.as_str().replace('_', "");
            let val = if let Some(rest) = s.strip_prefix("16#") {
                i64::from_str_radix(rest, 16)
            } else if let Some(rest) = s.strip_prefix("8#") {
                i64::from_str_radix(rest, 8)
            } else if let Some(rest) = s.strip_prefix("2#") {
                i64::from_str_radix(rest, 2)
            } else {
                s.parse()
            }
            .map_err(|_| anyhow!("Invalid integer literal: {}", s))?;
            Ok(Expression::Literal(Literal::Integer(val)))
        }
        Rule::real_literal => {
            let val: f64 = inner
                .as_str()
                .parse()
                .map_err(|_| anyhow!("Invalid real literal"))?;
            Ok(Expression::Literal(Literal::Real(val)))
        }
        Rule::string_literal => {
            let s = inner.as_str();
            // Remove quotes
            let val = s[1..s.len() - 1].to_string();
            Ok(Expression::Literal(Literal::String(val)))
        }
        Rule::time_literal => {
            let ns = super::lexer::parse_time_literal(inner.as_str())
                .map_err(|e| anyhow!("Invalid time literal: {}", e))?;
            Ok(Expression::Literal(Literal::Time(ns)))
        }
        _ => Err(anyhow!("Unexpected literal type: {:?}", inner.as_rule())),
    }
}

fn parse_variable(pair: Pair<Rule>) -> Result<Expression> {
    let mut inner = pair.into_inner();
    let name = inner
        .next()
        .ok_or_else(|| anyhow!("Expected variable name"))?
        .as_str()
        .to_string();
    let mut expr = Expression::Variable(name);

    for item in inner {
        match item.as_rule() {
            Rule::array_index => {
                let index_pair = item
                    .into_inner()
                    .next()
                    .ok_or_else(|| anyhow!("Expected array index expression"))?;
                let index_span = span_from_pair(&index_pair);
                let index = parse_expression(index_pair)?;
                expr = Expression::ArrayAccess {
                    array: Box::new(Spanned::new(expr, Span::default())),
                    index: Box::new(Spanned::new(index, index_span)),
                };
            }
            Rule::field_access => {
                let field = item
                    .into_inner()
                    .next()
                    .ok_or_else(|| anyhow!("Expected field name"))?
                    .as_str()
                    .to_string();
                expr = Expression::FieldAccess {
                    object: Box::new(Spanned::new(expr, Span::default())),
                    field,
                };
            }
            _ => {}
        }
    }

    Ok(expr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_program() {
        let source = r#"
            PROGRAM Main
            VAR
                x : INT := 0;
            END_VAR
                x := x + 1;
            END_PROGRAM
        "#;

        let result = parse(source);
        assert!(result.is_ok(), "Parse failed: {:?}", result.err());

        let unit = result.unwrap();
        assert_eq!(unit.units.len(), 1);

        match &unit.units[0].node {
            ProgramUnit::Program(p) => {
                assert_eq!(p.name, "Main");
                assert_eq!(p.variables.len(), 1);
                assert_eq!(p.body.len(), 1);
            }
            _ => panic!("Expected Program"),
        }
    }

    #[test]
    fn test_parse_if_statement() {
        let source = r#"
            PROGRAM Test
            VAR
                flag : BOOL;
                counter : INT;
            END_VAR
                IF flag THEN
                    counter := counter + 1;
                ELSE
                    counter := 0;
                END_IF;
            END_PROGRAM
        "#;

        let result = parse(source);
        assert!(result.is_ok(), "Parse failed: {:?}", result.err());
    }

    #[test]
    fn test_parse_for_loop() {
        let source = r#"
            PROGRAM Test
            VAR
                i : INT;
                sum : INT := 0;
            END_VAR
                FOR i := 1 TO 10 DO
                    sum := sum + i;
                END_FOR;
            END_PROGRAM
        "#;

        let result = parse(source);
        assert!(result.is_ok(), "Parse failed: {:?}", result.err());
    }

    #[test]
    fn test_parse_function() {
        let source = r#"
            FUNCTION Add : INT
            VAR_INPUT
                a : INT;
                b : INT;
            END_VAR
                Add := a + b;
            END_FUNCTION
        "#;

        let result = parse(source);
        assert!(result.is_ok(), "Parse failed: {:?}", result.err());

        let unit = result.unwrap();
        match &unit.units[0].node {
            ProgramUnit::Function(f) => {
                assert_eq!(f.name, "Add");
                assert_eq!(f.return_type, DataType::Int);
            }
            _ => panic!("Expected Function"),
        }
    }

    #[test]
    fn test_parse_expressions() {
        let source = r#"
            PROGRAM Expr
            VAR
                a, b, c : INT;
                result : BOOL;
            END_VAR
                a := 1 + 2 * 3;
                result := (a > b) AND (b < c);
            END_PROGRAM
        "#;

        let result = parse(source);
        assert!(result.is_ok(), "Parse failed: {:?}", result.err());
    }

    #[test]
    fn test_parse_comma_separated_vars() {
        let source = r#"
            PROGRAM VarTest
            VAR
                a, b, c : INT := 10;
                x, y : REAL;
            END_VAR
                a := b + c;
            END_PROGRAM
        "#;

        let result = parse(source);
        assert!(result.is_ok(), "Parse failed: {:?}", result.err());

        let unit = result.unwrap();
        match &unit.units[0].node {
            ProgramUnit::Program(p) => {
                assert_eq!(p.name, "VarTest");
                assert_eq!(p.variables.len(), 1); // One VAR block

                let var_block = &p.variables[0].node;
                // Should have 5 declarations: a, b, c, x, y
                assert_eq!(
                    var_block.declarations.len(),
                    5,
                    "Expected 5 declarations (a, b, c, x, y), got {}",
                    var_block.declarations.len()
                );

                // Check that a, b, c all have the same type and initial value
                let names: Vec<&str> = var_block
                    .declarations
                    .iter()
                    .map(|d| d.node.name.as_str())
                    .collect();
                assert!(names.contains(&"a"));
                assert!(names.contains(&"b"));
                assert!(names.contains(&"c"));
                assert!(names.contains(&"x"));
                assert!(names.contains(&"y"));

                // Verify types
                for decl in &var_block.declarations {
                    if ["a", "b", "c"].contains(&decl.node.name.as_str()) {
                        assert_eq!(decl.node.data_type, DataType::Int);
                        assert!(decl.node.initial_value.is_some());
                    } else {
                        assert_eq!(decl.node.data_type, DataType::Real);
                        assert!(decl.node.initial_value.is_none());
                    }
                }
            }
            _ => panic!("Expected Program"),
        }
    }
}
