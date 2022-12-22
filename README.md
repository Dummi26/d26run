# d26run
creates a new user and runs a command as that user. Provides isolation from your main user account. NOTE: This is only as secure as the default permissions used by useradd. It also relies on the commands doas, chown, useradd and userdel. Tested on and built for archlinux, but might work on other distros as well, who knows.
