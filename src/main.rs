use std::fs::{create_dir_all as mkdir, remove_dir_all as rmdir};
use std::path::Path;
use std::process::{Command, Stdio};

mod conf;
mod script;

use crate::script::VarValue;
use conf::Conf;

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
    let mut conf = Conf::new();
    conf.config_script_variables
        .insert("args".to_string(), VarValue::List(vec![]));

    let (configs, args) = if let Some(v) = conf.parse_args() {
        v
    } else {
        return;
    };

    println!("[c] Configs: {:?}", &configs);

    for config in &configs {
        conf.load(config);
    }

    match &conf.home_dir {
        Some(v) if v.is_empty() => conf.home_dir = None,
        _ => (),
    }
    match &conf.immutable_home_at.as_ref() {
        Some(v) if v.is_empty() => conf.immutable_home_at = None,
        _ => (),
    }
    match &conf.passwd.as_ref() {
        Some(v) if v.is_empty() => conf.passwd = None,
        _ => (),
    }

    let name = match &conf.name {
        Some(n) => n.to_owned(),
        None => String::new(),
    };
    let count = conf.count;

    let username = conf.get_username();
    let home_dir = match &conf.home_dir {
        Some(v) => v.to_owned(),
        None => format!("/tmp/dummi26_run/{username}/home"),
    };

    conf.apply_to_all_strings();

    let test = conf.test;

    println!("[C] Count: {count}");
    println!("[n] Name: {name}");
    if let Some(passwd) = &conf.passwd {
        println!("[p] Passwd: {passwd}");
    }
    println!("Username: {username}");
    println!("Home dir: {home_dir}");
    if let Some(v) = &conf.group {
        println!("Group:     {v}");
    }
    if let Some(v) = &conf.groups {
        println!("Groups:    {v}");
    }
    println!();

    if test {
        println!("----- test mode -----");
    }

    println!("Removing previous user:");
    if test {
        println!("Removing user '{}'", username.as_str());
    } else {
        Command::new("userdel")
            .arg(username.as_str())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .output()
            .unwrap();
    }

    if conf.userhomedel || conf.immutable_home_at.is_some() {
        if test {
            println!(
                "Removing home: '{:?}'",
                std::path::PathBuf::from(home_dir.as_str())
            );
        } else {
            println!(
                "Removing home: {:?}",
                rmdir(std::path::PathBuf::from(home_dir.as_str()))
            );
        }
    }

    println!("Adding new user:");
    if test {
        println!(" + '{}'", username);
    } else {
        let mut cmd = Command::new("useradd");
        if let Some(passwd) = &conf.passwd {
            cmd.args(["-p", passwd]);
        }
        if let Some(g) = &conf.group {
            cmd.args(["-g", g]);
        }
        if let Some(g) = &conf.groups {
            cmd.args(["-G", g]);
        }
        cmd.args([
            "--home-dir",
            home_dir.as_str(),
            // "--no-user-group",
            "--create-home",
            username.as_str(),
        ]);
        println!("Useradd command: {cmd:?}");
        cmd.stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .unwrap();
    }

    if let Some(immuthome) = conf.immutable_home_at {
        println!(
            "Copying existing home from '{immuthome} to '{home_dir}'... (setup immutable home)"
        );
        if test {
            println!("Done. (immutable home in test mode)");
        } else {
            for error in copy_dir_recursive_ignore_errors(&immuthome, &home_dir) {
                println!(" E: {error:?}");
            }
            println!("Done. (immutable home created)");
        }
    } else {
        if test {
            println!(
                "creating home dir if it doesn't exist already @ '{}'",
                home_dir
            );
        } else {
            mkdir(home_dir.as_str()).unwrap();
        }
    }

    if test {
        println!(
            "chown home dir: chown -R '{}' '{}'",
            username.as_str(),
            home_dir.as_str()
        );
    } else {
        println!(
            "chown home dir: {:?}",
            Command::new("chown")
                .args(["-R", username.as_str(), home_dir.as_str()])
                .status()
                .unwrap()
        );
    }

    println!("Running init commands from config, if any...");
    for (fatal, cmd) in conf.init {
        if let Some(command) = cmd.first() {
            let mut out = Command::new(&command);
            if cmd.len() > 1 {
                out.args(&cmd[1..]);
            }
            if test {
                println!("Command {out:?}");
            } else {
                let status = out.status();
                println!("Command {out:?}: {status:?}");
                if fatal && !status.unwrap().success() {
                    panic!("Cancelling because a fatal init command was unsuccessful.")
                };
            }
        }
    }

    if test {
        println!("Setup complete.");
    } else {
        println!("Setup complete, running command now...\n\n");
        println!(
            "\n\n[EXIT STATUS: {:?}]",
            Command::new("doas")
                .args(
                    ["-u".to_string(), username.clone(), "--".to_string()]
                        .into_iter()
                        .chain(conf.run.into_iter())
                        .chain(args)
                )
                .status()
        );
    }

    if conf.userdel {
        if test {
            println!("Removing user: '{}'", username);
        } else {
            println!(
                "Removing user: {:?}",
                Command::new("userdel")
                    .arg(username.as_str())
                    .stdout(Stdio::inherit())
                    .stderr(Stdio::inherit())
                    .status()
                    .unwrap()
            );
        }
    }
    if conf.userhomedel {
        if test {
            println!("Removing user home '{}'", home_dir);
        } else {
            println!("Removing user home: {:?}", rmdir(home_dir));
        }
    }
}

fn copy_dir_recursive_ignore_errors<P1, P2>(
    dir: P1,
    target: P2,
) -> Vec<(std::ffi::OsString, std::io::Error)>
where
    P1: AsRef<Path>,
    P2: AsRef<Path>,
{
    Command::new("cp")
        .arg("-r")
        .arg(dir.as_ref().as_os_str())
        .arg(target.as_ref().as_os_str())
        .status()
        .unwrap();
    vec![] // TODO!
}
