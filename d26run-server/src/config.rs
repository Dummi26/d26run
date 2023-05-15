use std::{collections::HashMap, fs};

use crate::{
    run::{RunCmdBuilder, VarValue},
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
                                let mut runcmd = RunCmdBuilder::default();
                                if let Err(err) = runcmd_from_file(name, &mut runcmd) {
                                    eprintln!(
                                        "[WARN] Skipping file '{}' due to parse error: {err:?}",
                                        name
                                    )
                                }
                                match runcmd.verify() {
                                    Ok(()) => {
                                        eprintln!("[INFO]     added run_cmd {name}");
                                        run_cmds.insert(name.to_owned(), runcmd);
                                    }
                                    Err(err) => {
                                        eprintln!(
                                            "[WARN] Skipping file '{}' due to error:\n    {err:?}",
                                            name
                                        )
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
    for line in file.lines() {
        let (left, right) = if let Some((left, right)) = line.split_once(' ') {
            (left, right)
        } else {
            (line, "")
        };
        match left {
            // comments or empty lines
            s if s.is_empty() || s.starts_with('#') || s.starts_with("//") => (),
            "config" => runcmd_from_file(right, config)?,
            "var" => {
                if let Some((name, value)) = right.split_once(' ') {
                    if let Some((mode, value)) = value.split_once(' ') {
                        if let Some(val) = match mode {
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
                                        default.to_owned(),
                                    ))
                                } else {
                                    eprintln!(
                                        "[WARN] Ignoring var from-input-or without default value)"
                                    );
                                    None
                                }
                            }
                            mode => {
                                eprintln!(
                                    "[WARN] Ignoring var statement with unknown mode '{mode}'."
                                );
                                None
                            }
                        } {
                            config.vars.insert(name.to_owned(), val);
                        }
                    } else {
                        eprintln!("[WARN] Ignoring 'var' statement with only one space.");
                    }
                } else {
                    eprintln!("[WARN] Ignoring bare 'var' statement.");
                }
            }
            "allow" => config.allow = Some(right.to_owned()),
            "command" => config.command = Some(right.to_owned()),
            "args_clear" => config.args.clear(),
            "arg_add" => config.args.push(right.to_owned()),
            "user_inherit" => config.user = Some(None),
            "user_id" => {
                config.user = Some(Some(if let Ok(v) = right.parse() {
                    v
                } else {
                    return Err(ConfigFromFileError::CouldNotParseId(right.to_owned()));
                }))
            }
            "group_inherit" => config.group = Some(None),
            "group_id" => {
                config.group = Some(Some(if let Ok(v) = right.parse() {
                    v
                } else {
                    return Err(ConfigFromFileError::CouldNotParseId(right.to_owned()));
                }))
            }
            "groups_clear" => config.groups.clear(),
            "group_add" => config.groups.push(if let Ok(v) = right.parse() {
                v
            } else {
                return Err(ConfigFromFileError::CouldNotParseId(right.to_owned()));
            }),
            "env_clear" => config.env.clear(),
            "env_add" => config
                .env
                .push(if let Some((name, value)) = right.split_once("=") {
                    (name.into(), Ok(value.into()))
                } else {
                    return Err(ConfigFromFileError::EnvAddWrongSyntax(right.to_owned()));
                }),
            "env_inherit" => {
                if let Some((right, default)) = right.split_once("=") {
                    config.env.push((right.into(), Err(Some(default.into()))));
                } else {
                    config.env.push((right.into(), Err(None)));
                }
            }
            "working_dir" => config.working_dir = Some(right.to_owned()),
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
