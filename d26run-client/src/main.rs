use std::{
    eprintln,
    fmt::Display,
    fs,
    io::{BufRead, BufReader, Read, Write},
    os::unix::net::UnixStream,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

fn main() {
    let mut socket = "/tmp/d26run-socket".to_owned();
    let mut mode = None;
    let args: Vec<_> = {
        let mut args = std::env::args().skip(1);
        loop {
            if let Some(arg) = args.next() {
                match arg.as_str() {
                    "--socket" => {
                        socket = args
                            .next()
                            .expect("--socket-path must be followed by another argument")
                    }
                    "--mode" => {
                        mode = Some(
                            match args
                                .next()
                                .expect("--mode must be followed by a mode")
                                .to_lowercase()
                                .as_str()
                            {
                                "wait" => RunMode::Wait,
                                "detach" => RunMode::Detach,
                                "output" => RunMode::ForwardOutput,
                                "interactive" => RunMode::ForwardInputOutput,
                                _ => panic!("--mode must be followed by wait, detach, or output."),
                            },
                        )
                    }
                    other if other.starts_with("-") => {
                        eprintln!("Unknown argument '{other}'");
                        std::process::exit(4);
                    }
                    _ => break [arg].into_iter().chain(args).collect(),
                }
            } else {
                break vec![];
            }
        }
    };
    if let Some(cmd) = args.get(0) {
        let cmd = cmd.as_str();
        match cmd {
            "run" => Con::init(socket)
                .unwrap()
                .run(
                    args.get(1)
                        .expect("run requires a second argument")
                        .as_str(),
                    args.iter().skip(2).filter_map(|v| v.split_once('=')),
                    mode,
                )
                .unwrap(),
            "reload" => Con::init(socket).unwrap().reload_configs(),
            "list" => {
                let cfgs = Con::init(socket).unwrap().list();
                println!("configs: {}", cfgs.len());
                for cfg in cfgs {
                    println!("{} ({})", cfg.0, cfg.1);
                }
            }
            _ => eprintln!("unknown command, run without arguments for tldr."),
        }
    } else {
        eprintln!(
            "d26run-client tldr:
    run <name> => run a config if permissions are sufficient (/etc/d26run/configs/<name>)
    reload => request that the server reloads all configurations. might not have an effect immedeately (rate limit)
    list => lists all available configs that can be used with 'run'.
"
        )
    }
}

pub enum RunMode {
    Detach,
    Wait,
    ForwardOutput,
    ForwardInputOutput,
}

#[derive(Debug)]
pub enum ConInitErr {
    CouldNotConnectToSocket(std::io::Error),
}
#[derive(Debug)]
pub enum ConRunErr {
    FailedToEditFileForAuth(String, std::io::Error),
}

impl Display for RunMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Detach => write!(f, "detach"),
            Self::Wait => write!(f, "wait"),
            Self::ForwardOutput => write!(f, "forward-output"),
            Self::ForwardInputOutput => write!(f, "forward-output-input"),
        }
    }
}

pub struct Con {
    stream: Arc<Mutex<BufReader<UnixStream>>>,
    id: usize,
    client_dir: String,
}
impl Con {
    pub fn init<P: AsRef<Path>>(addr: P) -> Result<Self, ConInitErr> {
        let stream = match std::os::unix::net::UnixStream::connect(addr) {
            Ok(v) => v,
            Err(e) => return Err(ConInitErr::CouldNotConnectToSocket(e)),
        };
        let mut o = Self {
            stream: Arc::new(Mutex::new(BufReader::new(stream))),
            id: 0,
            client_dir: String::new(),
        };
        o.init_();
        Ok(o)
    }
    fn init_(&mut self) {
        self.id = self.read_line().parse().unwrap();
        self.client_dir = format!("/tmp/d26run-client-{}/", self.id);
        eprintln!("{} -> {}", self.id, self.client_dir);
        // fs::create_dir(&self.client_dir).expect("failed: can't create client dir.");
    }
    /// write
    fn w(&self) -> std::sync::MutexGuard<BufReader<UnixStream>> {
        let o = self.stream.lock().unwrap();
        o
    }
    fn read_line(&mut self) -> String {
        let mut buf = String::new();
        self.w().read_line(&mut buf).unwrap();
        if buf.ends_with('\n') {
            buf.pop();
        }
        buf
    }
    pub fn list(&mut self) -> Vec<(String, String)> {
        writeln!(self.w().get_mut(), "list-configs").unwrap();
        let response = self.read_line();
        assert!(response.starts_with("listing configs; count: "));
        let count = response["listing configs; count: ".len()..]
            .trim()
            .parse()
            .expect(
                "failed: list-configs: server returned count that couldn't be parsed to an int...",
            );
        let mut o = Vec::with_capacity(count);
        for _ in 0..count {
            o.push((self.read_line(), self.read_line()));
        }
        o
    }
    pub fn reload_configs(&mut self) {
        // ask to reload
        writeln!(self.w().get_mut(), "reload-configs").unwrap();
        assert_eq!("reload-configs requested", self.read_line().as_str());
    }
    pub fn run<'a, V>(
        &mut self,
        config: &'a str,
        vars: V,
        mode: Option<RunMode>,
    ) -> Result<(), ConRunErr>
    where
        V: Iterator<Item = (&'a str, &'a str)>,
    {
        // set vars
        for (var_name, var_value) in vars {
            if !var_name.contains(' ') {
                writeln!(self.w().get_mut(), "set-var {var_name} {var_value}").unwrap();
            }
        }
        // ask to run
        if let Some(mode) = &mode {
            writeln!(self.w().get_mut(), "run mode={mode} {config}").unwrap();
        } else {
            writeln!(self.w().get_mut(), "run {config}").unwrap();
        }
        // wait until auth is ready
        let auth_line_start = "auth wait ";
        let auth_line = self.read_line();
        let auth_line = auth_line.as_str();
        if !auth_line.starts_with(auth_line_start) {
            panic!("expected 'auth wait <file>', got '{auth_line}'");
        }
        let path = &auth_line[auth_line_start.len()..];
        // authenticate (via file permissions)
        match fs::write(path, "auth") {
            Ok(_) => (),
            Err(e) => return Err(ConRunErr::FailedToEditFileForAuth(path.to_owned(), e)),
        };
        writeln!(self.w().get_mut(), "auth done").unwrap();
        // wait for confirmation
        assert_eq!("auth accept", self.read_line().as_str());
        // confirm that it was started
        match self.read_line().as_str() {
            "run start" => (),
            err => {
                if err.starts_with("run error_invalid_config: ") {
                    let err_count = err["run error_invalid_config: ".len()..]
                        .trim()
                        .parse()
                        .expect("failed: error_invalid_config: server returned count that couldn't be parsed to an int...");
                    for i in 0..err_count {
                        let err_len = self.read_line().parse().unwrap();
                        for _ in 0..err_len {
                            eprintln!("{} | {}", i + 1, self.read_line());
                        }
                    }
                    panic!("couldn't run - there were {} errors.", err_count);
                }
            }
        }
        // forward stdin
        let fwd_stdin = if let Some(RunMode::ForwardInputOutput) = &mode {
            true
        } else {
            false
        };
        std::thread::scope(move |s| {
            if fwd_stdin {
                s.spawn(|| {
                    let mut stdin = std::io::stdin().lock();
                    let mut buf = [0];
                    loop {
                        stdin.read_exact(&mut buf).unwrap();
                        eprintln!("sending byte {}", buf[0]);
                        self.w().get_mut().write_all(&buf).unwrap();
                    }
                });
            }
            // forward stdout/stderr
            let mut what_byte = [0u8];
            self.w()
                .get_mut()
                .set_read_timeout(Some(Duration::from_secs_f32(0.05)))
                .unwrap();
            loop {
                if fwd_stdin {
                    std::thread::sleep(Duration::from_secs_f32(0.1));
                }
                {
                    if let Ok(1) = self.w().read(&mut what_byte) {
                    } else {
                        continue;
                    }
                }
                let what_byte = what_byte[0];
                if what_byte == 0 {
                    break;
                }
                let stderr = what_byte & 128 != 0;
                let len = what_byte & 127;
                let mut buf = vec![0u8; len as _];
                {
                    self.w().read_exact(&mut buf[..]).unwrap();
                }
                if stderr {
                    std::io::stderr().write(&buf[..]).unwrap();
                } else {
                    std::io::stdout().write(&buf[..]).unwrap();
                }
            }
            self.w()
                .get_mut()
                .shutdown(std::net::Shutdown::Both)
                .unwrap();
        });
        Ok(())
    }
}
