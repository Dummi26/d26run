# d26run

d26run is a system that lets you run commands (programs) as other user accounts.

It consists of two executables:
d26run-server, which runs as a system service (as root),
and d26run-client, which communicates with the server to spawn certain commands.

Which commands can be spawned (and by who) must be configured in `/etc/d26run/`:

- `/etc/d26run/configs/`
  + contains utf-8 text files
  + `d26run-client run <name>` refers to the configuration file `/etc/d26run/configs/<name>`
  + defines which command should be run, which user should be used to run that command, which groups the user belongs to, ...
  + can also define prep and clean commands which will run as root before/after the main command
- `/etc/d26run/allow/`
  + contains empty files
  + to authorize a d26run-client, that client must have permission to write to the file
  + the file will be copied to `/tmp/`. If the client fails to write `auth` to the file, its request will be denied

## Quick setup

as root, run

```sh
mkdir /etc/d26run/
mkdir /etc/d26run/configs/
mkdir /etc/d26run/allow/
touch /etc/d26run/allow/anyone
chmod 666 /etc/d26run/allow/anyone
useradd d26r_firefox
echo -e "allow anyone\n\nvar %DISPLAY from-input-or DISPLAY :0\n\nenv+set DISPLAY=%DISPLAY\n\nuser d26r_firefox\ngroup d26r_firefox\ng+group audio\n\ncommand firefox" > /etc/d26run/configs/firefox
d26run-server
```

as your normal user, run

```
# allow other users to use your X display
xhost +
# ask the server to start a terminal as the new user you created earlier
d26run-client run firefox DISPLAY=$DISPLAY
```

This should cause firefox to open on your screen,
runnings as the d26r_firefox user and unable to access your normal user's home directory,
unable to use sudo, etc.

This is in an early testing phase - it's usable,
but not exactly good or high-quality.

Once everything becomes more solid and things are less likely to change,
more documentation will be added.
