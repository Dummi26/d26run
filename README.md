# d26run

Isolates commands from your user account by creating a new user for each program.

Relies on the following commands:

- doas
- chown (it's not fatal if this fails, but it's better if it works)
- useradd
- userdel

# Setup

Make sure the commands mentioned above all work (install doas if necessary, ...)
Add 'permit nopass keepenv [root]' and 'permit nopass [user] cmd d26run' to '/etc/doas.conf'. More information can be found in src/main.rs.

# Usage

## Without a config

    d26run [options..] -- [command] [args..]

Options:

- c[path to config]
- C[count]
- n[name]
- h[home dir]
- -[nouserdel, userhomedel, noconfcmd]

For example, to launch thunar in /tmp from a new user:

    doas -- d26run -- thunar /tmp

Thunar will show HOME as '/tmp/dummi26/run/d26r[PID]_/home/' and shouldn't have permissions to access /home/[user].

To use a different home directory and append fileman to the temporary username, run:

    doas -- d26run h/tmp/other_home nfileman -- thunar /tmp

## With a config

A config is a UTF-8 text file.

To use it, run:

    doas -- d26run c[path]

A config *can* specify the following things:

- run: the command that should be run as the new user (and it's arguments: the first run specifies the executable, all others are used as arguments for the program)
- init!: a command that should be run as the user executing d26run (most likely root). This can be used to, for example, add the newly created user to some groups you wish to use.
- init_: like init!, but a nonzero exit status is not fatal. Use this for things like mkdir if the directory might already exist.
- init+: Adds an argument to the most recent init[!_] command. (See examples)
- count: Specifies the count used for the username. If not specified, this is the PID.
- name: Specifies what should be appended to 'd26r[PID]_' to form the username. This just makes it easier to know what user is supposed to do what. n[name] as an argument to the command has higher priority, so it's not guaranteed that this will be used as the name.
- setname: Like name, but this can overwrite n[name].
- home: Specified the path to the new user's home directory. Can be overwritten using h[dir]
- sethome: like home, but can't be overwritten. (compare with name vs setname)

Empty lines and lines starting with # will be ignored. Use # for comments.

[d26%VAL] will be replaced with the corresponding value, if it exists:

- count: the count that was specified. (default: PID of the process)
- username: the username (equal to d26r[d26%count]_[d26%name])
- name: the name that was specified, or an empty string.
- home_dir: The home directory. This substitution doesn't work for specifying the home directory, because that would be confusing.

## With a config and with additional commands

- If a config does not specify anything for run, you can use -- [command] [args] as normal.
- If a config specifies run, the command that is executed will be [run..] [args..], for example:
  - Config specified 'run echo' and 'run test'
  - we run d26run c/tmp/cfg -- echo something
  - This will run echo with 3 arguments: 'test', 'echo', and 'something'

## Example configs

    name obs
    home /home/d26r/[d26%name]

    run obs
    run --startreplaybuffer

    init_ usermod
    init+ -G
    init+ video,audio
    init+ [d26%username]

Running 'doas -- d26run c/tmp/obs.txt' will create a new user named d26r[PID]_obs with HOME in /home/d26r/obs, add that user to the video and audio groups, and then launch OBS. Because home is not in /tmp, all OBS configurations will be persistent.

Running 'doas -- d26run c/tmp/obs.txt ntest' will use test as the name instead of obs. Because the config uses [d26%name] for home, this name change will also change the home directory.
