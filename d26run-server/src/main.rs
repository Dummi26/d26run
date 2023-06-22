#![feature(setgroups)]
#![feature(fs_try_exists)]
#![feature(unix_chown)]

use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::linux::fs::MetadataExt;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::sync::atomic::AtomicBool;
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};
use std::{
    fs,
    io::{BufRead, BufReader},
};

use crate::run::{Runner, ToRunCmdInfo};

mod config;
mod run;
mod server;

const DIR_CONFIGS: &'static str = "/etc/d26run/configs/";
const DIR_ALLOWS: &'static str = "/etc/d26run/allow/";

fn main() {
    let mut test_mode = false;
    let mut socket_path = "/tmp/d26run-socket".to_string();
    {
        let mut args = std::env::args().skip(1);
        loop {
            if let Some(arg) = args.next() {
                match arg.as_str() {
                    "--test-mode" => test_mode = true,
                    "--socket-path" => {
                        socket_path = args
                            .next()
                            .expect("--socket-path must be followed by another argument")
                    }
                    other /* if other.starts_with("-") */ => {
                        eprintln!("[ERR!] Unknown argument '{other}'");
                        std::process::exit(4);
                    }
                }
            } else {
                break;
            }
        }
    }
    if test_mode {
        eprintln!("[INFO] test-mode enabled!");
    }
    eprintln!("[INFO] socket_path: {socket_path}");
    let min_duration_between_reloads = Duration::from_secs(15);
    if !test_mode {
        // remove previous socket and client directories
        if let Ok(dir) = fs::read_dir("/tmp/") {
            for entry in dir {
                if let Ok(entry) = entry {
                    if let Ok(file_type) = entry.file_type() {
                        if file_type.is_dir() {
                            if let Some(name) = entry.file_name().to_str() {
                                if name.starts_with("d26run-client-") {
                                    fs::remove_dir_all(entry.path()).expect(
                                        "couldn't remove previous d26run-client-* directory.",
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    if let Ok(true) = fs::try_exists(&socket_path) {
        fs::remove_file(&socket_path).unwrap();
    }
    // open the socket and chmod it
    let listener = std::os::unix::net::UnixListener::bind(&socket_path).unwrap();
    let mut socket_permissions = fs::metadata(&socket_path).unwrap().permissions();
    socket_permissions.set_mode(0o666);
    fs::set_permissions(&socket_path, socket_permissions).unwrap();
    // accept connections
    let mut current_id = 0u128;
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
            std::thread::spawn(move || server::handle_con(stream, id, config, please_reload));
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
