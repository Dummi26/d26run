use std::{collections::HashMap, process::Command};

#[derive(Clone, Debug, PartialEq)]
pub enum VarValue {
    Bool(bool),
    Int(i128),
    Float(f64),
    String(String),
    List(Vec<Self>),
    Nothing,
}
pub fn parse_expression(
    expr: &str,
    vars: &mut HashMap<String, VarValue>,
) -> Result<VarValue, String> {
    // println!("Parsing '{expr}'");
    Ok('r: {
        // what comes first here will be evaluated last (think: everything below a certain
        // operation must happen before the operation itself can be performed.

        // : and :: (for strings)
        if let Some((name, expression)) = expr.split_once("=") {
            match expression.chars().next() {
                Some('=') => (),
                ch => {
                    let value = if let Some(':') = ch {
                        VarValue::String(expression[1..].to_string())
                    } else {
                        parse_expression(expression, vars)?
                    };
                    vars.insert(name.trim().to_string(), value.clone());
                    break 'r VarValue::Nothing;
                }
            }
        }
        // ! (invert bools)
        {
            let trim = expr.trim();
            if let Some('!') = trim.chars().next() {
                break 'r match parse_expression(&trim[1..], vars)? {
                    VarValue::Bool(v) => VarValue::Bool(!v),
                    _ => VarValue::Nothing,
                };
            }
        }
        // : (functions)
        if expr.contains(':') {
            let mut split = expr.split(':');
            if let Some(func) = split.next() {
                let mut parts = Vec::new();
                for part in split {
                    parts.push(parse_expression(part, vars)?);
                }
                break 'r match (func.trim(), parts.as_slice()) {
                    ("print", [v]) => {
                        eprintln!("{}", parse_expression_val_to_string(v));
                        VarValue::Nothing
                    }
                    ("debugprint", [v]) => {
                        eprintln!("{v:?}");
                        VarValue::Nothing
                    }
                    ("list", _) => VarValue::List(parts),
                    ("if", [VarValue::Bool(c), _, _]) => {
                        if *c {
                            std::mem::replace(&mut parts[1], VarValue::Nothing)
                        } else {
                            std::mem::replace(&mut parts[2], VarValue::Nothing)
                        }
                    }
                    ("for", [VarValue::List(v), ..]) => {
                        let varname = "for";
                        let pvar = match vars.get(varname) {
                            Some(v) => Some(v.clone()),
                            None => {
                                vars.insert(varname.to_string(), VarValue::Nothing);
                                None
                            }
                        };
                        let mut break_val = None;
                        // for action in &parts[1..] { println!("{action:?}") }
                        for v in v {
                            // println!("{v:?}");
                            *vars.get_mut(varname).unwrap() = v.clone();
                            for action in &parts[1..] {
                                match action {
                                    VarValue::String(action) => {
                                        match parse_expression(action, vars) {
                                            Ok(v) => match v {
                                                VarValue::Nothing => (),
                                                val => {
                                                    break_val = Some(val);
                                                    break;
                                                }
                                            },
                                            Err(e) => {
                                                println!("[!] config: couldn't parse action in for loop: {e}");
                                                break;
                                            }
                                        }
                                    }
                                    _ => {
                                        println!(
                                            "[!] config: action in for loop was not a string!"
                                        );
                                        break;
                                    }
                                }
                            }
                        }
                        if let Some(pvar) = pvar {
                            *vars.get_mut(varname).unwrap() = pvar;
                        } else {
                            vars.remove(varname);
                        }
                        if let Some(bval) = break_val {
                            bval
                        } else {
                            VarValue::Nothing
                        }
                    }
                    ("to_string", [v]) => VarValue::String(parse_expression_val_to_string(v)),
                    ("t_bool", [v]) => VarValue::Bool(match v {
                        VarValue::Bool(_) => true,
                        _ => false,
                    }),
                    // TODO ^
                    (
                        "filter",
                        [VarValue::List(v), VarValue::String(varname), VarValue::String(filter)],
                    ) => VarValue::List({
                        let prev = vars.remove(varname);
                        let mut filtered = Vec::new();
                        for v in v {
                            vars.insert(varname.clone(), v.clone());
                            match parse_expression(filter, vars) {
                                Ok(VarValue::Bool(true)) => filtered.push(v.clone()),
                                _ => (),
                            }
                        }
                        if let Some(prev) = prev {
                            vars.insert(varname.clone(), prev);
                        } else {
                            vars.remove(varname);
                        }
                        filtered
                    }),
                    ("empty", [VarValue::String(v)]) => VarValue::Bool(v.is_empty()),
                    ("empty", [VarValue::List(v)]) => VarValue::Bool(v.is_empty()),
                    ("length", [VarValue::String(v)]) => VarValue::Int(v.len() as _),
                    ("length", [VarValue::List(v)]) => VarValue::Int(v.len() as _),
                    ("cmd-output", [VarValue::String(cmd), ..]) => {
                        let mut args = Vec::new();
                        for part in parts.iter().skip(1) {
                            args.push(parse_expression_val_to_string(part));
                        }
                        println!("Running command in script: {cmd}");
                        match Command::new(cmd).args(args).output() {
                            Ok(out) => VarValue::String(
                                String::from_utf8_lossy(out.stdout.as_slice()).to_string(),
                            ),
                            Err(e) => {
                                println!(" [CMD/E] {e:?}");
                                VarValue::Nothing
                            }
                        }
                    }
                    (func, args) => {
                        println!(
                            "[!] config: not a function: {func} with {} arguments",
                            args.len()
                        );
                        VarValue::Nothing
                    }
                };
            }
        }
        // && ||
        if let Some((l, r, op)) = parse_expression_split_at_operator(expr, &["&&", "||"]) {
            let l = parse_expression(l, vars)?;
            let r = parse_expression(r, vars)?;
            break 'r match (l, r) {
                (VarValue::Bool(l), VarValue::Bool(r)) => VarValue::Bool(match op {
                    0 => l && r,
                    1 => l || r,
                    _ => unreachable!(),
                }),
                _ => VarValue::Nothing,
            };
        }
        // ==
        if let Some((l, r)) = expr.split_once("==") {
            break 'r VarValue::Bool(parse_expression(l, vars)? == parse_expression(r, vars)?);
        }
        // + -
        if let Some((l, r, op)) = parse_expression_split_at_operator(expr, &["+", "-"]) {
            let l = parse_expression(l, vars)?;
            let r = parse_expression(r, vars)?;
            let floats = match (&l, &r) {
                (VarValue::Int(l), VarValue::Int(r)) => {
                    break 'r VarValue::Int(match op {
                        0 => *l + *r,
                        1 => *l - *r,
                        _ => unreachable!(),
                    })
                }
                (VarValue::Float(l), VarValue::Float(r)) => Some((*l, *r)),
                (VarValue::Int(l), VarValue::Float(r)) => Some((*l as f64, *r)),
                (VarValue::Float(l), VarValue::Int(r)) => Some((*l, *r as f64)),
                _ => None,
            };
            if let Some((l, r)) = floats {
                break 'r VarValue::Float(match op {
                    0 => l + r,
                    1 => l - r,
                    _ => unreachable!(),
                });
            };
            if op == 0 {
                match (l, r) {
                    (VarValue::String(a), VarValue::String(b)) => {
                        break 'r VarValue::String(format!("{a}{b}"))
                    }
                    (VarValue::List(mut a), VarValue::List(b)) => {
                        break 'r VarValue::List({
                            a.extend(b.into_iter());
                            a
                        })
                    }
                    _ => (),
                }
            } else {
                break 'r VarValue::Nothing;
            }
        }
        // * /
        if let Some((l, r, op)) = parse_expression_split_at_operator(expr, &["*", "/"]) {
            let l = parse_expression(l, vars)?;
            let r = parse_expression(r, vars)?;
            let (l, r) = match (l, r) {
                (VarValue::Int(l), VarValue::Int(r)) => {
                    break 'r VarValue::Int(match op {
                        0 => l * r,
                        1 => l / r,
                        _ => unreachable!(),
                    })
                }
                (VarValue::Float(l), VarValue::Float(r)) => (l, r),
                (VarValue::Int(l), VarValue::Float(r)) => (l as f64, r),
                (VarValue::Float(l), VarValue::Int(r)) => (l, r as f64),
                _ => break 'r VarValue::Nothing,
            };
            break 'r VarValue::Float(match op {
                0 => l * r,
                1 => {
                    if r == 0.0 {
                        break 'r VarValue::Nothing;
                    }
                    l / r
                }
                _ => unreachable!(),
            });
        }
        // int literal
        if let Ok(v) = expr.trim().parse() {
            break 'r VarValue::Int(v);
        }
        // float literal
        if let Ok(v) = expr.trim().parse() {
            break 'r VarValue::Float(v);
        }
        // bool literal
        match expr.trim().to_lowercase().as_str() {
            "true" => break 'r VarValue::Bool(true),
            "false" => break 'r VarValue::Bool(false),
            _ => (),
        }
        // variable
        if let Some(val) = vars.get(expr.trim()) {
            break 'r val.clone();
        }
        VarValue::Nothing
    })
}

pub fn parse_expression_val_to_string(val: &VarValue) -> String {
    match val {
        VarValue::Bool(v) => format!("{v}"),
        VarValue::Int(v) => format!("{v}"),
        VarValue::Float(v) => format!("{v}"),
        VarValue::String(v) => v.to_string(),
        VarValue::List(v) => {
            let mut buf = String::new();
            for v in v {
                buf.push_str(&parse_expression_val_to_string(v))
            }
            buf
        }
        VarValue::Nothing => String::new(),
    }
}

/// Returns the expressions left/right of the operator and the index of the operator in the slice.
fn parse_expression_split_at_operator<'a>(
    expr: &'a str,
    operators: &[&str],
) -> Option<(&'a str, &'a str, usize)> {
    let mut operator_id = 0;
    let mut operator_index = expr.len(); // guaranteed to be greater than any pattern's starting index
    let mut expressions = None;
    for (op_id, operator) in operators.iter().enumerate() {
        if let Some(i) = expr.find(operator) {
            if i < operator_index {
                operator_id = op_id;
                operator_index = i;
                expressions = Some((&expr[0..i], &expr[i + operator.len()..]));
            }
        }
    }
    if let Some((l, r)) = expressions {
        Some((l, r, operator_id))
    } else {
        None
    }
}
