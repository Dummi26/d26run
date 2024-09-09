#![feature(setgroups)]

use std::{
    env::args,
    os::unix::process::CommandExt,
    path::Path,
    process::{exit, Command}, sync::{atomic::AtomicBool, Arc},
};

use users::os::unix::UserExt;

fn main() -> std::io::Result<()> {
    let current_command = Arc::new(AtomicBool::new(false));
    let current_command_h = Arc::clone(&current_command);
    let _ = ctrlc::set_handler(move || if !current_command_h.load(std::sync::atomic::Ordering::Relaxed) {
        eprintln!("Received signal, exiting");
        exit(255);
    });
    let mut cli_args = args().skip(1);
    let exec = match cli_args.next() {
        Some(v) => v,
        None => {
            eprintln!(
                "Mandatory argument missing: exec (path in and relative to /etc/d26run/exec/)"
            );
            exit(255);
        }
    };
    let mut exec_split = exec.split('/');
    if let Some(ex) = exec_split.next() {
        if let Some(file) = find_first_exec("/etc/d26run/exec/", ex, exec_split) {
            match std::fs::read_to_string(file.path()) {
                Ok(file) => {
                    let mut allowed = false;
                    let mut current_uid = None;
                    let mut current_uid =
                        || *current_uid.get_or_insert_with(|| users::get_current_uid());
                    let mut current_groups = None;
                    let mut p_user = None;
                    let mut p_group = None;
                    let mut p_groups = vec![];
                    let mut working_dir = None;
                    let mut env_changes = vec![];
                    let mut execs = vec![];
                    for line in file.lines() {
                        if !(line.is_empty() || line.starts_with('#')) {
                            if line.starts_with("allow ") {
                                if let Some(user) = line.strip_prefix("allow user ") {
                                    if !allowed {
                                        if let Some(uid) = user.parse::<u32>().ok().or_else(|| {
                                            users::get_user_by_name(user).map(|v| v.uid())
                                        }) {
                                            if uid == current_uid() {
                                                allowed = true;
                                            }
                                        }
                                    }
                                } else if let Some(group) = line.strip_prefix("allow group ") {
                                    if !allowed {
                                        if let Some(gid) = group.parse::<u32>().ok().or_else(|| {
                                            users::get_group_by_name(group).map(|v| v.gid())
                                        }) {
                                            if current_groups
                            .get_or_insert_with(|| {
                                users::group_access_list()
                                    .map(|v| v.into_iter().map(|v| v.gid()).collect::<Vec<_>>())
                            })
                            .as_ref().is_ok_and(|v| v.contains(&gid)) {
                                                allowed = true;
                                            }
                                        }
                                    }
                                } else if line == "allow anyone" {
                                    allowed = true;
                                } else {
                                    eprintln!(
                                        "Invalid line: '{line}'. Expected 'allow user ...' or 'allow group ...'."
                                    );
                                    exit(255);
                                }
                            } else if line == "new" {
                                p_user = None;
                                p_group = None;
                                p_groups = vec![];
                                working_dir = None;
                                env_changes = vec![];
                            } else if let Some(u) = line.strip_prefix("user ") {
                                p_user = Some(u.parse::<u32>().map_err(|_| u.to_owned()));
                            } else if let Some(g) = line.strip_prefix("group ") {
                                p_group = Some(g.parse::<u32>().map_err(|_| g.to_owned()));
                            } else if let Some(g) = line.strip_prefix("groups = ") {
                                p_groups.clear();
                                for g in g.split_whitespace() {
                                    p_groups.push(g.parse::<u32>().map_err(|_| g.to_owned()));
                                }
                            } else if let Some(g) = line.strip_prefix("groups + ") {
                                p_groups.push(g.parse::<u32>().map_err(|_| g.to_owned()));
                            } else if let Some(wd) = line.strip_prefix("working-dir ") {
                                working_dir = Some(wd.to_owned());
                            } else if let Some(env) = line.strip_prefix("env ") {
                                if env == "clear" {
                                    env_changes.push(None);
                                } else if let Some(key) = env.strip_prefix("unset ") {
                                    env_changes.push(Some((key.to_owned(), None)));
                                } else if let Some(pair) = env.strip_prefix("set ") {
                                    let (key, val) = pair.split_once(' ').unwrap_or((pair, ""));
                                    env_changes.push(Some((key.to_owned(), Some(val.to_owned()))));
                                } else {
                                    eprintln!(
                                        "Invalid line: '{line}'. Expected 'env clear', 'env unset <key>', or 'env set <key> <env value>'."
                                    );
                                    exit(255);
                                }
                            } else if let Some(exec) = line.strip_prefix("exec ") {
                                if let Some(u) = p_user.clone() {
                                    if let Some(g) = p_group.clone() {
                                        execs.push((
                                            u,
                                            g,
                                            p_groups.clone(),
                                            exec.to_owned(),
                                            vec![],
                                            working_dir.clone(),
                                            env_changes.clone(),
                                        ));
                                    } else {
                                        eprintln!(
                                        "Invalid line: '{line}'. Expected 'exec ...' only after 'group ...'."
                                    );
                                        exit(255);
                                    }
                                } else {
                                    eprintln!(
                                        "Invalid line: '{line}'. Expected 'exec ...' only after 'user ...'."
                                    );
                                    exit(255);
                                }
                            } else if let Some(arg) = line.strip_prefix("arg ") {
                                if let Some(exec) = execs.last_mut() {
                                    exec.4.push(arg.to_owned());
                                } else {
                                    eprintln!(
                                        "Invalid line: '{line}'. Expected 'arg ...' only after 'exec ...'."
                                    );
                                    exit(255);
                                }
                            } else if let Some(args) = line.strip_prefix("args ") {
                                if let Some(exec) = execs.last_mut() {
                                    if args == "all" {
                                        exec.4.extend(&mut cli_args);
                                    } else if let Some(v) = args.parse::<usize>().ok().filter(|v| *v > 0) {
                                        for _ in 0..v {
                                            if let Some(arg) = cli_args.next() {
                                                exec.4.push(arg);
                                            } else {
                                                break;
                                            }
                                        }
                                    } else {
                                        eprintln!(
                                            "Invalid line: '{line}'. Expected 'args all' or 'args <integer>', where <integer> >= 1."
                                        );
                                        exit(255);
                                    }
                                } else {
                                    eprintln!(
                                        "Invalid line: '{line}'. Expected 'args ...' only after 'exec ...'."
                                    );
                                    exit(255);
                                }
                            } else {
                                eprintln!(
                                    "Invalid line: '{line}'. Expected 'allow ...', 'user ...', 'group ...', 'groups = ...', 'groups + ...', 'working-dir ...', 'env ...', 'exec ...', 'arg ...', 'args', or 'new'."
                                );
                                exit(255);
                            }
                        }
                    }
                    if allowed {
                        for (u, g, gs, exec, args, working_dir, env_changes) in execs {
                            if let Some(u) = u.as_ref()
                                .map(|v| Some(*v))
                                .unwrap_or_else(|u| users::get_user_by_name(&u).map(|v| v.uid()))
                            {
                                if let Some(g) = g.as_ref().map(|v| Some(*v)).unwrap_or_else(|g| {
                                    users::get_group_by_name(&g).map(|v| v.gid())
                                }) {
                                    if Path::new(&exec).is_absolute() {
                                        let user = users::get_user_by_uid(u);
                                        let mut cmd = Command::new(&exec);
                                        cmd.uid(u).gid(g).args(&args);
                                        if let Some(user) = &user {
                                            cmd.env("HOME", user.home_dir());
                                        } else {
                                            cmd.env_remove("HOME");
                                        }
                                        cmd.groups(gs.iter().filter_map(|g| if let Some(g) = g.as_ref().map(|v| Some(*v)).unwrap_or_else(|g| users::get_group_by_name(g).map(|g| g.gid())) { Some(g) } else {
                                            eprintln!("Group '{}' not found, not adding it to this exec.", g.as_ref().unwrap_err());
                                            None
                                        }).collect::<Vec<_>>().as_slice());
                                        if let Some(wd) = working_dir {
                                            cmd.current_dir(wd);
                                        } else if let Some(user) = &user {
                                            cmd.current_dir(user.home_dir());
                                        }
                                        for env_change in env_changes {
                                            match env_change {
                                                None => {
                                                    cmd.env_clear();
                                                }
                                                Some((key, None)) => {
                                                    cmd.env_remove(key);
                                                }
                                                Some((key, Some(val))) => {
                                                    cmd.env(key, val);
                                                }
                                            }
                                        }
                                        current_command.store(true, std::sync::atomic::Ordering::Relaxed);
                                        match cmd.status() {
                                            Ok(_) => (),
                                            Err(e) => {
                                                let exec_ws = if exec.contains(char::is_whitespace) {
                                                    "\""
                                                } else {
                                                    ""
                                                };
                                                eprintln!(
                                                    "Error running command '{exec_ws}{}{exec_ws}{}': {e}", exec.replace('\\', "\\\\").replace('"', "\\\""), args.iter().map(|a| {
                                                        let arg_ws = if a.contains(char::is_whitespace) {
                                                            "\""
                                                        } else {
                                                            ""
                                                        };
                                                        format!(" {arg_ws}{}{arg_ws}", a.replace('\\', "\\\\").replace('"', "\\\""))
                                                    
                                                    }).collect::<String>()
                                                );
                                            }
                                        }
                                        current_command.store(false, std::sync::atomic::Ordering::Relaxed);
                                    } else {
                                        eprintln!("Exec path '{exec}' must be an absolute path! skipping this exec.");
                                    }
                                } else {
                                    eprintln!(
                                        "Group '{}' not found, skipping this exec!",
                                        g.unwrap_err()
                                    );
                                }
                            } else {
                                eprintln!(
                                    "User '{}' not found, skipping this exec!",
                                    u.unwrap_err()
                                );
                            }
                        }
                    } else {
                        eprintln!("You are not allowed to use this exec!");
                    }
                }
                Err(e) => {
                    eprintln!("Couldn't read exec file '{exec}': {e}");
                    exit(255);
                }
            }
        } else {
            eprintln!("Exec file '{exec}' not found");
            exit(255);
        }
    } else {
        eprintln!(
            "Mandatory argument missing or empty: exec (path in and relative to /etc/d26run/exec/)"
        );
        exit(255);
    }
    Ok(())
}

fn find_first_exec(
    path: &(impl AsRef<Path> + ?Sized),
    ex: &str,
    mut exec: std::str::Split<char>,
) -> Option<std::fs::DirEntry> {
    for conf in match std::fs::read_dir(path.as_ref()) {
        Ok(v) => v,
        Err(e) => {
            eprintln!(
                "Couldn't list files in {:?}: {e}",
                path.as_ref().to_string_lossy()
            );
            exit(255);
        }
    }
    .filter_map(Result::ok)
    {
        if conf.file_name().as_os_str() == std::ffi::OsStr::new(ex) {
            if let Some(ex2) = exec.next() {
                return find_first_exec(&conf.path(), ex2, exec);
            } else {
                return Some(conf);
            }
        }
    }
    None
}
