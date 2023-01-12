use std::fs::{create_dir_all as mkdir, remove_dir_all as rmdir, read_to_string};
use std::process::{Command, Stdio};
use std::path::Path;
use std::collections::HashMap;
use palaver::thread::gettid;

/*

Make sure to add 'permit nopass keepenv [your root user]'
and optionally 'permit nopass [your user] cmd d26run' to /etc/doas.conf.
this will allow root to use doas without providing the root password and will make doas retain the env vars.
the second part lets you use d26run without having to type your password every time.
this might be a small security issue if you run any malicious apps as your normal user (instead of through this program))

Running 'xhost +' as your normal user will allow any user to access your x server, which is required for GUI apps on X.
However, this prevents userdel from working properly, which might cause issues.
A workaround for this is just to not use C (or count in the config), so that the username will depend on the process id. If the same PID appers twice (previous process has stopped, but user with that ID still exists), use C[num] to overwrite the ID.
TODO: Automatically disconnect the user from X somehow?

TODO:

- Fix immutable_home_at:
  - cannot copy ~/.cache/doc
    - maybe write own recursive copy function, that just ignores errors like that one?
  - actually test it

- Test noconfcmd

- Implement new stuff for config
  - if
  - better run
  - better init (switch to on [event] and do [command/arg]?)
  - add a way to do \n in do [...] (idea: do without anything starts a multiline section that continues until a line doesn't start with a whitespace)
  - maybe add functions for more advances stuff?

*/

fn main() {
    let mut args = std::env::args().into_iter();
    let mut config = None;
    let mut count: u64 = gettid();
    let mut name = None;
    let mut home_dir = None;
    let mut userdel = true;
    let mut userhomedel = false;
    let mut noconfcmd = false;
    let mut immutable_home_at = None;

    let mut config_script_variables = HashMap::new();
    config_script_variables.insert("args".to_string(), VarValue::List(vec![]));

    loop {
        let arg = args.next();
        if let Some(arg) = arg {
            if arg == "--" { break; }
            match arg.chars().next() {
                Some('c') => { config = Some(arg[1..].to_string()); },
                Some('C') => { count = arg[1..].parse().expect("Syntax is C[count], where [count] is an integer!"); },
                Some('n') => { name = Some(arg[1..].to_string()); },
                Some('h') => { home_dir = Some(arg[1..].to_string()); },
                Some('H') => { immutable_home_at = Some(arg[1..].to_string()); },
                Some('-') => match match arg[1..].split_once('=') { Some(v) => (v.0, Some(v.1)), None => (&arg[1..], None) } {
                    ("nouserdel", _) => userdel = false,
                    ("userhomedel", _) => userhomedel = true,
                    ("noconfcmd", _) => noconfcmd = true,
                    ("cfgarg", Some(arg)) => if let Some(VarValue::List(v)) = config_script_variables.get_mut("args") {
                        v.push(VarValue::String(arg.to_string()))
                    },
                    _ => println!("Ignoring unknown argument '{arg}'."),
                },
                _ => (),
            }
        } else if config.is_none() {
            println!("Syntax is 'd26run [args] -- [command + args]'.");
            println!("The '--' is mandatory unless a config is specified using 'c[path]'.");
            return;
        } else {
            break;
        }
    }

    println!("[c] Config: {}", match &config { None => "-".to_string(), Some(c) => format!("'{c}'") });

    let mut run = vec![];
    let mut init = vec![];
    if let Some(config) = &config {
        let mut depth: Vec<(bool, ())> = vec![];
        let mut last_run = true;
        match read_to_string(config) {
            Ok(config) => {
                for (line_num, line) in config.lines().enumerate() {
                    match (line.chars().next(), match line.split_once(' ') { Some(v) => (v.0, Some(v.1)), None => (line, None) }) {
                        (Some('#'), _) | (None, _) => (), // comment, do nothing
                        (_, ("end", _)) => {
                            if depth.pop().is_none() {
                                println!("[!] Line {line_num} was an 'end', but there is nothing it could be referring to.");
                            } else {
                                last_run = match depth.last() {
                                    None => true,
                                    Some(v) => v.0,
                                };
                            }
                        },
                        (Some(' '), _) => {
                            if last_run {
                                if let Err(e) = parse_expression(line, &mut config_script_variables) {
                                    println!("[!] config: couldn't parse expression: {e}");
                                }
                            }
                        }
                        (_, ("if", Some(condition))) => {
                            let is_true = if last_run {
                                match parse_expression(condition, &mut config_script_variables) {
                                    Ok(v) => match v {
                                        VarValue::Bool(v) => v,
                                        _ => {
                                            println!("if statement's condition was not a bool, falling back to default value (false). Value: {v:?}");
                                            false
                                        },
                                    },
                                    Err(e) => {
                                        println!("Couldn't parse if statement's condition, falling back to default value (false). Error: {e}");
                                        false
                                    },
                                }
                            } else { false };
                            depth.push((is_true, ()));
                            last_run = is_true;
                        },

                        (_, (_, None)) => (), // NOTE: Do something with this?
                        (_, (what, Some(args_unparsed))) => if last_run { // do something
                            let mut args = String::new();
                            {
                                let mut buf = String::new();
                                for ch in args_unparsed.chars() {
                                    buf.push(ch);
                                    if buf.starts_with("[d26var:") {
                                        if buf.ends_with("]") {
                                            let varname = &buf[8..buf.len()-1];
                                            if let Some(val) = config_script_variables.get(varname) {
                                                args.push_str(&parse_expression_val_to_string(val));
                                                buf.clear();
                                            } else {
                                            }
                                        }
                                    } else if ! "[d26var:".starts_with(&buf) {
                                        args.push_str(&buf);
                                        buf.clear();
                                    }
                                }
                            }
                            match what {
                                // run [something] adds [something] to the args for doas. The first one is the program to be executed, anything following that are args for that program: 'doas -u [user] -- [run...]'
                                "run" => if ! noconfcmd { run.push(args) },
                                // init is a command that will be executed WITH THIS PROGRAMS PERMISSIONS, NOT AS THE 'd26r...' USER! BE CAREFUL WITH THIS!
                                "init!" => init.push((true, vec![args])),
                                "init_" => init.push((false, vec![args])), // non-fatal
                                "init+" => init.last_mut().expect("used init+ before init! or init_!").1.push(args), // adds one argument to the last-defined init program.
                                "count" => count = match args.parse() { Ok(v) => v, Err(e) => panic!("Syntax is count [count], where [count] is an integer! Found '{args}'. {e:?}") },
                                "name" => if name.is_none() { name = Some(args) },
                                "setname" => name = Some(args),
                                "home" => if home_dir.is_none() { home_dir = Some(args) },
                                "sethome" => home_dir = Some(args),
                                "immuthome" => if immutable_home_at.is_none() { immutable_home_at = Some(args) },
                                "setimmuthome" => immutable_home_at = Some(args),
                                _ => println!("[CONFIG] '{what}' is not a valid action!"),
                            }
                        },
                    }
                }
            },
            Err(e) => panic!("Could not read config: {e}"),
        }
    }

    match &home_dir { Some(v) if v.is_empty() => home_dir = None, _ => () }
    match &immutable_home_at.as_ref() { Some(v) if v.is_empty() => immutable_home_at = None, _ => () }
    let name = match name { Some(n) => n, None => String::new() };

    let username = format!("d26r{count}_{name}");
    let home_dir = match home_dir {
        Some(v) => v
            .replace("[d26%count]", &count.to_string())
            .replace("[d26%username]", &username)
            .replace("[d26%name]", &name),
        None => format!("/tmp/dummi26/run/{username}/home")
    };

    println!("[C] Count: {count}");
    println!("[n] Name: {name}");
    println!("Username: {username}");
    println!("Home dir: {home_dir}");
    println!();

    println!("Removing previous user:");
    Command::new("userdel").arg(username.as_str()).stdout(Stdio::inherit()).stderr(Stdio::inherit()).output().unwrap();

    if userhomedel || immutable_home_at.is_some() { println!("Removing home: {:?}", rmdir(std::path::PathBuf::from(home_dir.as_str()))); }

    if let Some(immuthome) = immutable_home_at {
        println!("Copying existing home from '{immuthome} to '{home_dir}'... (setup immutable home)");
        for error in copy_dir_recursive_ignore_errors(&immuthome, &home_dir) {
            println!(" E: {error:?}");
        }
        println!("Done. (immutable home created)");
    } else {
        mkdir(home_dir.as_str()).unwrap();
    }

    println!("Adding new user:");
    Command::new("useradd").args([
        "--home-dir", home_dir.as_str(),
        "--no-user-group",
        "--create-home",
        username.as_str(),
    ]).stdout(Stdio::inherit()).stderr(Stdio::inherit()).status().unwrap();
    println!("chown home dir: {:?}", Command::new("chown").args(["-R", username.as_str(), home_dir.as_str()]).status().unwrap());

    println!("Running init commands from config, if any...");
    for (fatal, cmd) in init {
        let mut cmds = cmd.into_iter();
        let cmd = cmds.next().unwrap();
        let count_s = count.to_string();
        let mut args = vec![];
        for arg in cmds {
            args.push(arg
            .replace("[d26%count]", &count_s)
            .replace("[d26%username]", &username)
            .replace("[d26%name]", &name)
            .replace("[d26%home_dir]", &home_dir)
            );
        }
        let out = Command::new(&cmd).args(args).status();
        println!("Command '{cmd}': {:?}", out);
        if fatal && ! out.unwrap().success() { panic!("Cancelling because a fatal init command was unsuccessful.") };
    }

    println!("Setup complete, running command now...\n\n");
    println!("\n\n[EXIT STATUS: {:?}]", Command::new("doas").args(["-u".to_string(), username.clone(), "--".to_string()].into_iter().chain(run.into_iter()).chain(args)).status());

    if userdel { println!("Removing user: {:?}", Command::new("userdel").arg(username.as_str()).stdout(Stdio::inherit()).stderr(Stdio::inherit()).status().unwrap()); }
    if userhomedel { println!("Removing user home: {:?}", rmdir(home_dir)); }
}



fn copy_dir_recursive_ignore_errors<P1, P2>(dir: P1, target: P2)-> Vec<(std::ffi::OsString, std::io::Error)> where P1: AsRef<Path>, P2: AsRef<Path> {
    Command::new("cp").arg("-r").arg(dir.as_ref().as_os_str()).arg(target.as_ref().as_os_str()).status().unwrap();
    vec![] // TODO!
}



#[derive(Clone, Debug, PartialEq)]
enum VarValue {
    Bool(bool),
    Int(i128),
    Float(f64),
    String(String),
    List(Vec<Self>),
    Nothing,
}
fn parse_expression(expr: &str, vars: &mut HashMap<String, VarValue>) -> Result<VarValue, String> {
    // println!("Parsing '{expr}'");
    Ok('r: { // what comes first here will be evaluated last (think: everything below a certain
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
                },
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
                    },
                    ("debugprint", [v]) => {
                        eprintln!("{v:?}");
                        VarValue::Nothing
                    },
                    ("list", _) => {
                        VarValue::List(parts)
                    }
                    ("if", [VarValue::Bool(c), _, _]) => {
                        if *c {
                            std::mem::replace(&mut parts[1], VarValue::Nothing)
                        } else {
                            std::mem::replace(&mut parts[2], VarValue::Nothing)
                        }
                    },
                    ("for", [VarValue::List(v), ..]) => {
                        let varname = "for";
                        let pvar = match vars.get(varname) { Some(v) => Some(v.clone()), None => { vars.insert(varname.to_string(), VarValue::Nothing); None } };
                        let mut break_val = None;
                        // for action in &parts[1..] { println!("{action:?}") }
                        for v in v {
                            // println!("{v:?}");
                            *vars.get_mut(varname).unwrap() = v.clone();
                            for action in &parts[1..] {
                                match action {
                                    VarValue::String(action) => match parse_expression(action, vars) {
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
                                        },
                                    }
                                    _ => {
                                        println!("[!] config: action in for loop was not a string!");
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
                    },
                    ("to_string", [v]) => VarValue::String(parse_expression_val_to_string(v)),
                    ("t_bool", [v]) => VarValue::Bool(match v {
                        VarValue::Bool(_) => true,
                        _ => false,
                    }),
                    // TODO ^
                    ("filter", [VarValue::List(v), VarValue::String(varname), VarValue::String(filter)]) => VarValue::List({
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
                    _ => {
                        println!("[!] config: not a function: {func} with {} arguments", parts.len());
                        VarValue::Nothing
                    }
                }
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
            }
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
                (VarValue::Int(l), VarValue::Int(r)) => break 'r VarValue::Int(match op {
                    0 => *l + *r,
                    1 => *l - *r,
                    _ => unreachable!(),
                }),
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
                    (VarValue::String(a), VarValue::String(b)) => break 'r VarValue::String(format!("{a}{b}")),
                    (VarValue::List(mut a), VarValue::List(b)) => break 'r VarValue::List({ a.extend(b.into_iter()); a }),
                    _ => (),
                }
            } else { break 'r VarValue::Nothing }
        }
        // * /
        if let Some((l, r, op)) = parse_expression_split_at_operator(expr, &["*", "/"]) {
            let l = parse_expression(l, vars)?;
            let r = parse_expression(r, vars)?;
            let (l, r) = match (l, r) {
                (VarValue::Int(l), VarValue::Int(r)) => break 'r VarValue::Int(match op {
                    0 => l * r,
                    1 => l / r,
                    _ => unreachable!(),
                }),
                (VarValue::Float(l), VarValue::Float(r)) => (l, r),
                (VarValue::Int(l), VarValue::Float(r)) => (l as f64, r),
                (VarValue::Float(l), VarValue::Int(r)) => (l, r as f64),
                _ => break 'r VarValue::Nothing,
            };
            break 'r VarValue::Float(match op {
                0 => l * r,
                1 => {
                    if r == 0.0 { break 'r VarValue::Nothing }
                    l / r
                },
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

fn parse_expression_val_to_string(val: &VarValue) -> String {
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
        },
        VarValue::Nothing => String::new(),
    }
}

/// Returns the expressions left/right of the operator and the index of the operator in the slice.
fn parse_expression_split_at_operator<'a>(expr: &'a str, operators: &[&str]) -> Option<(&'a str, &'a str, usize)> {
    let mut operator_id = 0;
    let mut operator_index = expr.len(); // guaranteed to be greater than any pattern's starting index
    let mut expressions = None;
    for (op_id, operator) in operators.iter().enumerate() {
        if let Some(i) = expr.find(operator) {
            if i < operator_index {
                operator_id = op_id;
                operator_index = i;
                expressions = Some((&expr[0..i], &expr[i+operator.len()..]));
            }
        }
    }
    if let Some((l, r)) = expressions {
        Some((l, r, operator_id))
    } else { None }
}
