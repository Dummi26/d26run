# d26run

Isolates commands from your user account by creating a new user for each program.

Relies on the following commands:

- doas
- chown (it's not fatal if this fails, but it's better if it works)
- cp (for immutable home)
- useradd
- userdel

# Setup

- Make sure the commands mentioned above all work (install doas if necessary, ...)

- Add 'permit nopass keepenv \[root user]' and 'permit nopass \[user] cmd d26run' to '/etc/doas.conf'. More information can be found in src/main.rs.

# Usage

## Without a config

    d26run [options..] -- [command] [args..]

Options:

- c\[path to config]
- C\[count]
- n\[name]
- p\[passwd (encrypted - see useradd -p or --password)]
- h\[home dir]
- H\[immutable home dir]
- -\[nouserdel, userhomedel, noconfcmd]

For example, to launch thunar in /tmp from a new user:

    doas -- d26run -- thunar /tmp

Thunar will show HOME as '/tmp/dummi26/run/d26r\[PID]\_/home/' and shouldn't have permissions to access /home/\[user].

To use a different home directory and append fileman to the temporary username, run:

    doas -- d26run h/tmp/other_home nfileman -- thunar /tmp

If you want an 'always fresh' experience without putting HOME in /tmp, you can use the immutable home dir as the preset.
If this is specified, then everything in the immutable home dir will be copied to the actual home dir (in /tmp if nothing else is specified), so you can add configs to your programs without letting them save anything. (They can save things, but all changes will be lost)

    doas -- d26run h/tmp/my_temp_home H/home/presets/firefox -- firefox

or just

    doas -- d26run H/home/presets/firefox -- firefox

## With a config

A config is a UTF-8 text file.

To use it, run:

    doas -- d26run c[path]

Multiple configs can be specified by using 'c\[config1] c\[config2] ...'

A config *can* specify the following things:

- run: the command that should be run as the new user (and it's arguments: the first run specifies the executable, all others are used as arguments for the program)
- config: loads another config. (behavior might depend on the position of this line in the config file: the inner config can overwrite any values specified **before** the line in the outer config) (can be used multiple times without issue, but can loop forever due to recursion!)
- init!: a command that should be run as the user executing d26run (most likely root). This can be used to, for example, add the newly created user to some groups you wish to use.
- init\_: like init!, but a nonzero exit status is not fatal. Use this for things like mkdir if the directory might already exist.
- init+: Adds an argument to the most recent init\[!\_] command. (See examples)
- count: Specifies the count used for the username. If not specified, this is the PID.
- name: Specifies what should be appended to 'd26r\[PID]\_' to form the username. This just makes it easier to know what user is supposed to do what. n\[name] as an argument to the command has higher priority, so it's not guaranteed that this will be used as the name.
- setname: Like name, but this can overwrite n\[name].
- passwd: Encrypted password. If this is present, -p [passwd] will be passed to useradd.
- setpasswd: Like passwd, but can overwrite p\[passwd].
- home: Specified the path to the new user's home directory. Can be overwritten using h\[dir]
- sethome: like home, but can't be overwritten. (compare with name vs setname)
- immuthome: before the new user is even created, the home directory will be created as an empty folder and then the path specified in immuthome will be copied to it. useful to provide some ~/.config/\* files for programs while keeping the user (effectively) readonly.
- setimmuthome: like immuthome, but can't be overwritten. (compare with name vs setname)

Lines starting with # will be ignored. Use # for comments.

\[d26%VAL] will be replaced with the corresponding value, if it exists:

- count: the count that was specified. (default: PID of the process)
- username: the username (equal to d26r\[d26%count]\_\[d26%name])
- name: the name that was specified, or an empty string.
- home\_dir: The home directory. This substitution doesn't work for specifying the home directory, because that would be confusing.

### Scripting

There are some basic scripting abilities. Any line starting with a ' ' space is considered part of a script.

- To define a string, use 'varname=:my string' (there are no string literals yet, which makes scripting very annoying, as this string syntax is the only workaround right now.)
- To set a variable to a value, use 'varname=expression'.
- Functions: To call a function, use 'func\_name : arg1 : arg2 : ...' with any number of arguments. Multiple function calls must occur on separate lines because brackets are not supported (yet?).
- Expressions:
  + ! is used to invert bools: '!true == false', '!false == true'
  + Operators: 'a [&& || == + - * /] b', i.e. '3 + 4 == 7'
  + Literals: If nothing else applies, int, float and bool literals will be parsed.
  + Variables: If this also fails, the expression is assumed to be a variable.

For examples of this, see /examples/.

Variables from your script(s) are accessible outside of scripts via [d26var:varname].

## With a config and with additional commands

- If a config does not specify anything for run, you can use -- \[command] \[args] as normal.
- If a config specifies run, the command that is executed will be \[run..] \[args..], for example:
  - Config specified 'run echo' and 'run test'
  - we run d26run c/tmp/cfg -- echo something
  - This will run echo with 3 arguments: 'test', 'echo', and 'something'
- To avoid this, provide -noconfcmd, for example to open a terminal as the user that is normally intended for web browsing:
  - 'd26run c/tmp/firefox -noconfcmd -- alacritty'
- Configs can specify name or setname, home or sethome, etc. In general, name has the lowest priority, followed by the args (n\[...]), and setname has the highest priority. If a value is provided, but it is an empty string, it will be treated as if no value was provided. Because of this, if a config specifies 'name weird\_name', this can be reset by simply providing 'n' as an argument.

## Example configs (see /examples/ for more advanced stuff)

    name obs
    home /home/d26r/[d26%name]

    run obs
    run --startreplaybuffer

    init_ usermod
    init+ -G
    init+ video,audio
    init+ [d26%username]

Running 'doas -- d26run c/tmp/obs.txt' will create a new user named d26r[PID]\_obs with HOME in /home/d26r/obs, add that user to the video and audio groups, and then launch OBS. Because home is not in /tmp, all OBS configurations will be persistent.

Running 'doas -- d26run c/tmp/obs.txt ntest' will use test as the name instead of obs. Because the config uses [d26%name] for home, this name change will also change the home directory.
