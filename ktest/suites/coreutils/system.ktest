# System / environment introspection: whoami, id, hostname, uname,
# pwd, env, printenv, tty, date, sleep, cal, uptime, kill.
#
# These commands inspect runtime state, so we mostly assert on shape
# (regex) rather than exact values. The reference behaviour is GNU
# coreutils as installed on the host.

# --- whoami ------------------------------------------------------------

> whoami
< root

# --- id ----------------------------------------------------------------

> id -u
< 0

> id -g
< 0

> id
~ uid=0.*gid=0

# --- groups ------------------------------------------------------------

> groups
~ root

# --- hostname ----------------------------------------------------------

> hostname
< linux-rs

# --- uname -------------------------------------------------------------

> uname
~ \S

> uname -s
~ \S

# --- pwd ---------------------------------------------------------------

> cd /
> pwd
< /

> cd /root
> pwd
< /root

# --- env / printenv ---------------------------------------------------

> echo $HOME
< /root

> printenv HOME
< /root

> env | grep -c HOME
~ ^\s*\d+\s*$

# --- date --------------------------------------------------------------
# Just sanity-check shape: date should produce a non-empty line.

> date
~ \d{4}

# --- cal ---------------------------------------------------------------

> cal 1 2024
~ January 2024
~ Su\s+Mo\s+Tu\s+We\s+Th\s+Fr\s+Sa

# --- sleep -------------------------------------------------------------

> sleep 0
> echo sleep0_rc=$?
< sleep0_rc=0

# --- time --------------------------------------------------------------

> time true
~ (?i)real|user|sys|s$

# --- tty ---------------------------------------------------------------

> tty
~ /dev/tty

# --- uptime ------------------------------------------------------------

> uptime
~ up|load|\d
