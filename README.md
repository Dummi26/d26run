# d26run

d26run execute commands defined in `/etc/d26run/exec` as other users without your sudo password.
using unix file permissions and groups, this gives you a simple way to do permission management, where every program may have different permissions.
for example, you may create a new user for your web browser and then use `d26run` to start the browser.
if you run another program as your normal user, it will not be able to access any of your web browser's data.

## execs

files in `/etc/d26run/exec` define what commands should be executed and who is allowed to execute them.

An example of such a file:

```
# my main user
allow user mark

# another user named browser is used for web browsing
user browser
group browser

env unset XDG_RUNTIME_DIR
exec /bin/firefox
```

## setup

```sh
# compile d26run
cargo build --release
```

and then as `root`:

```sh
# copy the executable into your $PATH (doesn't have to be /bin/)
cp target/release/d26run /bin/
# set file permissions (setuid/setgid)
chown root:root /bin/d26run
chmod 775 /bin/d26run
chmod ug+s /bin/d26run

# create config directory
mkdir -p /etc/d26run/exec
```

then create at least one config, and you can start using d26run.

## execs

### with groups

The d26r-code group gives the program access to the `/code` directory.
Only some programs are allowed to see (or change!) code in my projects.

```
allow user mark

user d26r_code-main
group d26r_code-main
groups + d26r-code
groups + audio

env unset XDG_RUNTIME_DIR
exec /bin/terminal
arg -e
arg bash
arg -c
arg cd /code; tmux || bash || sh
```

### running a command in a temporary user account

`/etc/d26run/exec/temp`:

```
allow anyone

user root
group root

env unset XDG_RUNTIME_DIR
exec /bin/bash
arg /etc/d26run/scripts/temp_command.sh
args all
```

A script creates a new user account, uses `sudo` to run the command, and, once the command exits, removes the user again.
It runs `pkill` to end any background processes spawned by the temporary user.

`/etc/d26run/scripts/temp_command.sh`:

```sh
#!/bin/bash
my_id="$$"
mkdir -p /tmp/d26run-temphome
chmod 0755 /tmp/d26run-temphome
useradd --home-dir "/tmp/d26run-temphome/$my_id" --create-home --user-group --groups audio "d26r_temp_$my_id" 2>/dev/null

sudo -u "d26r_temp_$my_id" -D "/tmp/d26run-temphome/$my_id" -- "$@"

if userdel -r "d26r_temp_$my_id" 2>/dev/null; then              exit; else printf '\n.'; fi
pkill -u "d26r_temp_$my_id"
sleep 2
if userdel -r "d26r_temp_$my_id" 2>/dev/null; then printf '\n'; exit; else printf '.'; fi
sleep 2
if userdel -r "d26r_temp_$my_id" 2>/dev/null; then printf '\n'; exit; else printf '.'; fi
sleep 2
if userdel -r "d26r_temp_$my_id" 2>/dev/null; then printf '\n'; exit; else printf '.'; fi
sleep 2
if userdel -r "d26r_temp_$my_id" 2>/dev/null; then printf '\n'; exit; else printf '.'; fi
sleep 2
if userdel -r "d26r_temp_$my_id" 2>/dev/null; then printf '\n'; exit; else printf '. '; fi
pkill -u "d26r_temp_$my_id" --signal kill
sleep 1
if userdel -r "d26r_temp_$my_id" 2>/dev/null; then printf '\n'; exit; else printf '.'; fi
sleep 1
if userdel -r "d26r_temp_$my_id" 2>/dev/null; then printf '\n'; exit; else printf '.'; fi
sleep 1
if userdel -r "d26r_temp_$my_id" 2>/dev/null; then printf '\n'; exit; else printf '.'; fi
sleep 1
if userdel -r "d26r_temp_$my_id" 2>/dev/null; then printf '\n'; exit; else printf '.'; fi
sleep 1
if userdel -r "d26r_temp_$my_id" 2>/dev/null; then printf '\n'; exit; else printf '.\n'; fi
userdel -rf "d26r_temp_$my_id"
```

to use `sudo -D <dir>`, add the following to a `sudo` config file (`/etc/sudoers` or `/etc/sudoers.d/...`):
(you can remove `-D <...>` from the script if you don't want to change your sudo config)

```
Defaults:root runcwd=*
```
