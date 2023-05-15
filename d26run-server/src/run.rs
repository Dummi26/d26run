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
    pub user: Option<u32>,
    pub group: Option<u32>,
    pub groups: Vec<u32>,
    pub env: Vec<(String, Result<OsString, Option<String>>)>,
    pub working_dir: Option<String>,
    // pub chroot: Option<String>,
}

#[derive(Default, Debug)]
pub struct RunCmdBuilder {
    // vars
    pub vars: HashMap<String, VarValue>,
    // access
    pub allow: Option<String>,
    // prep
    pub command_prep: Option<(String, Vec<String>)>,
    // what to run
    pub command: Option<String>,
    pub args: Vec<String>,
    pub user: Option<Option<u32>>,
    pub group: Option<Option<u32>>,
    pub groups: Vec<u32>,
    pub env: Vec<(String, Result<String, Option<String>>)>,
    pub working_dir: Option<String>,
    // after
    pub command_after: Option<(String, Vec<String>)>,
    // pub chroot: Option<Option<String>>,
}

#[derive(Debug)]
pub enum VarValue {
    Val(String),
    OutputOf(String, Vec<String>),
    Input(String),
    InputOrDefault(String, String),
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
}

impl RunCmdBuilder {
    pub fn to_runcmd(&self, vars: &HashMap<String, String>) -> Result<RunCmd, ToRunCmdError> {
        self.to_runcmd_(vars, false)
    }
    pub fn verify(&self) -> Result<(), ToRunCmdError> {
        self.to_runcmd_(&HashMap::new(), true).map(|_| ())
    }
    fn to_runcmd_(
        &self,
        vars: &HashMap<String, String>,
        ignore_vars: bool,
    ) -> Result<RunCmd, ToRunCmdError> {
        let mut vars_all: Vec<(String, String)> = if !ignore_vars {
            self.vars
                .iter()
                .map(|(key, value)| {
                    Ok((
                        key.to_owned(),
                        match value {
                            VarValue::Val(v) => v.to_owned(),
                            VarValue::OutputOf(exec, args) => {
                                let mut cmd = Command::new(exec);
                                cmd.args(args);
                                if let Ok(output) = cmd.output() {
                                    String::from_utf8_lossy(&output.stdout).into_owned()
                                } else {
                                    return Err(ToRunCmdError::VarFailedToRun(
                                        exec.to_owned(),
                                        args.clone(),
                                    ));
                                }
                            }
                            VarValue::Input(arg_name) => {
                                if let Some(val) = vars.get(arg_name) {
                                    val.to_owned()
                                } else {
                                    return Err(ToRunCmdError::VarMissingInput(
                                        arg_name.to_owned(),
                                    ));
                                }
                            }
                            VarValue::InputOrDefault(arg_name, default) => {
                                if let Some(val) = vars.get(arg_name) {
                                    val.to_owned()
                                } else {
                                    default.to_owned()
                                }
                            }
                        },
                    ))
                })
                .collect::<Result<_, _>>()?
        } else {
            vec![]
        };
        vars_all.sort_by(|(a, _), (b, _)| a.cmp(b));
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
        Ok(RunCmd {
            command: self
                .command
                .as_ref()
                .map(f)
                .ok_or_else(|| ToRunCmdError::MissingFieldCommand)?,
            args: self.args.iter().map(f).collect(),
            user: self
                .user
                .clone()
                .ok_or_else(|| ToRunCmdError::MissingFieldUser)?,
            group: self
                .group
                .clone()
                .ok_or_else(|| ToRunCmdError::MissingFieldGroup)?,
            groups: self.groups.clone(),
            env: self
                .env
                .iter()
                .map(|(name, val)| {
                    (
                        name.to_owned(),
                        match val {
                            Ok(val) => Ok(f(val).into()),
                            Err(None) => Err(None),
                            Err(Some(def)) => Err(Some(f(def))),
                        },
                    )
                })
                .collect(),
            working_dir: self.working_dir.as_ref().map(f),
        })
    }
}

pub struct Runner {
    cfg: RunCmd,
    pub child_process: Option<Child>,
}
impl Runner {
    pub fn new(cfg: RunCmd) -> Self {
        Self {
            cfg,
            child_process: None,
        }
    }
    pub fn start(&mut self) {
        let mut command = Command::new(&self.cfg.command);
        command.args(&self.cfg.args);
        if let Some(v) = self.cfg.user {
            command.uid(v);
        }
        if let Some(v) = self.cfg.group {
            command.gid(v);
        }
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command.groups(self.cfg.groups.as_slice().into());
        command.env_clear();
        command.envs(self.cfg.env.iter().filter_map(|(name, val)| {
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
        if let Some(dir) = &self.cfg.working_dir {
            command.current_dir(dir);
        }
        eprintln!("Spawning {:?}", command);
        match command.spawn() {
            Ok(child_proc) => self.child_process = Some(child_proc),
            Err(e) => eprintln!("[WARN] failed to spawn child process: {e:?}"),
        }
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
        }
    }
}
