use std::{collections::HashMap, ffi::OsString, os::unix::process::CommandExt, process::Command};

#[derive(Clone, Debug)]
pub struct RunCmd {
    pub defaults: HashMap<String, String>,
    pub command: String,
    pub args: Vec<String>,
    pub user: Option<u32>,
    pub group: Option<u32>,
    pub groups: Vec<u32>,
    pub env: Vec<(String, Result<OsString, Option<String>>)>,
    pub working_dir: Option<String>,
    // pub chroot: Option<String>,
}

#[derive(Default)]
pub struct RunCmdBuilder {
    pub default_vars: HashMap<String, String>,
    pub allow: Option<String>,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub user: Option<Option<u32>>,
    pub group: Option<Option<u32>>,
    pub groups: Vec<u32>,
    pub env: Vec<(String, Result<String, Option<String>>)>,
    pub working_dir: Option<String>,
    // pub chroot: Option<Option<String>>,
}

impl RunCmdBuilder {
    pub fn to_runcmd_with_vars(&self, vars: &HashMap<String, String>) -> Option<RunCmd> {
        self.to_runcmd_(Some(vars))
    }
    pub fn to_runcmd(&self) -> Option<RunCmd> {
        self.to_runcmd_(None)
    }
    fn to_runcmd_(&self, vars: Option<&HashMap<String, String>>) -> Option<RunCmd> {
        let mut vars_all: Vec<(_, _)> = self
            .default_vars
            .iter()
            .map(|(key, value)| {
                if let Some(vars) = vars {
                    if let Some(value) = vars.get(key) {
                        (key.to_owned(), value.to_owned())
                    } else {
                        (key.to_owned(), value.to_owned())
                    }
                } else {
                    (key.to_owned(), value.to_owned())
                }
            })
            .collect();
        vars_all.sort_by(|(a, _), (b, _)| a.cmp(b));
        let f = |val: &String| {
            let mut out = val.to_owned();
            for (key, val) in &vars_all {
                out = out.replace(key.as_str(), &val);
            }
            out
        };
        Some(RunCmd {
            defaults: self
                .default_vars
                .iter()
                .map(|(name, val)| (name.to_owned(), f(val)))
                .collect(),
            command: f(self.command.as_ref()?),
            args: self.args.iter().map(f).collect(),
            user: self.user.clone()?,
            group: self.group.clone()?,
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
            working_dir: match &self.working_dir {
                Some(v) => Some(f(v)),
                None => None,
            },
        })
    }
}

pub struct Runner {
    cfg: RunCmd,
}
impl Runner {
    pub fn new(cfg: RunCmd) -> Self {
        Self { cfg }
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
        command.groups(&self.cfg.groups[..]);
        command
            .env_clear()
            .envs(self.cfg.env.iter().filter_map(|(name, val)| {
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
        if let Err(_e) = command.spawn() {
            // do smth
        }
    }
}
