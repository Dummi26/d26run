use std::{
    collections::HashMap,
    ffi::OsString,
    fmt::Display,
    os::unix::process::CommandExt,
    process::{Child, Command, Stdio},
};

#[derive(Clone, Debug)]
pub struct RunCmd {
    pub command: String,
    pub args: Vec<String>,
    pub user: u32,
    pub group: u32,
    pub groups: Vec<u32>,
    pub env: Vec<(String, Result<OsString, Option<String>>)>,
    pub working_dir: Option<String>,
    // pub chroot: Option<String>,
    pub command_clean: Vec<Self>,
}

#[derive(Default, Debug)]
pub struct RunCmdBuilder {
    // vars
    pub vars: HashMap<String, VarValue>,
    // access
    pub allow: Option<String>,
    // prep and clean (runs before/after command)
    pub command_prep: Vec<Self>,
    pub command_clean: Vec<Self>,
    // what to run
    pub command: Option<String>,
    pub args: Vec<String>,
    pub user: Option<Result<u32, String>>,
    pub group: Option<Result<u32, String>>,
    pub groups: Vec<Result<u32, String>>,
    pub env: Vec<(String, Result<String, Option<String>>)>,
    pub working_dir: Option<String>,
    // pub chroot: Option<Option<String>>,
}

#[derive(Debug)]
pub enum VarValue {
    Val(String),
    OutputOf(String, Vec<String>),
    Input(String),
    InputOrDefault(String, Box<Self>),
    ConId,
}

#[derive(Debug)]
pub enum ToRunCmdError {
    // missing fields
    MissingFieldCommand,
    MissingFieldUser,
    MissingFieldGroup,
    // variables
    VarFailedToRun(String, Vec<String>),
    VarMissingInput(String),
    // unknown user/group
    UnknownUser(String),
    UnknownGroup(String),
}

pub struct ToRunCmdInfo {
    pub con_id: u128,
}

impl RunCmdBuilder {
    pub fn to_runcmd(
        &self,
        vars: &HashMap<String, String>,
        info: &ToRunCmdInfo,
    ) -> Result<RunCmd, Vec<ToRunCmdError>> {
        self.to_runcmd_wrapper(vars, info, None, false)
    }
    pub fn to_runcmd_check(
        &self,
        vars: &HashMap<String, String>,
        info: &ToRunCmdInfo,
    ) -> Result<RunCmd, Vec<ToRunCmdError>> {
        self.to_runcmd_wrapper(vars, info, None, true)
    }
    pub fn to_runcmd_with_existing_vars(
        &self,
        vars: &HashMap<String, String>,
        info: &ToRunCmdInfo,
        existing_vars: &Vec<(String, String)>,
    ) -> Result<RunCmd, Vec<ToRunCmdError>> {
        self.to_runcmd_wrapper(vars, info, Some(existing_vars), false)
    }
    fn to_runcmd_wrapper(
        &self,
        vars: &HashMap<String, String>,
        info: &ToRunCmdInfo,
        existing_vars: Option<&Vec<(String, String)>>,
        just_check: bool,
    ) -> Result<RunCmd, Vec<ToRunCmdError>> {
        let mut errors = Vec::new();
        let o = self.to_runcmd_(vars, existing_vars, info, &mut errors, just_check);
        if errors.is_empty() {
            if let Some(o) = o {
                Ok(o)
            } else {
                Err(errors)
            }
        } else {
            Err(errors)
        }
    }
    pub fn verify(
        &self,
        info: &ToRunCmdInfo,
    ) -> (Vec<ToRunCmdError>, Result<(), Vec<ToRunCmdError>>) {
        match self.to_runcmd_check(&HashMap::new(), info).map(|_| ()) {
            Ok(_) => (vec![], Ok(())),
            Err(e) => {
                let mut nf = vec![];
                let mut fatal = vec![];
                for e in e {
                    match e {
                        // these can still change -> not fatal
                        ToRunCmdError::VarFailedToRun(..)
                        | ToRunCmdError::VarMissingInput(..)
                        | ToRunCmdError::UnknownUser(_)
                        | ToRunCmdError::UnknownGroup(_) => nf.push(e),
                        e => fatal.push(e),
                    }
                }
                if fatal.is_empty() {
                    (nf, Ok(()))
                } else {
                    (nf, Err(fatal))
                }
            }
        }
    }
    fn to_runcmd_(
        &self,
        vars: &HashMap<String, String>,
        existing_vars: Option<&Vec<(String, String)>>,
        info: &ToRunCmdInfo,
        es: &mut Vec<ToRunCmdError>,
        just_check: bool,
    ) -> Option<RunCmd> {
        fn er<T>(r: Result<T, ToRunCmdError>, def: T, errors: &mut Vec<ToRunCmdError>) -> T {
            match r {
                Ok(v) => v,
                Err(e) => {
                    errors.push(e);
                    def
                }
            }
        }
        fn erd<T: Default>(r: Result<T, ToRunCmdError>, errors: &mut Vec<ToRunCmdError>) -> T {
            er(r, T::default(), errors)
        }
        fn map_var_fn(
            v: (&String, &VarValue),
            vars: &HashMap<String, String>,
            info: &ToRunCmdInfo,
        ) -> Result<String, ToRunCmdError> {
            let (key, value) = v;
            Ok(match value {
                VarValue::Val(v) => v.to_owned(),
                VarValue::OutputOf(exec, args) => {
                    let mut cmd = Command::new(exec);
                    cmd.args(args);
                    if let Ok(output) = cmd.output() {
                        String::from_utf8_lossy(&output.stdout).into_owned()
                    } else {
                        return Err(ToRunCmdError::VarFailedToRun(exec.to_owned(), args.clone()));
                    }
                }
                VarValue::Input(arg_name) => {
                    if let Some(val) = vars.get(arg_name) {
                        val.to_owned()
                    } else {
                        return Err(ToRunCmdError::VarMissingInput(arg_name.to_owned()));
                    }
                }
                VarValue::InputOrDefault(arg_name, default) => {
                    if let Some(val) = vars.get(arg_name) {
                        val.to_owned()
                    } else {
                        map_var_fn((key, default), vars, info)?
                    }
                }
                VarValue::ConId => format!("{}", info.con_id),
            })
        }
        let mut vars_all: Vec<(String, String)> = {
            let mut vars_all = Vec::with_capacity(self.vars.len());
            vars_all.reserve(self.vars.len());
            if let Some(v) = existing_vars {
                for (name, val) in v {
                    if !self.vars.contains_key(name) {
                        vars_all.push((name.to_owned(), val.to_owned()))
                    }
                }
            }
            for var in self.vars.iter() {
                vars_all.push((var.0.to_owned(), erd(map_var_fn(var, vars, info), es)));
            }
            vars_all
        };
        vars_all.sort_by(|(a, _), (b, _)| a.cmp(b));
        // makes variables work in this string
        let f = |val: &String| {
            let mut out = String::new();
            let mut vars_local: Vec<(Vec<char>, _, usize, usize)> = vars_all
                .iter()
                .filter(|(key, _)| !key.is_empty())
                .map(|(key, val)| (key.chars().collect(), val, 0, 0))
                .collect();
            for (byte_pos, ch) in val.char_indices() {
                out.push(ch);
                for (var_name, var_value, start, len) in vars_local.iter_mut() {
                    if ch == var_name[*len] {
                        if *len == 0 {
                            *start = byte_pos;
                        }
                        *len += 1;
                        if *len >= var_name.len() {
                            // end of varname
                            // remove the varname from the string (it has already been .push()ed)
                            out.truncate(*start);
                            // then add the variables value
                            out.push_str(var_value);
                            // reset the len of everything
                            for (_, _, _, len) in vars_local.iter_mut() {
                                *len = 0;
                            }
                            // no need to check the other variables
                            break;
                        }
                    } else {
                        *len = 0;
                    }
                }
            }
            out
        };
        if !just_check {
            for cmd in &self.command_prep {
                match cmd.to_runcmd_with_existing_vars(vars, info, &vars_all) {
                    Ok(v) => Runner::new_prep_or_clean(v).start().wait(),
                    Err(e) => {
                        es.extend(e);
                        return None;
                    }
                }
            }
        }
        Some(RunCmd {
            command: erd(
                self.command
                    .as_ref()
                    .map(f)
                    .ok_or_else(|| ToRunCmdError::MissingFieldCommand),
                es,
            ),
            args: self.args.iter().map(f).collect(),
            user: match er(
                self.user
                    .clone()
                    .ok_or_else(|| ToRunCmdError::MissingFieldUser),
                Ok(0),
                es,
            ) {
                Ok(id) => id,
                Err(name) => {
                    let name = f(&name);
                    erd(
                        match users::get_user_by_name(&name) {
                            Some(user) => Ok(user.uid()),
                            None => Err(ToRunCmdError::UnknownUser(name)),
                        },
                        es,
                    )
                }
            },
            group: match er(
                self.user
                    .clone()
                    .ok_or_else(|| ToRunCmdError::MissingFieldGroup),
                Ok(0),
                es,
            ) {
                Ok(id) => id,
                Err(name) => {
                    let name = f(&name);
                    erd(
                        match users::get_group_by_name(&name) {
                            Some(group) => Ok(group.gid()),
                            None => Err(ToRunCmdError::UnknownGroup(name)),
                        },
                        es,
                    )
                }
            },
            groups: {
                let mut o = Vec::with_capacity(self.groups.len());
                for group in self.groups.iter() {
                    o.push(match group {
                        Ok(v) => *v,
                        Err(name) => {
                            let name = f(name);
                            erd(
                                match users::get_group_by_name(&name) {
                                    Some(group) => Ok(group.gid()),
                                    None => Err(ToRunCmdError::UnknownGroup(name)),
                                },
                                es,
                            )
                        }
                    });
                }
                o
            },
            env: self
                .env
                .iter()
                .map(|(name, val)| {
                    (
                        f(name),
                        match val {
                            Ok(val) => Ok(f(val).into()),
                            Err(None) => Err(None),
                            Err(Some(def)) => Err(Some(f(def))),
                        },
                    )
                })
                .collect(),
            working_dir: self.working_dir.as_ref().map(f),
            command_clean: self
                .command_clean
                .iter()
                .filter_map(
                    |v| match v.to_runcmd_with_existing_vars(vars, info, &vars_all) {
                        Ok(v) => Some(v),
                        Err(e) => {
                            es.extend(e);
                            None
                        }
                    },
                )
                .collect(),
        })
    }
}

pub struct Runner {
    cmd: RunCmd,
    is_inner: bool,
    pub child_process: Option<Child>,
}
impl Runner {
    pub fn new(cmd: RunCmd) -> Self {
        Self {
            cmd,
            is_inner: false,
            child_process: None,
        }
    }
    pub fn new_prep_or_clean(cmd: RunCmd) -> Self {
        Self {
            cmd,
            is_inner: true,
            child_process: None,
        }
    }
    pub fn start(&mut self) -> &mut Self {
        let cmd = &mut self.cmd;
        let mut command = Command::new(&cmd.command);
        command.args(&cmd.args);
        command.uid(cmd.user);
        command.gid(cmd.group);
        if !self.is_inner {
            command
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
        }
        command.groups(cmd.groups.as_slice().into());
        command.env_clear();
        command.envs(cmd.env.iter().filter_map(|(name, val)| {
            Some((
                name.clone(),
                match val {
                    Ok(v) => v.clone(),
                    Err(default) => {
                        if let Some(v) = std::env::var_os(name) {
                            v
                        } else if let Some(default) = default {
                            default.into()
                        } else {
                            return None;
                        }
                    }
                },
            ))
        }));
        if let Some(dir) = &cmd.working_dir {
            command.current_dir(dir);
        }
        eprintln!(
            "Spawning {:?}:\n    args: {:?}\n     env: {:?}\n   cwdir: {:?}",
            command.get_program(),
            command.get_args(),
            command.get_envs(),
            command.get_current_dir(),
        );
        if !self.is_inner {
            eprintln!(" ~ ~ ~ ~ ~ running...");
        }
        match command.spawn() {
            Ok(child_proc) => self.child_process = Some(child_proc),
            Err(e) => {
                eprintln!("[WARN] failed to spawn child process: {e:?}");
            }
        }
        self
    }
    /// note: automatically called on drop - to prevent blocking, run wait in a thread and move the runner to that thread.
    pub fn wait(&mut self) {
        if let Some(v) = &mut self.child_process {
            _ = v.wait();
            if !self.is_inner {
                eprintln!(" ~ ~ ~ ~ ~ cleaning...");
            }
            for cmd in std::mem::replace(&mut self.cmd.command_clean, vec![]) {
                Runner::new_prep_or_clean(cmd).start().wait();
            }
            if !self.is_inner {
                eprintln!(" ~ ~ ~ ~ ~ done.");
            }
        }
    }
}
impl Drop for Runner {
    fn drop(&mut self) {
        self.wait()
    }
}

impl Display for ToRunCmdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingFieldCommand => write!(f, "missing field 'command'"),
            Self::MissingFieldUser => write!(f, "missing field 'user'"),
            Self::MissingFieldGroup => write!(f, "missing field 'group'"),
            Self::VarFailedToRun(exec, args) => {
                write!(f, "var: failed to run command {exec:?} with args {args:?}")
            }
            Self::VarMissingInput(input) => write!(f, "var: missing input '{input}'"),
            Self::UnknownUser(n) => write!(f, "unknown user '{n}' (couldn't find uid)!"),
            Self::UnknownGroup(n) => write!(f, "unknown group '{n}' (couldn't find gid)!"),
        }
    }
}
