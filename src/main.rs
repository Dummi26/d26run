use std::fs::{create_dir_all as mkdir, remove_dir_all as rmdir, read_to_string};
use std::process::{Command, Stdio};
use palaver::thread::gettid;

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

*/

fn main() {
    let mut args = std::env::args().into_iter();
    let mut config = None;
    let mut count: u64 = gettid();
    let mut name = None;
    let mut home_dir = None;
    let mut userdel = true;
    let mut userhomedel = false;
    loop {
        let arg = args.next();
        if let Some(arg) = arg {
            if arg == "--" { break; }
            match arg.chars().next() {
                Some('c') => { config = Some(arg[1..].to_string()); },
                Some('C') => { count = arg[1..].parse().expect("Syntax is C[count], where [count] is an integer!"); },
                Some('n') => { name = Some(arg[1..].to_string()); },
                Some('h') => { home_dir = Some(arg[1..].to_string()); },
                Some('-') => match match arg[1..].split_once('=') { Some(v) => (v.0, Some(v.1)), None => (&arg[1..], None) } {
                    ("nouserdel", _) => userdel = false,
                    ("userhomedel", _) => userhomedel = true,
                    _ => println!("Ignoring unknown argument '{arg}'."),
                },
                _ => (),
            }
        } else { break; }
    }

    println!("[c] Config: {}", match &config { None => "-".to_string(), Some(c) => format!("'{c}'") });

    let mut run = vec![];
    let mut init = vec![];
    if let Some(config) = &config {
        match read_to_string(config) {
            Ok(config) => {
                for line in config.lines() {
                    match (line.chars().next(), line.split_once(' ')) {
                        (Some('#'), _) | (_, None) => (), // comment, do nothing
                        (_, Some((what, args))) => { // do something
                            match what {
                                // run [something] adds [something] to the args for doas. The first one is the program to be executed, anything following that are args for that program: 'doas -u [user] -- [run...]'
                                "run" => run.push(args.to_string()),
                                // init is a command that will be executed WITH THIS PROGRAMS PERMISSIONS, NOT AS THE 'd26r...' USER! BE CAREFUL WITH THIS!
                                "init!" => init.push((true, vec![args.to_string()])),
                                "init_" => init.push((false, vec![args.to_string()])), // non-fatal
                                "init+" => init.last_mut().expect("used init+ before init! or init_!").1.push(args.to_string()), // adds one argument to the last-defined init program.
                                "count" => count = args.parse().expect("Syntax is count [count], where [count] is an integer!"),
                                "name" => if name.is_none() { name = Some(args.to_string()) },
                                "setname" => name = Some(args.to_string()),
                                "home" => if home_dir.is_none() { home_dir = Some(args.to_string()) },
                                "sethome" => home_dir = Some(args.to_string()),
                                _ => println!("[CONFIG] '{what}' is not a valid action!"),
                            }
                        },
                    }
                }
            },
            Err(e) => panic!("Could not read config: {e}"),
        }
    }
    
    let name = match name { Some(n) => n, None => String::new() };
    let username = format!("d26r{count}_{name}");
    let home_dir = match home_dir {
        Some(v) => v
            .replace("[d26%count]", &count.to_string())
            .replace("[d26%username]", &username)
            .replace("[d26%name]", &name),
        None => format!("/tmp/dummi26/run/{username}/home")
    };

    println!("[C] Count: {count}");
    println!("[n] Name: {name}");
    println!("Username: {username}");
    println!("Home dir: {home_dir}");
    println!();

    println!("Removing previous user:");
    Command::new("userdel").arg(username.as_str()).stdout(Stdio::inherit()).stderr(Stdio::inherit()).output().unwrap();

    mkdir("/tmp/dummi26/run").unwrap();
    if userhomedel { println!("Removing home: {:?}", rmdir(std::path::PathBuf::from(home_dir.as_str()))); }

    println!("Adding new user:");
    Command::new("useradd").args([
        "--home-dir", home_dir.as_str(),
        "--no-user-group",
        "--create-home",
        username.as_str(),
    ]).stdout(Stdio::inherit()).stderr(Stdio::inherit()).status().unwrap();
    println!("chown home dir: {:?}", Command::new("chown").args(["-R", username.as_str(), home_dir.as_str()]).status().unwrap());

    println!("Running init commands from config, if any...");
    for (fatal, cmd) in init {
        let mut cmds = cmd.into_iter();
        let cmd = cmds.next().unwrap();
        let count_s = count.to_string();
        let mut args = vec![];
        for arg in cmds {
            args.push(arg
            .replace("[d26%count]", &count_s)
            .replace("[d26%username]", &username)
            .replace("[d26%name]", &name)
            .replace("[d26%home_dir]", &home_dir)
            );
        }
        let out = Command::new(&cmd).args(args).status();
        println!("Command '{cmd}': {:?}", out);
        if fatal && ! out.unwrap().success() { panic!("Cancelling because a fatal init command was unsuccessful.") };
    }

    println!("Setup complete, running command now...\n\n");
    println!("\n\n[EXIT STATUS: {:?}]", Command::new("doas").args(["-u".to_string(), username.clone(), "--".to_string()].into_iter().chain(run.into_iter()).chain(args)).status());
    if userdel { println!("Removing user: {:?}", Command::new("userdel").arg(username.as_str()).stdout(Stdio::inherit()).stderr(Stdio::inherit()).status().unwrap()); }
    if userhomedel { println!("Removing user home: {:?}", rmdir(home_dir)); }
}
