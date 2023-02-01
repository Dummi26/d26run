use std::collections::HashMap;

use std::fs::read_to_string;

use palaver::thread::gettid;

use crate::script::{parse_expression, parse_expression_val_to_string, VarValue};

pub struct Conf {
    pub run: Vec<String>,
    pub init: Vec<(bool, Vec<String>)>,
    pub count: u64,
    pub test: bool,
    pub name: Option<String>,
    pub passwd: Option<String>,
    pub group: Option<String>,  // -g
    pub groups: Option<String>, // -G
    pub home_dir: Option<String>,
    pub userdel: bool,
    pub userhomedel: bool,
    pub noconfcmd: bool,
    pub immutable_home_at: Option<String>,

    pub config_script_variables: HashMap<String, VarValue>,
}
impl Conf {
    pub fn new() -> Self {
        Self {
            run: Vec::new(),
            init: Vec::new(),
            count: gettid(),
            test: false,
            name: None,
            passwd: None,
            group: None,
            groups: None,
            home_dir: None,
            userdel: true,
            userhomedel: false,
            noconfcmd: false,
            immutable_home_at: None,

            config_script_variables: HashMap::new(),
        }
    }
    pub fn get_username(&self) -> String {
        format!(
            "d26r{}_{}",
            self.count,
            match &self.name {
                Some(v) => v,
                None => "",
            }
        )
    }
    pub fn apply_to_all_strings(&mut self) {
        let mut run = std::mem::replace(&mut self.run, Vec::with_capacity(0));
        for v in run.iter_mut() {
            *v = self.apply_to_string(&v);
        }
        self.run = run;

        let mut init = std::mem::replace(&mut self.init, Vec::with_capacity(0));
        for v in init.iter_mut() {
            for v in v.1.iter_mut() {
                *v = self.apply_to_string(&v);
            }
        }
        self.init = init;

        if let Some(v) = self.name.take() {
            self.name = Some(self.apply_to_string(&v));
        }

        if let Some(v) = self.group.take() {
            self.group = Some(self.apply_to_string(&v));
        }

        if let Some(v) = self.groups.take() {
            self.groups = Some(self.apply_to_string(&v));
        }

        if let Some(v) = self.home_dir.take() {
            self.home_dir = Some(self.apply_to_string(&v));
        }

        if let Some(v) = self.immutable_home_at.take() {
            self.immutable_home_at = Some(self.apply_to_string(&v));
        }
    }
    pub fn apply_to_string(&self, string: &str) -> String {
        let mut string = string
            .replace("[d26%count]", &self.count.to_string())
            .replace("[d26%username]", &self.get_username());
        if string.contains("[d26%group]") {
            if let Some(v) = &self.group {
                string = string.replace("[d26%group]", &self.apply_to_string(v));
            }
        }
        if string.contains("[d26%groups]") {
            if let Some(v) = &self.groups {
                string = string.replace("[d26%groups]", &self.apply_to_string(v));
            }
        }
        if string.contains("[d26%home_dir]") {
            if let Some(v) = &self.home_dir {
                string = string.replace("[d26%home_dir]", &self.apply_to_string(v));
            }
        }
        if string.contains("[d26%immutable_home_at]") {
            if let Some(v) = &self.immutable_home_at {
                string = string.replace("[d26%immutable_home_at]", &self.apply_to_string(v));
            }
        }
        string
    }
    pub fn load(&mut self, conf_file: &str) {
        let mut depth: Vec<(bool, ())> = vec![];
        let mut last_run = true;
        match read_to_string(conf_file) {
            Ok(config) => {
                for (line_num, line) in config.lines().enumerate() {
                    match (
                        line.chars().next(),
                        match line.split_once(' ') {
                            Some(v) => (v.0, Some(v.1)),
                            None => (line, None),
                        },
                    ) {
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
                        }
                        (Some(' '), _) => {
                            if last_run {
                                if let Err(e) =
                                    parse_expression(line, &mut self.config_script_variables)
                                {
                                    println!("[!] config: couldn't parse expression: {e}");
                                }
                            }
                        }
                        (_, ("if", Some(condition))) => {
                            let is_true = if last_run {
                                match parse_expression(condition, &mut self.config_script_variables)
                                {
                                    Ok(v) => match v {
                                        VarValue::Bool(v) => v,
                                        _ => {
                                            println!("if statement's condition was not a bool, falling back to default value (false). Value: {v:?}");
                                            false
                                        }
                                    },
                                    Err(e) => {
                                        println!("Couldn't parse if statement's condition, falling back to default value (false). Error: {e}");
                                        false
                                    }
                                }
                            } else {
                                false
                            };
                            depth.push((is_true, ()));
                            last_run = is_true;
                        }

                        (_, (_, None)) => (), // NOTE: Do something with this?
                        (_, (what, Some(args_unparsed))) => {
                            if last_run {
                                // do something
                                let mut args = String::new();
                                {
                                    let mut buf = String::new();
                                    for ch in args_unparsed.chars() {
                                        buf.push(ch);
                                        if buf.starts_with("[d26var:") {
                                            if buf.ends_with("]") {
                                                let varname = &buf[8..buf.len() - 1];
                                                if let Some(val) =
                                                    self.config_script_variables.get(varname)
                                                {
                                                    args.push_str(&parse_expression_val_to_string(
                                                        val,
                                                    ));
                                                    buf.clear();
                                                } else {
                                                }
                                            }
                                        } else if !"[d26var:".starts_with(&buf) {
                                            args.push_str(&buf);
                                            buf.clear();
                                        }
                                    }
                                }
                                let args = args.trim_end_matches('\n').to_string();
                                match what {
                                // load another config
                                "config" => self.load(&args),
                                // run [something] adds [something] to the args for doas. The first one is the program to be executed, anything following that are args for that program: 'doas -u [user] -- [run...]'
                                "run" => if ! self.noconfcmd { self.run.push(args) },
                                // init is a command that will be executed WITH THIS PROGRAMS PERMISSIONS, NOT AS THE 'd26r...' USER! BE CAREFUL WITH THIS!
                                "init!" => self.init.push((true, vec![args])),
                                "init_" => self.init.push((false, vec![args])), // non-fatal
                                "init+" => self.init.last_mut().expect("used init+ before init! or init_!").1.push(args), // adds one argument to the last-defined init program.
                                "count" => self.count = match args.parse() { Ok(v) => v, Err(e) => panic!("Syntax is count [count], where [count] is an integer! Found '{args}'. {e:?}") },
                                "name" => if self.name.is_none() { self.name = Some(args) },
                                "setname" => self.name = Some(args),
                                "passwd" => if self.passwd.is_none() { self.passwd = Some(args) },
                                "setpasswd" => self.passwd = Some(args),
                                "group" => if self.group.is_none() { self.group = Some(args) },
                                "setgroup" => self.group = Some(args),
                                "groups" => if self.groups.is_none() { self.groups = Some(args) },
                                "setgroups" => self.groups = Some(args),
                                "addgroups" => match self.groups.take() {
                                    Some(mut groups) => {
                                        self.groups = if groups.len() == 0 {
                                            Some(args)
                                        } else {
                                            groups.push(',');
                                            groups.push_str(&args);
                                            Some(groups)
                                        }
                                    },
                                    None => self.groups = Some(args),
                                }
                                "home" => if self.home_dir.is_none() { self.home_dir = Some(args) },
                                "sethome" => self.home_dir = Some(args),
                                "immuthome" => if self.immutable_home_at.is_none() { self.immutable_home_at = Some(args) },
                                "setimmuthome" => self.immutable_home_at = Some(args),
                                _ => println!("[CONFIG] '{what}' is not a valid action!"),
                            }
                            }
                        }
                    }
                }
            }
            Err(e) => panic!("Could not read config: {e}"),
        }
    }
    pub fn parse_args(&mut self) -> Option<(Vec<String>, Vec<String>)> {
        let mut configs = Vec::new();
        let mut args = std::env::args().into_iter();
        let mut next_arg_prefix = None; // if set, the next arg will be prefixed with this.
        let mut next_arg_insert = None; // if set, the next arg will be taken from this before we continue with the actual args iterator.
        loop {
            let arg = if let Some(arg_to_insert) = next_arg_insert.take() {
                if let Some(prefix) = next_arg_prefix.take() {
                    Some(format!("{prefix}{arg_to_insert}"))
                } else {
                    Some(arg_to_insert)
                }
            } else {
                if let Some(arg) = args.next() {
                    if let Some(prefix) = next_arg_prefix.take() {
                        Some(format!("{prefix}{arg}"))
                    } else {
                        Some(arg)
                    }
                } else {
                    None
                }
            };
            if let Some(arg) = arg {
                if arg == "--" {
                    break;
                }
                match arg.chars().next() {
                    Some('c') => {
                        configs.push(arg[1..].to_string());
                    }
                    Some('p') => {
                        self.passwd = Some(arg[1..].to_string());
                    }
                    Some('g') => {
                        self.group = Some(arg[1..].to_string());
                    }
                    Some('G') => {
                        self.groups = Some(arg[1..].to_string());
                    }
                    Some('C') => {
                        self.count = arg[1..]
                            .parse()
                            .expect("Syntax is C[count], where [count] is an integer!");
                    }
                    Some('n') => {
                        self.name = Some(arg[1..].to_string());
                    }
                    Some('h') => {
                        self.home_dir = Some(arg[1..].to_string());
                    }
                    Some('H') => {
                        self.immutable_home_at = Some(arg[1..].to_string());
                    }
                    Some('-') => match match arg[1..].split_once('=') {
                        Some(v) => (v.0, Some(v.1)),
                        None => (&arg[1..], None),
                    } {
                        ("c" | "conf" | "config", v) => {
                            if let Some(v) = v {
                                next_arg_insert = Some(v.to_string());
                            }
                            next_arg_prefix = Some('c');
                        }
                        ("p" | "password", v) => {
                            if let Some(v) = v {
                                next_arg_insert = Some(v.to_string());
                            }
                            next_arg_prefix = Some('p');
                        }
                        ("g" | "group", v) => {
                            if let Some(v) = v {
                                next_arg_insert = Some(v.to_string());
                            }
                            next_arg_prefix = Some('g');
                        }
                        ("G" | "groups", v) => {
                            if let Some(v) = v {
                                next_arg_insert = Some(v.to_string());
                            }
                            next_arg_prefix = Some('G');
                        }
                        ("C" | "count", v) => {
                            if let Some(v) = v {
                                next_arg_insert = Some(v.to_string());
                            }
                            next_arg_prefix = Some('C');
                        }
                        ("n" | "name", v) => {
                            if let Some(v) = v {
                                next_arg_insert = Some(v.to_string());
                            }
                            next_arg_prefix = Some('n');
                        }
                        ("h" | "home" | "home_dir", v) => {
                            if let Some(v) = v {
                                next_arg_insert = Some(v.to_string());
                            }
                            next_arg_prefix = Some('h');
                        }
                        ("H" | "immuthome" | "immutable_home_dir", v) => {
                            if let Some(v) = v {
                                next_arg_insert = Some(v.to_string());
                            }
                            next_arg_prefix = Some('H');
                        }

                        ("test", _) => self.test = true,
                        ("nouserdel", _) => self.userdel = false,
                        ("userhomedel", _) => self.userhomedel = true,
                        ("noconfcmd", _) => self.noconfcmd = true,
                        ("cfgarg", Some(arg)) => {
                            if let Some(VarValue::List(v)) =
                                self.config_script_variables.get_mut("args")
                            {
                                v.push(VarValue::String(arg.to_string()))
                            }
                        }
                        _ => println!("Ignoring unknown argument '{arg}'."),
                    },
                    _ => (),
                }
            } else if configs.is_empty() {
                println!("Syntax is 'd26run [args] -- [command + args]'.");
                println!("The '--' is mandatory unless a config is specified using 'c[path]'.");
                return None;
            } else {
                break;
            }
        }
        Some((configs, args.collect()))
    }
}
