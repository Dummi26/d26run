#![feature(setgroups)]
#![feature(fs_try_exists)]

use std::collections::HashMap;
use std::io::{Read, Write};
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
            std::thread::spawn(move || {
                let client_dir = format!("/tmp/d26run-client-{id}/");
                let mut stream = BufReader::new(stream);
                writeln!(stream.get_mut(), "{id}")?;
                let mut line = String::new();
                let mut vars = HashMap::new();
                loop {
                    stream.read_line(&mut line)?;
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
                        ("list-configs", _) => {
                            writeln!(
                                stream.get_mut(),
                                "listing configs; count: {}",
                                config.run_cmds.len()
                            )?;
                            for (name, cfg) in config.run_cmds.iter() {
                                writeln!(stream.get_mut(), "{name}")?;
                                writeln!(
                                    stream.get_mut(),
                                    "{}",
                                    match &cfg.allow {
                                        Some(v) => v,
                                        None => "",
                                    }
                                )?;
                            }
                        }
                        ("reload-configs", _) => {
                            please_reload.store(true, std::sync::atomic::Ordering::Relaxed);
                            writeln!(stream.get_mut(), "reload-configs requested")?;
                        }
                        ("set-var", right) => {
                            if let Some((varname, value)) = right.split_once(' ') {
                                vars.insert(varname.to_owned(), value.to_owned());
                            }
                        }
                        ("run", runcfg) => 'run: {
                            let mut forward_output = false;
                            let mut detach = false;
                            let mut forward_input = false;
                            let runcfg = if let Some((args, runcfg)) = runcfg.split_once(' ') {
                                for arg in args.split(',') {
                                    let (arg, val) = if let Some((arg, val)) = arg.split_once('=') {
                                        (arg, Some(val))
                                    } else {
                                        (arg, None)
                                    };
                                    match arg {
                                        "mode" => {
                                            if let Some(val) = val {
                                                (detach, forward_output, forward_input) = match val {
                                                    "detach" => (true, false, false),
                                                    "wait" => (false, false, false),
                                                    "forward-output" => (false, true, false),
                                                    "forward-output-input" => (false, true, true),
                                                    _ => break 'run writeln!(
                                                        stream.get_mut(),
                                                        "run error_arg_value_invalid {arg} {val} // try detach, wait, forward-output or forward-output-input. wait is default."
                                                    )?,
                                                }
                                            } else {
                                                break 'run writeln!(
                                                    stream.get_mut(),
                                                    "run error_arg_no_value {arg}"
                                                )?;
                                            }
                                        }
                                        _ => {
                                            break 'run writeln!(
                                                stream.get_mut(),
                                                "run error_invalid_arg {arg}"
                                            )?
                                        }
                                    }
                                }
                                runcfg
                            } else {
                                runcfg
                            };
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
                                                    let info = ToRunCmdInfo { con_id: id };
                                                    match cfg.to_runcmd(&vars, &info) {
                                                        Ok(runcmd) => {
                                                            writeln!(
                                                                stream.get_mut(),
                                                                "run start"
                                                            )?;
                                                            stream.get_mut().flush()?;
                                                            let mut r = Runner::new(runcmd);
                                                            r.start();
                                                            if detach {
                                                                std::thread::spawn(move || {
                                                                    r.wait()
                                                                });
                                                            } else {
                                                                if let Some(child) =
                                                                    &mut r.child_process
                                                                {
                                                                    if !forward_output {
                                                                        r.wait();
                                                                    } else {
                                                                        /// If the thread returns Ok(()), the returned receiver was dropped or the reader reached EOF.
                                                                        fn thread_get_stdout<
                                                                            S: Read + Send + 'static,
                                                                        >(
                                                                            s: S,
                                                                        ) -> (
                                                                            std::thread::JoinHandle<
                                                                                Result<
                                                                                    (),
                                                                                    std::io::Error,
                                                                                >,
                                                                            >,
                                                                            mpsc::Receiver<u8>,
                                                                        )
                                                                        {
                                                                            let (so, out) =
                                                                                mpsc::channel();
                                                                            (
                                                                                std::thread::spawn(
                                                                                    move || {
                                                                                        let mut s =
                                                                                    BufReader::new(
                                                                                        s,
                                                                                    );
                                                                                        let mut b =
                                                                                            [0u8];
                                                                                        loop {
                                                                                            if s.read(
                                                                                            &mut b,
                                                                                        )? == 0
                                                                                        {
                                                                                            break;
                                                                                        };
                                                                                            if so
                                                                                        .send(b[0])
                                                                                        .is_err()
                                                                                    {
                                                                                        break;
                                                                                    }
                                                                                        }
                                                                                        Ok(())
                                                                                    },
                                                                                ),
                                                                                out,
                                                                            )
                                                                        }
                                                                        let mut stdout = child
                                                                            .stdout
                                                                            .take()
                                                                            .map(|s| {
                                                                                thread_get_stdout(s)
                                                                            });
                                                                        let mut stderr = child
                                                                            .stderr
                                                                            .take()
                                                                            .map(|s| {
                                                                                thread_get_stdout(s)
                                                                            });
                                                                        if forward_input {
                                                                            stream
                                                                            .get_mut()
                                                                            .set_read_timeout(Some(
                                                                            Duration::from_secs_f32(
                                                                                0.1,
                                                                            ),
                                                                        ))?;
                                                                        }
                                                                        loop {
                                                                            let mut sent_anything =
                                                                                false;
                                                                            let stdout_finished =
                                                                                if let Some((
                                                                                    t,
                                                                                    r,
                                                                                )) = &mut stdout
                                                                                {
                                                                                    let fin =
                                                                                    t.is_finished();
                                                                                    let mut buf =
                                                                                        Vec::new();
                                                                                    while let Ok(
                                                                                        r,
                                                                                    ) =
                                                                                        r.try_recv()
                                                                                    {
                                                                                        buf.push(r);
                                                                                        if buf.len()
                                                                                            >= 120
                                                                                        {
                                                                                            break;
                                                                                        }
                                                                                    }
                                                                                    if buf.len() > 1
                                                                                    {
                                                                                        let b = buf
                                                                                            .len()
                                                                                            as u8;
                                                                                        stream
                                                                                        .get_mut()
                                                                                        .write_all(
                                                                                            &[b],
                                                                                        )?;
                                                                                        stream
                                                                                        .get_mut()
                                                                                        .write_all(
                                                                                            &buf,
                                                                                        )?;
                                                                                        sent_anything =
                                                                                        true;
                                                                                    }
                                                                                    fin
                                                                                } else {
                                                                                    true
                                                                                };
                                                                            let stderr_finished =
                                                                                if let Some((
                                                                                    t,
                                                                                    r,
                                                                                )) = &mut stderr
                                                                                {
                                                                                    let fin =
                                                                                    t.is_finished();
                                                                                    let mut buf =
                                                                                        Vec::new();
                                                                                    while let Ok(
                                                                                        r,
                                                                                    ) =
                                                                                        r.try_recv()
                                                                                    {
                                                                                        buf.push(r);
                                                                                        if buf.len()
                                                                                            >= 120
                                                                                        {
                                                                                            break;
                                                                                        }
                                                                                    }
                                                                                    if buf.len() > 1
                                                                                    {
                                                                                        // 1st bit = 1 => stderr
                                                                                        let b = 128
                                                                                        | buf.len()
                                                                                            as u8;
                                                                                        stream
                                                                                        .get_mut()
                                                                                        .write_all(
                                                                                            &[b],
                                                                                        )?;
                                                                                        stream
                                                                                        .get_mut()
                                                                                        .write_all(
                                                                                            &buf,
                                                                                        )?;
                                                                                        sent_anything =
                                                                                        true;
                                                                                    }
                                                                                    fin
                                                                                } else {
                                                                                    true
                                                                                };
                                                                            if forward_input {
                                                                                if let Some(stdin) =
                                                                                    &mut child.stdin
                                                                                {
                                                                                    let mut w =
                                                                                        false;
                                                                                    loop {
                                                                                        let mut b =
                                                                                            [0];
                                                                                        if let Ok(1) = stream
                                                                                            .read(
                                                                                            &mut b,
                                                                                        )
                                                                                        {
                                                                                            _ = stdin.write_all(&b);
                                                                                            w = true;
                                                                                        } else {
                                                                                            break;
                                                                                        }
                                                                                    }
                                                                                    if w {
                                                                                        eprintln!("wrote some bytes to child's stdin.");
                                                                                        _ = stdin
                                                                                            .flush(
                                                                                            );
                                                                                    }
                                                                                }
                                                                            }
                                                                            if sent_anything {
                                                                                stream
                                                                                    .get_mut()
                                                                                    .flush()?;
                                                                            }
                                                                            if stdout_finished
                                                                                && stderr_finished
                                                                                && child
                                                                                    .try_wait()
                                                                                    .is_ok()
                                                                                && sent_anything
                                                                                    == false
                                                                            {
                                                                                r.wait();
                                                                                break;
                                                                            }
                                                                        }
                                                                    }
                                                                }
                                                            }
                                                            stream.get_mut().write(&[0])?;
                                                            stream.get_mut().flush()?;
                                                        }
                                                        Err(err) => {
                                                            writeln!(
                                                                stream.get_mut(),
                                                                "run error_invalid_config: {}",
                                                                err.len()
                                                            )?;
                                                            for err in err {
                                                                let text = err.to_string();
                                                                let lines: Vec<_> =
                                                                    text.lines().collect();
                                                                writeln!(
                                                                    stream.get_mut(),
                                                                    "{}",
                                                                    lines.len()
                                                                )?;
                                                                for line in lines {
                                                                    writeln!(
                                                                        stream.get_mut(),
                                                                        "{}",
                                                                        line
                                                                    )?;
                                                                }
                                                            }
                                                        }
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
                                            writeln!(
                                                stream.get_mut(),
                                                "unexpected_response auth done"
                                            )?;
                                        }
                                    } else {
                                        writeln!(
                                            stream.get_mut(),
                                            "auth fail could_not_copy_auth_file"
                                        )?;
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
                eprintln!("[INFO] disconnected [{id}].");
                Ok::<_, std::io::Error>(id)
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
