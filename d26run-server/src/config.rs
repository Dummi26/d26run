use std::{collections::HashMap, fs};

use crate::{
    run::{RunCmdBuilder, ToRunCmdInfo, VarValue},
    DIR_CONFIGS,
};

pub struct Config {
    /// a set of configs loaded from DIR_CONFIGS
    pub run_cmds: HashMap<String, RunCmdBuilder>,
    /// maps a regex (file path) to a run_cmds config that will be used to open the file.
    pub open_cmds: HashMap<String, String>,
}
pub fn init() -> Config {
    Config {
        run_cmds: if let Ok(dir) = fs::read_dir(DIR_CONFIGS) {
            eprintln!("[INFO] now loading run_cmds from '{DIR_CONFIGS}'.");
            let mut run_cmds = HashMap::new();
            for entry in dir {
                if let Ok(e) = entry {
                    let file_name = e.file_name();
                    if let Ok(file_type) = e.file_type() {
                        if file_type.is_file() {
                            if let Some(name) = file_name.to_str() {
                                eprintln!("[INFO] Now parsing {name}.");
                                let mut runcmd = RunCmdBuilder::default();
                                if let Err(err) = runcmd_from_file(name, &mut runcmd) {
                                    eprintln!(
                                        "[WARN] Skipping file '{}' due to parse error: {err:?}",
                                        name
                                    )
                                } else {
                                    let (non_fatal, out) =
                                        runcmd.verify(&ToRunCmdInfo { con_id: 0 });
                                    for e in non_fatal {
                                        eprintln!("[INFO]     non-fatal: {e}");
                                    }
                                    match out {
                                        Ok(()) => {
                                            eprintln!("[INFO]    + added run_cmd {name}");
                                            run_cmds.insert(name.to_owned(), runcmd);
                                        }
                                        Err(err) => {
                                            for e in err {
                                                eprintln!("[INFO]     ! fatal !: {e}");
                                            }
                                            eprintln!(
                                                "[WARN] Skipping file '{}' due to error.",
                                                name
                                            )
                                        }
                                    }
                                }
                            } else {
                                eprintln!(
                                    "[WARN] Skipping file with invalid name: '{}'.",
                                    e.file_name().to_string_lossy()
                                );
                            }
                        }
                    } else {
                        eprintln!(
                            "[WARN] Couldn't get file type for DIR_CONFIGS entry '{}', skipping.",
                            file_name.to_string_lossy()
                        );
                    }
                } else {
                    eprintln!("[WARN] Couldn't read an entry in DIR_CONFIGS (might skip a file?).",);
                }
            }
            eprintln!("[INFO] Loaded {} run_cmds.", run_cmds.len());
            run_cmds
        } else {
            eprintln!(
                "Warn: couldn't read directory '{DIR_CONFIGS}', so no configs will be loaded!"
            );
            HashMap::new()
        },
        open_cmds: HashMap::new(),
    }
}

pub fn runcmd_from_file(name: &str, config: &mut RunCmdBuilder) -> Result<(), ConfigFromFileError> {
    let file = std::fs::read_to_string(format!("{DIR_CONFIGS}{name}"))?;
    runcmd_from_lines(name, config, &mut file.lines().map(|v| v.to_owned()))
}
pub fn runcmd_from_lines<L: Iterator<Item = String>>(
    name: &str,
    config: &mut RunCmdBuilder,
    lines: &mut L,
) -> Result<(), ConfigFromFileError> {
    loop {
        let line = if let Some(l) = lines.next() {
            l
        } else {
            break;
        };
        let (left, right) = if let Some((left, right)) = line.split_once(' ') {
            (left, right)
        } else {
            (line.as_str(), "")
        };
        match left {
            // comments or empty lines
            s if s.is_empty() || s.starts_with('#') || s.starts_with("//") => (),
            "end" => break,
            "config" => {
                runcmd_from_file(right, config)?;
            }
            "var" => {
                if let Some((name, value)) = right.split_once(' ') {
                    fn get_var_val(name: &str, value: &str) -> Option<VarValue> {
                        let (mode, value) = if let Some(v) = value.split_once(' ') {
                            v
                        } else {
                            (value, "")
                        };
                        match mode {
                            "set" => Some(VarValue::Val(value.to_owned())),
                            "from-cmd" => Some(VarValue::OutputOf(value.to_owned(), vec![])),
                            "from-cmd-sh" => Some(VarValue::OutputOf(
                                "sh".to_owned(),
                                vec!["-c".to_owned(), value.to_owned()],
                            )),
                            "from-input" => Some(VarValue::Input(value.to_owned())),
                            "from-input-or" => {
                                if let Some((input, default)) = value.split_once(' ') {
                                    Some(VarValue::InputOrDefault(
                                        input.to_owned(),
                                        Box::new(VarValue::Val(default.to_owned())),
                                    ))
                                } else {
                                    eprintln!(
                                        "[WARN] Ignoring var from-input-or without default value)"
                                    );
                                    None
                                }
                            }
                            "from-input-or-else" => {
                                if let Some((input, default)) = value.split_once(' ') {
                                    Some(VarValue::InputOrDefault(
                                        input.to_owned(),
                                        Box::new(get_var_val(
                                            &(name.to_owned() + " / default"),
                                            default,
                                        )?),
                                    ))
                                } else {
                                    eprintln!(
                                        "[WARN] Ignoring var from-input-or without default value)"
                                    );
                                    None
                                }
                            }
                            "con-id" => Some(VarValue::ConId),
                            mode => {
                                eprintln!(
                                    "[WARN] Ignoring var statement with unknown mode '{mode}'."
                                );
                                None
                            }
                        }
                    }
                    if let Some(val) = get_var_val(name, value) {
                        config.vars.insert(name.to_owned(), val);
                    }
                } else {
                    eprintln!("[WARN] Ignoring bare 'var' statement.");
                }
            }
            "allow" => config.allow = Some(right.to_owned()),
            "cmd-prep" => config.command_prep.push({
                let mut cfg = RunCmdBuilder::default();
                runcmd_from_lines(&format!("{name}/prep"), &mut cfg, lines)?;
                cfg
            }),
            "cmd-clean" => config.command_clean.push({
                let mut cfg = RunCmdBuilder::default();
                runcmd_from_lines(&format!("{name}/prep"), &mut cfg, lines)?;
                cfg
            }),
            "command" => config.command = Some(right.to_owned()),
            "args-clear" => config.args.clear(),
            "arg" => config.args.push(right.to_owned()),
            "uid" => {
                config.user = Some(Ok(if let Ok(v) = right.parse() {
                    v
                } else {
                    return Err(ConfigFromFileError::CouldNotParseId(right.to_owned()));
                }))
            }
            "user" => config.user = Some(Err(right.to_owned())),
            "gid" => {
                config.group = Some(Ok(if let Ok(v) = right.parse() {
                    v
                } else {
                    return Err(ConfigFromFileError::CouldNotParseId(right.to_owned()));
                }))
            }
            "group" => config.group = Some(Err(right.to_owned())),
            "g-clear" => config.groups.clear(),
            "g+gid" => config.groups.push(if let Ok(v) = right.parse() {
                Ok(v)
            } else {
                return Err(ConfigFromFileError::CouldNotParseId(right.to_owned()));
            }),
            "g+group" => config.groups.push(Err(right.to_owned())),
            "env-clear" => config.env.clear(),
            "env+set" => config
                .env
                .push(if let Some((name, value)) = right.split_once("=") {
                    (name.into(), Ok(value.into()))
                } else {
                    return Err(ConfigFromFileError::EnvAddWrongSyntax(right.to_owned()));
                }),
            "env+inherit" => {
                if let Some((right, default)) = right.split_once("=") {
                    config.env.push((right.into(), Err(Some(default.into()))));
                } else {
                    config.env.push((right.into(), Err(None)));
                }
            }
            "working-dir" => config.working_dir = Some(right.to_owned()),
            v => return Err(ConfigFromFileError::UnknownStatement(v.to_owned())),
        }
    }
    Ok(())
}

#[derive(Debug)]
pub enum ConfigFromFileError {
    IoError(std::io::Error),
    UnknownStatement(String),
    CouldNotParseId(String),
    EnvAddWrongSyntax(String),
}
impl From<std::io::Error> for ConfigFromFileError {
    fn from(value: std::io::Error) -> Self {
        Self::IoError(value)
    }
}
