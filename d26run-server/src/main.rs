#![feature(setgroups)]
#![feature(fs_try_exists)]

use std::collections::HashMap;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::{
    fs,
    io::{BufRead, BufReader},
};

use crate::run::Runner;

mod config;
mod run;

const DIR_CONFIGS: &'static str = "/etc/d26run/configs/";
const DIR_ALLOWS: &'static str = "/etc/d26run/allow/";

fn main() {
    let min_duration_between_reloads = Duration::from_secs(15);
    // remove previous socket and client directories
    if let Ok(dir) = fs::read_dir("/tmp/") {
        for entry in dir {
            if let Ok(entry) = entry {
                if let Ok(file_type) = entry.file_type() {
                    if file_type.is_dir() {
                        if let Some(name) = entry.file_name().to_str() {
                            if name.starts_with("d26run-client-") {
                                fs::remove_dir_all(entry.path())
                                    .expect("couldn't remove previous d26run-client-* directory.");
                            }
                        }
                    }
                }
            }
        }
    }
    if let Ok(true) = fs::try_exists("/tmp/d26run-socket") {
        fs::remove_file("/tmp/d26run-socket").unwrap();
    }
    // create the prep_dir with read/write access and no access for anyone else.
    let prep_dir = "/tmp/d26run-prep/";
    fs::create_dir_all(prep_dir).unwrap();
    let mut perms = fs::metadata(prep_dir).unwrap().permissions();
    perms.set_mode(0o600);
    fs::set_permissions(prep_dir, perms).unwrap();
    // open the socket and chmod it
    let listener = std::os::unix::net::UnixListener::bind("/tmp/d26run-socket").unwrap();
    let mut socket_permissions = fs::metadata("/tmp/d26run-socket").unwrap().permissions();
    socket_permissions.set_mode(0o666);
    fs::set_permissions("/tmp/d26run-socket", socket_permissions).unwrap();
    // accept connections
    let mut current_id = 0;
    let mut config = Arc::new(config::init());
    let mut last_reload = Instant::now();
    let please_reload = Arc::new(AtomicBool::new(false));
    loop {
        if let Ok((stream, _addr)) = listener.accept() {
            // update stuff
            if please_reload.load(std::sync::atomic::Ordering::Relaxed)
                && last_reload.elapsed() > min_duration_between_reloads
            {
                config = Arc::new(config::init());
                last_reload = Instant::now();
            }
            // start task
            let id = current_id;
            current_id += 1;
            let config = Arc::clone(&config);
            let please_reload = Arc::clone(&please_reload);
            std::thread::spawn(move || {
                let client_dir = format!("/tmp/d26run-client-{id}/");
                let mut stream = BufReader::new(stream);
                writeln!(stream.get_mut(), "{id}")?;
                let mut line = String::new();
                let mut vars = HashMap::new();
                loop {
                    stream.read_line(&mut line).unwrap();
                    if line.is_empty() {
                        break;
                    }
                    let (command, args) = if let Some((command, args)) = line.split_once(' ') {
                        (command, args.to_owned())
                    } else {
                        (line.as_str(), String::new())
                    };
                    match (command.trim(), args.trim_end_newline()) {
                        // ("open", file) => {}
                        ("reload-configs", _) => {
                            please_reload.store(true, std::sync::atomic::Ordering::Relaxed);
                            writeln!(stream.get_mut(), "reload-configs requested")?;
                        }
                        ("set-var", right) => {
                            if let Some((varname, value)) = right.split_once(' ') {
                                vars.insert(varname.to_owned(), value.to_owned());
                            }
                        }
                        ("run", runcfg) => {
                            if let Some(cfg) = config.run_cmds.get(runcfg) {
                                if let Some(allow) = &cfg.allow {
                                    let auth_file = format!("{client_dir}auth");
                                    let allow_src = format!("{DIR_ALLOWS}{}", allow);
                                    if let Ok(_) = fs::copy(allow_src, &auth_file) {
                                        writeln!(stream.get_mut(), "auth wait")?;
                                        if stream.line().as_str() == "auth done" {
                                            match fs::try_exists(&auth_file) {
                                                Ok(false) => {
                                                    writeln!(stream.get_mut(), "auth accept")?;
                                                    match cfg.to_runcmd_with_vars(&vars) {
                                                        Some(runcmd) => Runner::new(runcmd).start(),
                                                        None => writeln!(
                                                            stream.get_mut(),
                                                            "run error_invalid_config"
                                                        )?,
                                                    }
                                                }
                                                Ok(true) => {
                                                    writeln!(stream.get_mut(), "auth deny exists")?
                                                }
                                                Err(e) => writeln!(
                                                    stream.get_mut(),
                                                    "auth deny error {}",
                                                    e.to_string().replace('\n', "\\n")
                                                )?,
                                            }
                                        } else {
                                            break;
                                        }
                                    } else {
                                        writeln!(stream.get_mut(), "auth fail")?;
                                    }
                                } else {
                                    writeln!(stream.get_mut(), "auth deny error_undefined_allow")?;
                                }
                            } else {
                                writeln!(stream.get_mut(), "run unknown")?;
                            }
                            vars.clear();
                        }
                        _ => (),
                    }
                    line.clear();
                }
                eprintln!("disconnected [{id}].");
                Ok::<u32, std::io::Error>(id)
            });
        }
    }
}

trait NewlineRemover {
    fn trim_end_newline(&self) -> &str;
}
impl NewlineRemover for str {
    fn trim_end_newline(&self) -> &str {
        if self.ends_with('\n') {
            &self[0..self.len() - 1]
        } else {
            self
        }
    }
}
trait Liner {
    fn line(&mut self) -> String;
}
impl Liner for BufReader<UnixStream> {
    fn line(&mut self) -> String {
        let mut buf = String::new();
        _ = self.read_line(&mut buf);
        if buf.ends_with('\n') {
            buf.pop();
        }
        buf
    }
}
