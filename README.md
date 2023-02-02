# What

d26run creates a new temporary user, configures it, and then runs a command as that user.
Basically:

    useradd $username
    # some setup
    doas -u $username -- $command
    userdel $username

# Why

## Temporary $HOME

By default, d26run puts the temporary users home directory in `/tmp/dummi26_run/[username]/home/` and removes this entire folder after the command finishes. This gives an "always fresh" effect, as the programs configuration, cache, etc. will be effectively wiped after the program exits (assuming `-userhomedel`).

## Separation

Because of how users work, each user can have a separate account and config for any installed program. Using d26run, there can be multiple users using the same program on different accounts, simultaneously.

Just like with regular users, these temporary users aren't allowed to access any other users files. This means that running malicious programs through d26run can prevent them from accessing or even destroying most of your data, which is owned by some other user.

NOTE: I'm no expert with security or how linux permissions work exactly. This will probably help keep your system and data secure, but it's not guaranteed to do so and might even open up some security loopholes that I wasn't aware of.
If your `useradd` grants more permissions than it did during my testing, you might need to reconfigure things for this to be as secure as you would like.
If privilege escalation is somehow possible (through some kernel exploit or by exploiting doas, ...), this might prevent against basic data-scanners, but not against all malware.

## Permissions (per-application like on android/ios; at least almost)

By giving different users different permissions, you can manage which files this user can or can't access. For example, only users in the `video` group may access camera devices. This means that running a program with d26run will prevent it from accessing these video devices unless you specifically add the new user to the video group. This can be expanded by adding custom groups and setting file permissions in a sensible way.

# Dependencies and Setup

d26run relies on the following commands:

- doas
- chown (it's not fatal if this fails, but it's better if it works)
- cp (for immutable home)
- useradd
- userdel

Setup:

- Make sure the commands mentioned above all work (install doas if necessary, ...)

- Add 'permit nopass keepenv \[root user]' and 'permit nopass \[user] cmd d26run' to '/etc/doas.conf'. More information can be found in src/main.rs.

# Usage

## Basic examples

    doas -- d26run

This command needs root/su permissions to work properly.

The `--` is used in case any arguments provided to d26run contain `--`, which might mess with the doas arg parser.

	doas -- d26run -- $SHELL

This is the most basic command. It will launch a shell as a new temporary user. Some noteworthy things are:

The shell configuration is gone.
`echo $HOME` shows a location in /tmp/
`ls` can't read your normal users home directory

	mkdir /tmp/test
	doas -- d26run h/tmp/test -- $SHELL
	# doas -- d26run -h=/tmp/test -- $SHELL
	# doas -- d26run -h /tmp/test -- $SHELL
	# doas -- d26run -home /tmp/test -- $SHELL

Now, `echo $HOME` will show /tmp/test/ instead. (any of the four d26run commands do the same thing)

	doas -- d26run Gvideo,audio -- $SHELL

This will put you in a shell again. Typing `groups` will show that the temporary user has been added to the video and audio groups.

## Creating a config file

Configs are UTF-8 text files. They are parsed line by line.

Lines starting with `#` are comments and will be ignored. Empty lines will also be ignored.

Lines starting with ` ` (space) are script lines. This will be explained later.

I recommend taking a look at my config examples and only referring to the following section for details, as it is quite a lot of mostly unnecessary information that is hard to remember.

To configure d26run:

	config [path to config]

load another config before this one

	run [command/arg]

adds the following text as a command or argument to the command: The first thing added using run is the command, anything after that is an argument.

	init! [command]
a command that will be executed after the temporary user was created, but before it is actually used for
	 anything. If this command fails, d26run will panic.

	init_ [command]

like init!, but the commands failure will be ignored and d26run will continue running.

	init+ [arg]

adds one argument to the last command specified by init! or init_.

	count [num]

sets the count (used when generating the username) to a certain number.

	name [name]
	setname [name]

sets the temporary users name. The username will be d26r[count]_[name].

	passwd [encrypted password]
	setpasswd [encrypted password]

sets the temporary users password. This is provided to useradd using -p. It can be generated from plaintext using openssl: openssl passwd -1 [plaintext password]

	group [group]
	setgroup [group]

sets the temporary users main group (by default, a group named just like the user)

	groups [group(s)]
	setgroups [group(s)]

a comma-separated list of groups the user should be in
	
	addgroups [group(s)]

like groups and setgroups, but appends to the groups the user is already in instead of overwriting them. If groups isn't set yet, this will be the same as groups or setgroups.

	home [dir]
	sethome [dir]

sets the temporary users home directory. If this is not deleted, it will be reused, making the user permanent (even though the user is still deleted, the next user will chown the home directory and take over).

	immuthome [dir]
	setimmuthome [dir]

copies all files from the immutable directory to the temporary users home. Useful to provide some configuration for the programs you want to use while still preventing the programs from saving anything.

Many actions like `name` have a `setname` counterpart. The non-set version will set the value only if no other value was specified previously:

Assuming only `name` is used, the actual value will be that specified first (top of the config)

Assuming only `setname` is used, the actual value will be that specified last (bottom of the config)

Assuming `setname` is used at all, the value will be that specified by `setname`.

Assuming a config specified a `name`, adding `n[name]` to the arguments will overwrite it.

If `[d26var:varname]` is used in any value and a variable named varname exists, it will be replaced with the content of the variable (or its string representation). See the section about scripting for info on variables. This replace action takes place when the line is parsed, meaning the variable needs to be set in a line above where it is used.

If `[d26%{count, username, group, groups, home_dir, immutable_home_at}]` is used, it will be replaced with the corresponding value. This means that a config specifying `name test` and `home /tmp/[d26%name]` will have its home directory set to /tmp/nope instead of /tmp/home if you run `d26run -name nope c/tmp/config.txt -test` because the `[d26%name]` wasn't replaced when the config was parsed but only at the very end, after the config's name (test) was overwritten to be "nope" instead.

If a config specified something via `run` and the arguments specify more stuff by providing arguments after the `--`, the arguments specified after `--` will be added to the `run` commands arguments, and will not be treated as a new command. To ignore the `run` lines while reading configs, add `-noconfcmd` to d26runs arguments:

		# conf.txt
		run echo
		run some_arg

		doas -- d26run -c conf.txt -- echo something_else
		# outputs 'some_arg echo something_else'
		
		doas -- d26run -c conf.txt -noconfcmd -- echo something_else
		# outputs 'something_else'

## Scripting in configs

( TODO! )

