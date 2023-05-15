use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    os::unix::net::UnixStream,
    path::Path,
};

fn main() {
    let mut socket = "/tmp/d26run-socket".to_owned();
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
            "run" => Con::init(socket).run(
                args.get(1)
                    .expect("run requires a second argument")
                    .as_str(),
                args.iter().skip(2).filter_map(|v| v.split_once('=')),
            ),
            "reload" => Con::init(socket).reload_configs(),
            _ => eprintln!("unknown command, run without arguments for tldr."),
        }
    } else {
        eprintln!(
            "d26run-client tldr:
    run <name> => run a config if permissions are sufficient (/etc/d26run/configs/<name>)
    reload => request that the server reloads all configurations. might not have an effect immedeately (rate limit)
"
        )
    }
}

pub struct Con {
    stream: BufReader<UnixStream>,
    id: usize,
    client_dir: String,
}
impl Con {
    pub fn init<P: AsRef<Path>>(addr: P) -> Self {
        let stream = std::os::unix::net::UnixStream::connect(addr).unwrap();
        let mut o = Self {
            stream: BufReader::new(stream),
            id: 0,
            client_dir: String::new(),
        };
        o.init_();
        o
    }
    fn init_(&mut self) {
        self.id = self.read_line().parse().unwrap();
        self.client_dir = format!("/tmp/d26run-client-{}/", self.id);
        eprintln!("{} -> {}", self.id, self.client_dir);
        fs::create_dir(&self.client_dir).unwrap();
    }
    /// write
    fn w(&mut self) -> &mut UnixStream {
        self.stream.get_mut()
    }
    fn read_line(&mut self) -> String {
        let mut buf = String::new();
        self.stream.read_line(&mut buf).unwrap();
        if buf.ends_with('\n') {
            buf.pop();
        }
        buf
    }
    pub fn reload_configs(&mut self) {
        // ask to reload
        writeln!(self.w(), "reload-configs").unwrap();
        assert_eq!("reload-configs requested", self.read_line().as_str());
    }
    pub fn run<'a, V>(&mut self, config: &'a str, vars: V)
    where
        V: Iterator<Item = (&'a str, &'a str)>,
    {
        // set vars
        for (var_name, var_value) in vars {
            if !var_name.contains(' ') {
                writeln!(self.w(), "set-var {var_name} {var_value}").unwrap();
            }
        }
        // ask to run
        writeln!(self.w(), "run {config}").unwrap();
        // wait until auth is ready
        assert_eq!("auth wait", self.read_line().as_str());
        // authenticate (via file permissions)
        fs::remove_file(format!("{}auth", self.client_dir).as_str()).unwrap();
        writeln!(self.w(), "auth done").unwrap();
        // wait for confirmation
        assert_eq!("auth accept", self.read_line().as_str());
        // confirm that it was started
        assert_eq!("run start", self.read_line().as_str());
        // forward stdout/stderr
        let mut what_byte = [0u8];
        loop {
            self.stream.read_exact(&mut what_byte).unwrap();
            let what_byte = what_byte[0];
            if what_byte == 0 {
                break;
            }
            let stderr = what_byte & 128 != 0;
            let len = what_byte & 127;
            let mut buf = vec![0u8; len as _];
            self.stream.read_exact(&mut buf[..]).unwrap();
            if stderr {
                std::io::stderr().write(&buf[..]).unwrap();
            } else {
                std::io::stdout().write(&buf[..]).unwrap();
            }
        }
    }
}
impl Drop for Con {
    fn drop(&mut self) {
        if let Err(e) = fs::remove_dir_all(&self.client_dir) {
            eprintln!("[WARN] Error removing d26run-client-dir - this isn't problematic, but might clutter your /tmp/:\n    {e:?}");
        }
    }
}
