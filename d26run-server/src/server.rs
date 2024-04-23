use std::{
    collections::HashMap,
    fs,
    io::{BufRead, BufReader, Read, Write},
    os::{linux::fs::MetadataExt, unix::net::UnixStream},
    sync::{atomic::AtomicBool, mpsc, Arc},
    time::Duration,
};

use crate::{
    config::Config,
    run::{Runner, ToRunCmdInfo},
    Liner, NewlineRemover, DIR_ALLOWS,
};

pub fn handle_con(
    stream: UnixStream,
    id: u128,
    config: Arc<Config>,
    please_reload: Arc<AtomicBool>,
) {
    match handle_con_internal(stream, id, config, please_reload) {
        Ok(()) => {
            eprintln!("[INFO] disconnected [{id}]. (dc)");
        }
        Err(_) => {
            eprintln!("[INFO] disconnected [{id}] (error).");
        }
    }
}
fn handle_con_internal(
    stream: UnixStream,
    id: u128,
    config: Arc<Config>,
    please_reload: Arc<AtomicBool>,
) -> Result<(), std::io::Error> {
    let mut stream = BufReader::new(stream);
    writeln!(stream.get_mut(), "{id}")?;
    let mut line = String::new();
    let mut vars = HashMap::new();
    let mut auth_id = 0;
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
                        let allow_src = format!("{DIR_ALLOWS}{allow}");
                        auth_id += 1;
                        let auth_file = format!("/tmp/d26run-auth-{id}-{auth_id}");
                        let ok = || {
                            // auth
                            let auth_file_meta = fs::metadata(&allow_src)?;
                            fs::File::create(&auth_file)?;
                            fs::set_permissions(&auth_file, auth_file_meta.permissions())?;
                            std::os::unix::fs::chown(
                                &auth_file,
                                Some(auth_file_meta.st_uid()),
                                Some(auth_file_meta.st_gid()),
                            )?;
                            std::io::Result::Ok(())
                        };
                        let init_client_dir = ok();
                        if init_client_dir.is_ok() {
                            writeln!(stream.get_mut(), "auth wait {auth_file}")?;
                            if stream.line().as_str() == "auth done" {
                                match fs::File::open(&auth_file) {
                                    Ok(file) => {
                                        if file
                                            .bytes()
                                            .take(5)
                                            .collect::<Result<Vec<_>, _>>()
                                            .ok()
                                            .and_then(|b| String::from_utf8(b).ok())
                                            .is_some_and(|v| v.trim() == "auth")
                                        {
                                            writeln!(stream.get_mut(), "auth accept")?;
                                            let info = ToRunCmdInfo { con_id: id };
                                            match cfg.to_runcmd(&vars, &info) {
                                                Ok(runcmd) => {
                                                    writeln!(stream.get_mut(), "run start")?;
                                                    stream.get_mut().flush()?;
                                                    let mut r = Runner::new(runcmd);
                                                    r.start();
                                                    if detach {
                                                        std::thread::spawn(move || r.wait());
                                                    } else {
                                                        if let Some(child) = &mut r.child_process {
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
                                                                        Result<(), std::io::Error>,
                                                                    >,
                                                                    mpsc::Receiver<u8>,
                                                                )
                                                                {
                                                                    let (so, out) = mpsc::channel();
                                                                    (
                                                                        std::thread::spawn(
                                                                            move || {
                                                                                let mut s =
                                                                                    BufReader::new(
                                                                                        s,
                                                                                    );
                                                                                let mut b = [0u8];
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
                                                                    .map(|s| thread_get_stdout(s));
                                                                let mut stderr = child
                                                                    .stderr
                                                                    .take()
                                                                    .map(|s| thread_get_stdout(s));
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
                                                                    let mut sent_anything = false;
                                                                    let stdout_finished =
                                                                        if let Some((t, r)) =
                                                                            &mut stdout
                                                                        {
                                                                            let fin =
                                                                                t.is_finished();
                                                                            let mut buf =
                                                                                Vec::new();
                                                                            while let Ok(r) =
                                                                                r.try_recv()
                                                                            {
                                                                                buf.push(r);
                                                                                if buf.len() >= 120
                                                                                {
                                                                                    break;
                                                                                }
                                                                            }
                                                                            if buf.len() > 1 {
                                                                                let b =
                                                                                    buf.len() as u8;
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
                                                                        if let Some((t, r)) =
                                                                            &mut stderr
                                                                        {
                                                                            let fin =
                                                                                t.is_finished();
                                                                            let mut buf =
                                                                                Vec::new();
                                                                            while let Ok(r) =
                                                                                r.try_recv()
                                                                            {
                                                                                buf.push(r);
                                                                                if buf.len() >= 120
                                                                                {
                                                                                    break;
                                                                                }
                                                                            }
                                                                            if buf.len() > 1 {
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
                                                                            let mut w = false;
                                                                            loop {
                                                                                let mut b = [0];
                                                                                if let Ok(1) =
                                                                                    stream.read(
                                                                                        &mut b,
                                                                                    )
                                                                                {
                                                                                    _ = stdin
                                                                                        .write_all(
                                                                                            &b,
                                                                                        );
                                                                                    w = true;
                                                                                } else {
                                                                                    break;
                                                                                }
                                                                            }
                                                                            if w {
                                                                                eprintln!("wrote some bytes to child's stdin.");
                                                                                _ = stdin.flush();
                                                                            }
                                                                        }
                                                                    }
                                                                    if sent_anything {
                                                                        stream.get_mut().flush()?;
                                                                    }
                                                                    if stdout_finished
                                                                        && stderr_finished
                                                                        && child.try_wait().is_ok()
                                                                        && sent_anything == false
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
                                                        let lines: Vec<_> = text.lines().collect();
                                                        writeln!(
                                                            stream.get_mut(),
                                                            "{}",
                                                            lines.len()
                                                        )?;
                                                        for line in lines {
                                                            writeln!(stream.get_mut(), "{}", line)?;
                                                        }
                                                    }
                                                }
                                            }
                                        } else {
                                            writeln!(stream.get_mut(), "auth deny failed")?
                                        }
                                    }
                                    Err(e) => writeln!(
                                        stream.get_mut(),
                                        "auth deny error {}",
                                        e.to_string().replace('\n', "\\n")
                                    )?,
                                }
                            } else {
                                writeln!(stream.get_mut(), "unexpected_response auth done")?;
                            }
                        } else {
                            eprintln!("could_not_copy_auth_file: {:?}", init_client_dir);
                            writeln!(stream.get_mut(), "auth fail could_not_copy_auth_file")?;
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
    Ok::<_, std::io::Error>(())
}
